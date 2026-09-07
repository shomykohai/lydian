use axum::Extension;
use axum::extract::{Request, State};
use axum::http::HeaderValue;
use axum::response::Response;
use log::debug;
use reqwest::header::CACHE_CONTROL;
use tower::Service;

use crate::models::{SubsonicErrorCode, SubsonicRespFormat};
use crate::routes::{LydianState, SubsonicExt};
use crate::services::{GetCoverArt, RemoteLookupService};
use crate::storage::keys::StorageKey;
use crate::workers::download::DownloadSource;

const IMMUTABLE: HeaderValue = HeaderValue::from_static("public, max-age=604800, immutable");

fn cacheable(mut response: Response) -> Response {
    response.headers_mut().insert(CACHE_CONTROL, IMMUTABLE);
    response
}

pub async fn get_cover_art(
    State(state): LydianState,
    Extension(subsonic): SubsonicExt<'_>,
    format: SubsonicRespFormat,
    req: Request,
) -> Response {
    debug!("Cover art request: {}", req.uri().path());
    let (parts, body) = req.into_parts();

    let Ok(op) = subsonic.parse_query::<GetCoverArt>(&parts) else {
        return subsonic.build_error(format, SubsonicErrorCode::RequiredParameterMissing);
    };

    let file_key =
        StorageKey::CoverArt { id: op.id.raw().to_string(), size: op.size }.to_file_name();

    if let Some(target_path) = state.storage.disk.resolve_path_prefix(&file_key).await {
        return cacheable(subsonic.stream_local_media(format, target_path, parts, None).await);
    }

    if !op.id.is_external()
        && let Some(proxy_response) = subsonic.try_media(format, &parts, body).await
    {
        return cacheable(proxy_response);
    }

    let mut service = RemoteLookupService { remotes: &state.remotes, cache: state.cache.clone() };

    service.call(op).await.map_or_else(
        |_| subsonic.build_error(format, SubsonicErrorCode::NotFound),
        |remote_stream| {
            if let Some(source) = DownloadSource::from_response(&remote_stream) {
                state.queue_download(file_key, source);
            }

            cacheable(subsonic.stream_media(format, remote_stream))
        },
    )
}
