use axum::Extension;
use axum::extract::{Request, State};
use axum::response::Response;
use log::debug;
use reqwest::header::RANGE;
use tower::Service;

use crate::models::{SubsonicErrorCode, SubsonicRespFormat};
use crate::providers::StreamRequest;
use crate::routes::{LydianState, SubsonicExt};
use crate::services::{RemoteLookupService, StreamMedia};
use crate::storage::keys::StorageKey;

pub async fn stream_media(
    State(state): LydianState,
    Extension(subsonic): SubsonicExt<'_>,
    format: SubsonicRespFormat,
    req: Request,
) -> Response {
    debug!("Media stream request: {}", req.uri().path());

    let (parts, body) = req.into_parts();

    let range_header = parts.headers.get(RANGE).and_then(|h| h.to_str().ok()).map(|s| s.to_owned());

    let Ok(stream_request) = subsonic.parse_query::<StreamRequest>(&parts) else {
        return subsonic.build_error(format, SubsonicErrorCode::RequiredParameterMissing);
    };

    let file_key = StorageKey::Media {
        id: stream_request.id.to_string(),
        format: stream_request.format.unwrap_or_default(),
    }
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

    let op = StreamMedia { request: stream_request, range_header };

    let mut service = RemoteLookupService { remotes: &state.remotes, cache: state.cache.clone() };

    service.call(op).await.map_or_else(
        |_| subsonic.build_error(format, SubsonicErrorCode::NotFound),
        |resp| {
            state.queue_download(file_key, resp.url());
            subsonic.stream_media(format, resp)
        },
    )
}
