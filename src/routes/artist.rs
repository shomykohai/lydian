use axum::Extension;
use axum::extract::{Request, State};
use axum::response::Response;
use log::debug;
use reqwest::StatusCode;
use tower::Service;

use crate::models::{ArtistInfo, ArtistWithAlbumsID3, SubsonicErrorCode, SubsonicRespFormat};
use crate::routes::{LydianState, SubsonicExt};
use crate::services::{GetArtist, GetArtistInfo, RemoteLookupService};

pub async fn get_artist_info(
    State(state): LydianState,
    Extension(subsonic): SubsonicExt<'_>,
    format: SubsonicRespFormat,
    req: Request,
) -> Response {
    debug!("Artist info request: {}", req.uri().path());

    let (parts, body) = req.into_parts();

    if let Some(final_response) = subsonic.try_req::<ArtistInfo<'_>>(format, &parts, body).await {
        return final_response;
    }

    let Ok(op) = subsonic.parse_query::<GetArtistInfo<'_>>(&parts) else {
        return subsonic.build_error(format, SubsonicErrorCode::RequiredParameterMissing);
    };

    let mut service = RemoteLookupService { remotes: &state.remotes, cache: state.cache.clone() };

    service.call(op).await.map_or_else(
        |_| subsonic.build_error(format, SubsonicErrorCode::NotFound),
        |artist| subsonic.build_response(format, StatusCode::OK, Some(artist)),
    )
}

pub async fn get_artist(
    State(state): LydianState,
    Extension(subsonic): SubsonicExt<'_>,
    format: SubsonicRespFormat,
    req: Request,
) -> Response {
    debug!("Artist request: {}", req.uri().path());

    let (parts, body) = req.into_parts();

    if let Some(final_response) =
        subsonic.try_req::<ArtistWithAlbumsID3>(format, &parts, body).await
    {
        return final_response;
    }

    let Ok(op) = subsonic.parse_query::<GetArtist<'_>>(&parts) else {
        return subsonic.build_error(format, SubsonicErrorCode::RequiredParameterMissing);
    };

    let mut service = RemoteLookupService { remotes: &state.remotes, cache: state.cache.clone() };

    service.call(op).await.map_or_else(
        |_| subsonic.build_error(format, SubsonicErrorCode::NotFound),
        |artist| subsonic.build_response(format, StatusCode::OK, Some(artist)),
    )
}
