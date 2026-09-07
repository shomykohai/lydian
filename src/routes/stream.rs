use axum::Extension;
use axum::extract::{Request, State};
use axum::http::Method;
use axum::response::Response;
use log::debug;
use reqwest::header::{CONTENT_TYPE, RANGE};
use tower::Service;

use crate::models::{SubsonicErrorCode, SubsonicRespFormat, SubsonicStreamFormat};
use crate::providers::StreamRequest;
use crate::routes::{LydianState, SubsonicExt};
use crate::services::{RemoteLookupService, StreamMedia};
use crate::storage::keys::StorageKey;
use crate::workers::download::DownloadSource;

pub async fn stream_media(
    State(state): LydianState,
    Extension(subsonic): SubsonicExt<'_>,
    format: SubsonicRespFormat,
    req: Request,
) -> Response {
    debug!("Media stream request: {}", req.uri().path());

    let (parts, body) = req.into_parts();

    let range_header = parts.headers.get(RANGE).and_then(|h| h.to_str().ok()).map(|s| s.to_owned());
    let is_head_request = parts.method == Method::HEAD;
    let is_full_request = range_header.is_none();

    let Ok(stream_request) = subsonic.parse_query::<StreamRequest>(&parts) else {
        return subsonic.build_error(format, SubsonicErrorCode::RequiredParameterMissing);
    };

    let wanted_format = stream_request.format.unwrap_or_default();
    let is_transcoding = wanted_format.is_transcode();
    // Only present if the `transcodeOffset` extension is available.
    let offset = if is_transcoding { stream_request.time_offset.unwrap_or_default() } else { 0 };

    let file_key =
        StorageKey::Media { id: stream_request.id.to_string(), format: SubsonicStreamFormat::Raw }
            .to_file_name();

    if let Some(target_path) = state.storage.disk.resolve_path_prefix(&file_key).await {
        return subsonic
            .stream_local_media(format, target_path, parts, Some(&stream_request))
            .await;
    }

    if !stream_request.id.is_external()
        && let Some(proxy_response) = subsonic.try_media(format, &parts, body).await
    {
        return proxy_response;
    }

    let bitrate = stream_request.max_bit_rate.unwrap_or_default();

    let lookup_op = StreamMedia {
        request: stream_request,
        range_header: if is_transcoding { None } else { range_header },
    };

    let mut service = RemoteLookupService { remotes: &state.remotes, cache: state.cache.clone() };

    let Ok(resp) = service.call(lookup_op).await else {
        return subsonic.build_error(format, SubsonicErrorCode::NotFound);
    };

    let source = DownloadSource::from_response(&resp);
    let content_type_header =
        resp.headers().get(CONTENT_TYPE).and_then(|v| v.to_str().ok()).map(str::to_owned);

    let can_tee_cache = !is_head_request
        && is_full_request
        && !is_transcoding
        && source.as_ref().is_some_and(|source| {
            source
                .content_type()
                .is_none_or(|declared| content_type_header.as_deref() == Some(declared))
        });

    if can_tee_cache && let Some(content_type) = content_type_header {
        let cache_name =
            format!("{file_key}.{}", StorageKey::extension_for(&file_key, &content_type));

        return subsonic.stream_media_cached(format, resp, cache_name).await;
    }

    if !is_head_request && let Some(source) = source {
        state.queue_download(file_key, source);
    }

    if is_transcoding {
        return subsonic.transcode_media(format, resp, wanted_format, bitrate, offset).await;
    }

    subsonic.stream_media(format, resp)
}
