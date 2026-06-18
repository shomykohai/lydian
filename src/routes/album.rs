use axum::Extension;
use axum::extract::{Request, State};
use axum::response::Response;
use log::debug;
use reqwest::StatusCode;
use tower::Service;

use crate::models::{AlbumInfo, AlbumWithSongsID3, SubsonicErrorCode, SubsonicRespFormat};
use crate::routes::{LydianState, SubsonicExt};
use crate::services::{GetAlbum, GetAlbumInfo, RemoteLookupService};

pub async fn get_album_info<'a>(
    State(state): LydianState,
    Extension(subsonic): SubsonicExt<'a>,
    format: SubsonicRespFormat,
    req: Request,
) -> Response {
    debug!("Album info request: {}", req.uri().path());

    let (parts, body) = req.into_parts();

    if let Some(subsonic_resp) = subsonic.try_req::<AlbumInfo<'a>>(format, &parts, body).await {
        return subsonic_resp;
    }

    let Ok(op) = subsonic.parse_query::<GetAlbumInfo<'_>>(&parts) else {
        return subsonic.build_error(format, SubsonicErrorCode::RequiredParameterMissing);
    };

    let mut service = RemoteLookupService { remotes: &state.remotes, cache: state.cache.clone() };

    service.call(op).await.map_or_else(
        |_| subsonic.build_error(format, SubsonicErrorCode::NotFound),
        |album| subsonic.build_response(format, StatusCode::OK, Some(album)),
    )
}

pub async fn get_album<'a>(
    State(state): LydianState,
    Extension(subsonic): SubsonicExt<'a>,
    format: SubsonicRespFormat,
    req: Request,
) -> Response {
    debug!("Album request: {}", req.uri().path());

    let (parts, body) = req.into_parts();

    if let Some(subsonic_resp) =
        subsonic.try_req::<AlbumWithSongsID3<'a>>(format, &parts, body).await
    {
        return subsonic_resp;
    }

    let Ok(op) = subsonic.parse_query::<GetAlbum<'_>>(&parts) else {
        return subsonic.build_error(format, SubsonicErrorCode::RequiredParameterMissing);
    };

    let mut service = RemoteLookupService { remotes: &state.remotes, cache: state.cache.clone() };

    service.call(op).await.map_or_else(
        |_| subsonic.build_error(format, SubsonicErrorCode::NotFound),
        |album| subsonic.build_response(format, StatusCode::OK, Some(album)),
    )
}
