use axum::Extension;
use axum::extract::{Request, State};
use axum::response::{IntoResponse, Response};
use log::debug;
use reqwest::StatusCode;
use tower::Service;

use crate::models::{
    AlbumInfo,
    AlbumWithSongsID3,
    FuzzyMatch,
    Mergeable,
    SubsonicErrorCode,
    SubsonicRespFormat,
};
use crate::routes::{LydianState, SubsonicExt};
use crate::services::{GetAlbum, GetAlbumByName, GetAlbumInfo, RemoteLookupService};

pub async fn get_album_info<'a>(
    State(state): LydianState,
    Extension(subsonic): SubsonicExt<'a>,
    format: SubsonicRespFormat,
    req: Request,
) -> Response {
    debug!("Album info request: {}", req.uri().path());

    let (parts, body) = req.into_parts();

    let Ok(op) = subsonic.parse_query::<GetAlbumInfo<'_>>(&parts) else {
        return subsonic.build_error(format, SubsonicErrorCode::RequiredParameterMissing);
    };

    if !op.id.is_external()
        && let Some(subsonic_resp) = subsonic.try_req::<AlbumInfo<'a>>(format, &parts, body).await
    {
        return subsonic_resp;
    }

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

    let Ok(op) = subsonic.parse_query::<GetAlbum<'_>>(&parts) else {
        return subsonic.build_error(format, SubsonicErrorCode::RequiredParameterMissing);
    };

    let mut subsonic_resp = if !op.id.is_external() {
        Some(
            subsonic
                .request::<AlbumWithSongsID3<'_>>(
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

    let is_migrated: bool = sqlx::query_scalar(
        r#"
        SELECT EXISTS(
            SELECT 1 FROM migrations
            WHERE (external_id = ? OR internal_id = ?) AND id_type = 'album'
        )
        "#,
    )
    .bind(op.id.raw())
    .bind(op.id.raw())
    .fetch_one(&state.storage.db)
    .await
    .unwrap_or(false);

    /* We mght have migrated the whole album, so querying is just
     * wasting time. So, we just return the subsonic response since
     * we are sure it contains the full album.
     */
    if is_migrated && let Some(Ok(sub_resp)) = subsonic_resp {
        /* TODO: Consider if we should update the migration with the new navidrome ID if not set */
        return sub_resp.into_response();
    }

    let mut service = RemoteLookupService { remotes: &state.remotes, cache: state.cache.clone() };

    let mut unified_result = AlbumWithSongsID3::default();
    let mut origin_track_count = 0;

    if let Some(Ok(ref mut resp)) = subsonic_resp
        && !resp.is_err()
        && let Some(ref mut answer) = resp.answer
    {
        origin_track_count = answer.base.song_count;
        unified_result = std::mem::take(answer);
    }

    let search_query: String;
    if !unified_result.base.id.is_empty() {
        search_query = format!(
            "{} {}",
            unified_result.base.name,
            unified_result.base.artist.as_deref().unwrap_or("")
        );
        let album_op = GetAlbumByName { name: &search_query };
        let albums = service.call(album_op).await.ok();

        if let Some(albums) = albums {
            for album in albums {
                if unified_result.is_similar_to(&album) {
                    if album.base.song_count == origin_track_count {
                        /* This means this album can be marked as migrated already,
                         * so we won't be hitting the remotes for this album again in the
                         * future, since it is already full!
                         * This might happen if the album was added externally and not
                         * by lydian.
                         */
                        sqlx::query(
                            r#"
                            INSERT INTO migrations (external_id, internal_id, id_type, output_file_path, service)
                            VALUES (?, ?, 'album', NULL, 'local')
                            ON CONFLICT(external_id, id_type, service) DO UPDATE SET internal_id = excluded.internal_id
                            "#,
                        )
                        .bind(album.base.id.to_string())
                        .bind(op.id.raw())
                        .execute(&state.storage.db)
                        .await
                        .ok();
                    }

                    unified_result.merge(album);
                    break;
                }
            }
        }
    } else {
        if let Ok(remote_album) = service.call(op).await {
            unified_result = remote_album;
        }
    }

    if unified_result.base.id.is_empty() {
        subsonic.build_error(format, SubsonicErrorCode::NotFound)
    } else {
        subsonic.build_response(format, StatusCode::OK, Some(unified_result))
    }
}
