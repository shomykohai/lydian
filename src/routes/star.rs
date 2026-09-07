use std::sync::Arc;

use axum::Extension;
use axum::extract::{Query, Request, State};
use axum::response::Response;
use log::{debug, error};
use reqwest::StatusCode;
use tokio::sync::watch;
use tower::Service;

use crate::models::{AlbumWithSongsID3, Id, IdType, SubsonicErrorCode, SubsonicRespFormat};
use crate::routes::{LydianState, SubsonicExt, Username};
use crate::services::{MigrationRequest, MigrationService};
use crate::storage::keys::StorageKey;
use crate::workers::download::DownloadSource;

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct StarPamams<'a> {
    #[serde(borrow)]
    pub id: Option<Id<'a>>,
    #[serde(borrow)]
    pub album_id: Option<Id<'a>>,
}

pub async fn star(
    State(state): LydianState,
    user: Query<Username>,
    Extension(subsonic): SubsonicExt<'_>,
    format: SubsonicRespFormat,
    req: Request,
) -> Response {
    debug!("Star request: {}", req.uri().path());

    let (parts, body) = req.into_parts();

    let Ok(params) = subsonic.parse_query::<StarPamams>(&parts) else {
        return subsonic.build_error(format, SubsonicErrorCode::RequiredParameterMissing);
    };

    let (tagged_id, mut target_type) = if let Some(ref album_id) = params.album_id {
        (album_id, IdType::Album)
    } else if let Some(ref id) = params.id {
        (id, IdType::Song)
    } else {
        /* Not an ID we can handle, yeet back to subsonic */
        if let Some(res) = subsonic.try_req::<()>(format, &parts, body).await {
            return res;
        }
        return subsonic.build_response(format, StatusCode::OK, Some(()));
    };

    let mut active_provider = None;

    let is_migrated: bool = sqlx::query_scalar(
        r#"
        SELECT EXISTS(
            SELECT 1 FROM migrations
            WHERE (external_id = ? OR internal_id = ?)
        )
        "#,
    )
    .bind(tagged_id.raw())
    .bind(tagged_id.raw())
    .fetch_one(&state.storage.db)
    .await
    .unwrap_or(false);

    if is_migrated {
        if let Some(res) = subsonic.try_req::<()>(format, &parts, body).await {
            return res;
        }
        return subsonic.build_response(format, StatusCode::OK, Some(()));
    }

    let mut target_fetch_id = tagged_id;
    let mut matched_remote_album = None;

    match &tagged_id {
        Id::External { .. } => {
            active_provider = state.remotes.iter().find(|p| p.is_own_id(tagged_id));
        }
        Id::Local { .. } => {
            let path = parts.uri.path().replace("star", "getAlbum");

            let query = if params.album_id.is_some() {
                format!("{}&id={}", parts.uri.query().unwrap_or(""), tagged_id)
            } else {
                parts.uri.query().unwrap_or("").to_string()
            };

            if let Ok(local_album) = subsonic
                .request::<AlbumWithSongsID3<'_>>(
                    parts.method.clone(),
                    &path,
                    Some(&query),
                    parts.headers.clone(),
                    None,
                )
                .await
                && let Some(ref album) = local_album.answer
                && !album.base.id.is_empty()
            {
                target_type = IdType::Album;

                for provider in &state.remotes {
                    if let Some(remote_album) =
                        provider.get_matching_remote_album(&album.base).await
                    {
                        active_provider = Some(provider);
                        matched_remote_album = Some(remote_album);
                        break;
                    }
                }

                if let Some(ref remote_album) = matched_remote_album {
                    target_fetch_id = &remote_album.base.id;
                }
            }
        }
    }

    let Some(provider) = active_provider else {
        if let Some(res) = subsonic.try_req::<()>(format, &parts, body).await {
            return res;
        }
        return subsonic.build_response(format, StatusCode::OK, Some(()));
    };

    let mut songs_to_migrate = Vec::new();
    let mut completion_receivers = Vec::new();

    match target_type {
        IdType::Song => {
            if let Ok(song) = provider.get_song(target_fetch_id).await {
                songs_to_migrate.push((target_fetch_id.clone(), song));
            }
        }
        IdType::Album => {
            if let Ok(album) = provider.get_album(target_fetch_id).await {
                for song in album.song {
                    songs_to_migrate.push((song.id.clone(), song));
                }
            }
        }
        _ => {}
    }

    for (media_id, song) in &songs_to_migrate {
        let storage_key =
            StorageKey::Media { id: media_id.to_string(), format: state.config.download.quality };
        let file_name = storage_key.to_file_name();

        if !state.storage.disk.exists_by_prefix(&file_name).await {
            if let Some(source) = provider.download_source(media_id).await {
                let (tx, rx) = watch::channel(false);
                state.services.queue_download_with_completion(
                    file_name.clone(),
                    source,
                    Arc::new(tx),
                );
                completion_receivers.push(rx);
            }

            if let Some(cover_art) = &song.cover_art {
                let cover_art_key = StorageKey::CoverArt { id: cover_art.to_string(), size: None };
                let cover_art_file_name = cover_art_key.to_file_name();

                if !state.storage.disk.exists_by_prefix(&cover_art_file_name).await
                    && let Ok(resp) = provider.get_cover_art(cover_art, None).await
                    && let Some(source) = DownloadSource::from_response(&resp)
                {
                    let (tx, rx) = watch::channel(false);
                    state.services.queue_download_with_completion(
                        cover_art_file_name.clone(),
                        source,
                        Arc::new(tx),
                    );
                    completion_receivers.push(rx);
                }
            }
        }
    }

    if !songs_to_migrate.is_empty() {
        for mut rx in completion_receivers {
            while !*rx.borrow() {
                if rx.changed().await.is_err() {
                    break;
                }
            }
        }

        /* Time to migrate!! */
        let mut migration_service = MigrationService { state: Arc::clone(&state) };
        for (media_id, song) in songs_to_migrate {
            let file_name = StorageKey::Media {
                id: media_id.to_string(),
                format: state.config.download.quality,
            }
            .to_file_name();

            if !state.storage.disk.exists_by_prefix(&file_name).await {
                debug!(
                    "File {} still doesn't exist after waiting for download, skipping migration",
                    file_name
                );
                continue;
            }

            let user =
                if state.config.subsonic.multi_user_library { Some(user.u.as_str()) } else { None };
            let migration_req = MigrationRequest { old_path: &file_name, metadata: &song, user };
            if let Err(e) = migration_service.call(migration_req).await {
                error!("Migration failed for song {}: {}", file_name, e);
            }
        }

        if target_type == IdType::Album {
            let (external_id, service) = match &tagged_id {
                Id::External { key, origin, .. } => (key.as_ref(), origin.as_ref()),
                Id::Local { raw } => (raw.as_ref(), "local"),
            };

            if let Err(e) = sqlx::query(
                r#"
                INSERT INTO migrations (external_id, id_type, output_file_path, service)
                VALUES (?, 'album', NULL, ?)
                ON CONFLICT(external_id, id_type, service) DO NOTHING
                "#,
            )
            .bind(external_id)
            .bind(service)
            .execute(&state.storage.db)
            .await
            {
                error!("Failed to register album migration: {}", e);
            }
        }

        subsonic.scan(format).await;
    }

    if let Some(final_response) = subsonic.try_req::<()>(format, &parts, body).await {
        return final_response;
    }
    subsonic.build_response(format, StatusCode::OK, Some(()))
}
