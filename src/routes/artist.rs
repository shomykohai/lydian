use axum::Extension;
use axum::extract::{Request, State};
use axum::response::Response;
use is_empty::IsEmpty;
use log::debug;
use reqwest::StatusCode;
use tower::Service;

use crate::models::{
    ArtistInfo,
    ArtistWithAlbumsID3,
    FuzzyMatch,
    Mergeable,
    SubsonicErrorCode,
    SubsonicRespFormat,
};
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

    let Ok(op) = subsonic.parse_query::<GetArtistInfo<'_>>(&parts) else {
        return subsonic.build_error(format, SubsonicErrorCode::RequiredParameterMissing);
    };

    let mut subsonic_resp = subsonic
        .request::<ArtistInfo<'_>>(
            parts.method.clone(),
            parts.uri.path(),
            parts.uri.query(),
            parts.headers.clone(),
            Some(body),
        )
        .await;

    let mut service = RemoteLookupService { remotes: &state.remotes, cache: state.cache.clone() };

    let mut unified_result = if let Ok(ref mut resp) = subsonic_resp
        && !resp.is_err()
        && let Some(answer) = resp.answer.take()
    {
        answer
    } else {
        ArtistInfo::default()
    };

    if let Ok(remote_artist_info) = service.call(op).await {
        unified_result.merge(remote_artist_info);
    }

    if unified_result.is_empty() {
        subsonic.build_error(format, SubsonicErrorCode::NotFound)
    } else {
        subsonic.build_response(format, StatusCode::OK, Some(unified_result))
    }
}

pub async fn get_artist(
    State(state): LydianState,
    Extension(subsonic): SubsonicExt<'_>,
    format: SubsonicRespFormat,
    req: Request,
) -> Response {
    debug!("Artist request: {}", req.uri().path());

    let (parts, body) = req.into_parts();

    let Ok(op) = subsonic.parse_query::<GetArtist<'_>>(&parts) else {
        return subsonic.build_error(format, SubsonicErrorCode::RequiredParameterMissing);
    };

    let mut service = RemoteLookupService { remotes: &state.remotes, cache: state.cache.clone() };

    let mut subsonic_resp = if !op.id.is_external() {
        Some(
            subsonic
                .request::<ArtistWithAlbumsID3<'_>>(
                    parts.method.clone(),
                    parts.uri.path(),
                    parts.uri.query(),
                    parts.headers.clone(),
                    Some(body),
                )
                .await,
        )
    } else {
        None
    };

    let mut unified_result = if let Some(Ok(ref mut resp)) = subsonic_resp
        && !resp.is_err()
        && let Some(ref mut answer) = resp.answer
    {
        std::mem::take(answer)
    } else {
        ArtistWithAlbumsID3::default()
    };

    if let Ok(mut remote_artist) = service.call(op).await {
        if !unified_result.base.id.is_empty()
            && let Some(ref mut remote_albums) = remote_artist.album
        {
            for album in remote_albums {
                album.artist_id = Some(unified_result.base.id.clone());
                if let Some(ref mut artists) = album.artists {
                    // TODO: Link other local artists too if they exist
                    for a in artists {
                        if a.is_similar_to(&unified_result.base) {
                            a.id = unified_result.base.id.clone();
                        }
                    }
                }
            }
        }

        if !unified_result.base.id.is_empty() {
            unified_result.merge(remote_artist);
        } else {
            unified_result = remote_artist;
        }
    }

    if let Some(ref albums) = unified_result.album {
        unified_result.base.album_count = Some(albums.len() as u32);
    }

    if unified_result.base.id.is_empty() {
        subsonic.build_error(format, SubsonicErrorCode::NotFound)
    } else {
        subsonic.build_response(format, StatusCode::OK, Some(unified_result))
    }
}
