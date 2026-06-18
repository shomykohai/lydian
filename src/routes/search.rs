use axum::Extension;
use axum::extract::{Request, State};
use axum::response::Response;
use log::debug;
use reqwest::StatusCode;
use tower::Service;

use crate::models::{Mergeable, SearchResult, SubsonicErrorCode, SubsonicRespFormat};
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

    let mut subsonic_resp = subsonic
        .request::<SearchResult<'_>>(
            parts.method.clone(),
            parts.uri.path(),
            parts.uri.query(),
            parts.headers.clone(),
            Some(body),
        )
        .await;

    let Ok(search_req) = subsonic.parse_query::<Search3<'_>>(&parts) else {
        return subsonic.build_error(format, SubsonicErrorCode::RequiredParameterMissing);
    };

    /* If client sent an empty query, we might be doing a global search (get all artists, albums,
     * songs...) In this case, returning remotes would be overkill and not feasible. Better
     * for us to return the origin server results only.
     * TODO: Consider if we want to support adding cached remote results.
     */
    if search_req.request.query.trim().is_empty() {
        if let Ok(resp) = subsonic_resp {
            return subsonic.build_response(format, StatusCode::OK, resp.answer);
        }
        return subsonic.build_error(format, SubsonicErrorCode::Generic);
    }

    let mut service = RemoteLookupService { remotes: &state.remotes, cache: state.cache.clone() };

    let remote_results = service.call(search_req).await;

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

    subsonic.build_response(format, StatusCode::OK, Some(unified_results))
}
