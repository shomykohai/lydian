use std::borrow::Cow;

use axum::Extension;
use axum::extract::{Request, State};
use axum::response::Response;
use log::debug;
use reqwest::StatusCode;
use tower::Service;

use crate::models::{Mergeable, SearchResult, SubsonicErrorCode, SubsonicRespFormat};
use crate::providers::SearchRequest;
use crate::routes::{LydianState, SubsonicExt};
use crate::services::{RemoteLookupService, Search3};

pub async fn search3(
    State(state): LydianState,
    Extension(subsonic): SubsonicExt<'_>,
    format: SubsonicRespFormat,
    req: Request,
) -> Response {
    debug!("Search request: {}", req.uri().path());

    let (parts, body) = req.into_parts();

    let Ok(search_req) = subsonic.parse_query::<Search3<'_>>(&parts) else {
        return subsonic.build_error(format, SubsonicErrorCode::RequiredParameterMissing);
    };

    let origin = subsonic.request::<SearchResult<'_>>(
        parts.method.clone(),
        parts.uri.path(),
        parts.uri.query(),
        parts.headers.clone(),
        Some(body),
    );

    /* If client sent an empty query, we might be doing a global search (get all artists, albums,
     * songs...) In this case, returning remotes would be overkill and not feasible. Better
     * for us to return the origin server results only.
     * TODO: Consider if we want to support adding cached remote results.
     */
    if search_req.request.query.trim().is_empty() {
        if let Ok(resp) = origin.await {
            return subsonic.build_response(format, StatusCode::OK, resp.answer);
        }
        return subsonic.build_error(format, SubsonicErrorCode::Generic);
    }

    let mut service = RemoteLookupService { remotes: &state.remotes, cache: state.cache.clone() };

    let query = search_req.request.query.to_string();
    let limits = SearchRequest {
        query: Cow::Borrowed(""),
        artist_count: search_req.request.artist_count,
        artist_offset: search_req.request.artist_offset,
        album_count: search_req.request.album_count,
        album_offset: search_req.request.album_offset,
        song_count: search_req.request.song_count,
        song_offset: search_req.request.song_offset,
    };

    let (mut subsonic_resp, remote_results) = futures::join!(origin, service.call(search_req));

    let mut unified_results = SearchResult { artist: vec![], album: vec![], song: vec![] };

    if let Ok(ref mut resp) = subsonic_resp
        && !resp.is_err()
        && let Some(ref mut answer) = resp.answer
    {
        unified_results.artist.append(&mut answer.artist);
        unified_results.album.append(&mut answer.album);
        unified_results.song.append(&mut answer.song);
    }

    if let Ok(remote_batch) = remote_results {
        unified_results.merge(remote_batch);
    }

    unified_results.order_by(&query, &limits);

    subsonic.build_response(format, StatusCode::OK, Some(unified_results))
}
