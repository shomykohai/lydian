use axum::Extension;
use axum::extract::{Request, State};
use axum::response::Response;
use log::debug;
use tower::Service;

use crate::models::{SubsonicErrorCode, SubsonicRespFormat};
use crate::routes::{LydianState, SubsonicExt};
use crate::services::{GetCoverArt, RemoteLookupService};
use crate::storage::keys::StorageKey;

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

    let file_key = StorageKey::CoverArt { id: op.id.clone(), size: op.size.unwrap_or(u32::MAX) }
        .to_file_name();

    if let Some(target_path) = state.storage.disk.resolve_path_prefix(&file_key).await {
        return subsonic.stream_local_media(format, target_path, parts).await;
    }

    if let Some(proxy_response) = subsonic.try_media(format, &parts, body).await {
        return proxy_response;
    }

    let mut service = RemoteLookupService { remotes: &state.remotes, cache: state.cache.clone() };

    service.call(op).await.map_or_else(
        |_| subsonic.build_error(format, SubsonicErrorCode::NotFound),
        |remote_stream| {
            state.queue_download(file_key, remote_stream.url());
            subsonic.stream_media(format, remote_stream)
        },
    )
}
