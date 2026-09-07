use axum::Extension;
use axum::extract::{Request, State};
use axum::response::Response;
use is_empty::IsEmpty;
use log::debug;
use reqwest::{Method, StatusCode};
use tower::Service;

use crate::models::{
    ArtistInfo,
    ArtistWithAlbumsID3,
    Limit,
    Mergeable,
    SIMILAR_ARTIST_LIMIT,
    SubsonicEntity,
    SubsonicErrorCode,
    SubsonicRespFormat,
};
use crate::providers::ArtistRef;
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

    let mut service = RemoteLookupService { remotes: &state.remotes, cache: state.cache.clone() };
    let mut op = op;
    let count = op.count;

    let describe = async {
        if op.id.is_external() {
            return None;
        }

        subsonic
            .request::<ArtistWithAlbumsID3<'_>>(
                Method::GET,
                "/rest/getArtist",
                Some(&format!("id={}", op.id.raw())),
                parts.headers.clone(),
                None,
            )
            .await
            .ok()
            .and_then(|resp| resp.answer)
    };

    let (mut subsonic_resp, described) = futures::join!(
        subsonic.request::<ArtistInfo<'_>>(
            parts.method.clone(),
            parts.uri.path(),
            parts.uri.query(),
            parts.headers.clone(),
            Some(body),
        ),
        describe,
    );

    if let Some(artist) = described.filter(|artist| !artist.base.name.is_empty()) {
        op.reference = Some(ArtistRef::of(&artist));
    }

    let remote_artist_info = service.call(op).await;

    let mut unified_result = if let Ok(ref mut resp) = subsonic_resp
        && !resp.is_err()
        && let Some(answer) = resp.answer.take()
    {
        answer
    } else {
        ArtistInfo::default()
    };

    if let Ok(remote_artist_info) = remote_artist_info {
        unified_result.merge(remote_artist_info);
    }

    unified_result.similar_artist.limit(count.unwrap_or(SIMILAR_ARTIST_LIMIT));

    if unified_result.is_empty() {
        subsonic.build_error(format, SubsonicErrorCode::NotFound)
    } else {
        let field = <ArtistInfo<'_> as SubsonicEntity>::field_for(parts.uri.path());

        subsonic.build_response_for(format, StatusCode::OK, Some(unified_result), field)
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

    let mut op = op;
    let local = !op.id.is_external();

    let mut subsonic_resp = if local {
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

    if local && !unified_result.base.name.is_empty() {
        op.reference = Some(ArtistRef::of(&unified_result));
    }

    let remote_artist = service.call(op).await;

    if let Ok(mut remote_artist) = remote_artist {
        if !unified_result.base.id.is_empty()
            && let Some(ref mut remote_albums) = remote_artist.album
        {
            for album in remote_albums {
                album.artist_id = Some(unified_result.base.id.clone());
                if let Some(ref mut artists) = album.artists {
                    // TODO: Link other local artists too if they exist
                    for a in artists {
                        if a.matches(&unified_result.base) {
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
