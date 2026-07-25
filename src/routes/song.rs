use axum::Extension;
use axum::extract::{Request, State};
use axum::response::Response;
use log::debug;
use reqwest::StatusCode;
use tower::Service;

use crate::models::{Mergeable, Song, SubsonicErrorCode, SubsonicRespFormat, TopSongs};
use crate::routes::{LydianState, SubsonicExt};
use crate::services::{GetSong, GetTopSongs, RemoteLookupService};

pub async fn get_top_songs(
    State(state): LydianState,
    Extension(subsonic): SubsonicExt<'_>,
    format: SubsonicRespFormat,
    req: Request,
) -> Response {
    debug!("Top songs request: {}", req.uri().path());

    let (parts, body) = req.into_parts();

    let mut subsonic_resp = subsonic
        .request::<TopSongs<'_>>(
            parts.method.clone(),
            parts.uri.path(),
            parts.uri.query(),
            parts.headers.clone(),
            Some(body),
        )
        .await;

    let Ok(op) = subsonic.parse_query::<GetTopSongs<'_>>(&parts) else {
        return subsonic.build_error(format, SubsonicErrorCode::RequiredParameterMissing);
    };

    let mut service = RemoteLookupService { remotes: &state.remotes, cache: state.cache.clone() };

    let mut unified_result = TopSongs { song: None };

    if let Ok(ref mut resp) = subsonic_resp
        && !resp.is_err()
        && let Some(ref mut answer) = resp.answer
        && let Some(local_songs) = answer.song.take()
    {
        unified_result.song = Some(local_songs);
    }

    if let Ok(remote_songs) = service.call(op).await {
        unified_result.merge(remote_songs);
    }

    subsonic.build_response(format, StatusCode::OK, Some(unified_result))
}

pub async fn get_song(
    State(state): LydianState,
    Extension(subsonic): SubsonicExt<'_>,
    format: SubsonicRespFormat,
    req: Request,
) -> Response {
    debug!("Song request: {}", req.uri().path());

    let (parts, body) = req.into_parts();

    let Ok(op) = subsonic.parse_query::<GetSong<'_>>(&parts) else {
        return subsonic.build_error(format, SubsonicErrorCode::RequiredParameterMissing);
    };

    if !op.id.is_external()
        && let Some(final_response) = subsonic.try_req::<Song<'_>>(format, &parts, body).await
    {
        return final_response;
    }

    let mut service = RemoteLookupService { remotes: &state.remotes, cache: state.cache.clone() };

    service.call(op).await.map_or_else(
        |_| subsonic.build_error(format, SubsonicErrorCode::NotFound),
        |song| subsonic.build_response(format, StatusCode::OK, Some(song)),
    )
}
