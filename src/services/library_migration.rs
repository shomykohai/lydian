use std::sync::Arc;
use std::task::{Context, Poll};

use audiotags::{Album as AudioTagAlbum, MimeType as AudioMimeType, Picture, Tag};
use futures::future::BoxFuture;
use log::{debug, error};
use mime_type::{MimeFormat, MimeType};
use tokio::io::AsyncReadExt;
use tokio::task;
use tower::Service;

use crate::AppState;
use crate::config::{
    TEMPLATE_ALBUM,
    TEMPLATE_ARTIST,
    TEMPLATE_TITLE,
    TEMPLATE_TRACK,
    TEMPLATE_USERNAME,
};
use crate::models::Song;
use crate::services::ServiceError::{self, TaggingError};
use crate::storage::keys::StorageKey;

macro_rules! update_if_none {
    ($tag:expr, $getter:ident, $setter:ident, $val:expr, $modified:expr) => {
        if $tag.$getter().is_none() {
            $tag.$setter($val);
            $modified = true;
        }
    };
}

struct SongMetadata {
    title: Option<String>,
    cover_art: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    album_artist: Option<String>,
    composer: Option<String>,
    genre: Option<String>,
    year: Option<i32>,
    track: Option<u16>,
    disc_number: Option<u16>,
}

pub struct MigrationRequest<'a> {
    pub old_path: &'a str,
    pub metadata: &'a Song<'a>,
    pub user: Option<&'a str>,
}

pub struct MigrationService {
    pub state: Arc<AppState>,
}

impl<'a> Service<MigrationRequest<'a>> for MigrationService {
    type Error = ServiceError;
    type Future = BoxFuture<'a, Result<Self::Response, Self::Error>>;
    type Response = ();

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: MigrationRequest<'a>) -> Self::Future {
        let old_path = req.old_path;
        let song = req.metadata;

        let mut path = if self.state.config.subsonic.multi_user_library {
            format!(
                "{}/{}/{}",
                self.state.config.subsonic.music_folder,
                self.state.config.subsonic.multi_user_library_template,
                self.state.config.subsonic.folder_template
            )
        } else {
            format!(
                "{}/{}",
                self.state.config.subsonic.music_folder, self.state.config.subsonic.folder_template
            )
        };
        let artist = song.artist.as_deref().unwrap_or("Unknown Artist");
        let album = song.album.as_deref().unwrap_or("Unknown Album");
        let track = song.track.map(|t| t.to_string()).unwrap_or_else(|| "00".to_string());
        let title = song.title.as_ref();

        path = path.replace(TEMPLATE_ARTIST, artist);
        path = path.replace(TEMPLATE_ALBUM, album);
        path = path.replace(TEMPLATE_TRACK, &track);
        path = path.replace(TEMPLATE_TITLE, title);

        if let Some(user) = req.user {
            path = path.replace(TEMPLATE_USERNAME, user);
        }

        debug!("Migrating file from {} to {}", old_path, path);

        let state = self.state.clone();
        Box::pin(async move {
            if tokio::fs::try_exists(&path).await.unwrap_or(false) {
                return Ok(());
            }

            let final_path = match state.storage.disk.move_file_by_prefix(old_path, &path).await {
                Ok(p) => p,
                Err(e) => {
                    error!("Failed to migrate file from {} to {}: {}", old_path, path, e);
                    return Err(ServiceError::Migration {
                        from: old_path.to_string(),
                        to: path.to_string(),
                        source: e,
                    });
                }
            };

            let metadata = SongMetadata {
                title: Some((*song.title).to_string()),
                cover_art: song.cover_art.as_ref().map(|s| s.to_string()),
                artist: song.artist.as_deref().map(String::from),
                album: song.album.as_deref().map(String::from),
                album_artist: song.display_album_artist.as_deref().map(String::from),
                composer: song.display_composer.as_deref().map(String::from),
                genre: song.genre.as_deref().map(String::from),
                year: song.year.map(|y| y as i32),
                track: song.track.map(|t| t as u16),
                disc_number: song.disc_number.map(|d| d as u16),
            };

            let (cover_art_data, mime_type) = match &metadata.cover_art {
                Some(cover_art) => {
                    let disk = &state.storage.disk;

                    let key = &StorageKey::CoverArt { id: cover_art.to_string(), size: None }
                        .to_file_name();

                    if let (Ok(mut file), Some(ext)) = (
                        disk.get_file_by_prefix(key).await,
                        disk.get_extension_by_prefix(key).await,
                    ) {
                        let mut buf = Vec::new();

                        let data = file.read_to_end(&mut buf).await.ok().map(|_| buf);
                        let mime_str = MimeType::from_ext(&ext).map(|m| m.to_string());

                        (data, mime_str)
                    } else {
                        (None, None)
                    }
                }
                None => (None, None),
            };

            let final_path_str = final_path.clone().to_string_lossy().to_string();
            task::spawn_blocking(move || {
                let mut tag = Tag::new().read_from_path(&final_path).map_err(TaggingError)?;
                let mut tag_modified = false;

                if let Some(ref title) = metadata.title {
                    update_if_none!(tag, title, set_title, title.as_str(), tag_modified);
                }

                if let Some(ref artist) = metadata.artist {
                    update_if_none!(tag, artist, set_artist, artist.as_str(), tag_modified);
                }

                if let Some(ref album_name) = metadata.album
                    && tag.album().is_none()
                {
                    tag.set_album(AudioTagAlbum {
                        title: album_name.as_str(),
                        artist: metadata.artist.as_deref(),
                        cover: None,
                    });
                    tag_modified = true;
                }

                if let Some(ref aa) = metadata.album_artist {
                    update_if_none!(tag, album_artist, set_album_artist, aa.as_str(), tag_modified);
                }

                if let Some(ref comp) = metadata.composer {
                    update_if_none!(tag, composer, set_composer, comp.to_string(), tag_modified);
                }

                if let Some(ref genre) = metadata.genre {
                    update_if_none!(tag, genre, set_genre, genre.as_str(), tag_modified);
                }

                if let Some(year) = metadata.year {
                    update_if_none!(tag, year, set_year, year, tag_modified);
                }

                if let Some(track) = metadata.track {
                    update_if_none!(tag, track_number, set_track_number, track, tag_modified);
                }

                if let Some(disc) = metadata.disc_number {
                    update_if_none!(tag, disc_number, set_disc_number, disc, tag_modified);
                }

                let audio_mime = mime_type.and_then(|m| AudioMimeType::try_from(m.as_str()).ok());

                if let (Some(data), Some(mime)) = (&cover_art_data, audio_mime)
                    && tag.album_cover().is_none()
                {
                    let picture = Picture::new(data, mime);
                    tag.set_album_cover(picture);
                    tag_modified = true;
                }

                if tag_modified {
                    tag.write_to_path(&final_path.into_os_string().to_string_lossy()).unwrap();
                }

                Ok::<(), ServiceError>(())
            })
            .await??;

            sqlx::query(
                r#"
                INSERT INTO migrations (external_id, id_type, output_file_path, service)
                VALUES (?, ?, ?, ?)
                ON CONFLICT(external_id, id_type, service) DO NOTHING
                "#,
            )
            .bind(song.id.to_string())
            .bind(song.id.id_type().as_str())
            .bind(final_path_str)
            .bind(song.id.service().unwrap_or("unknown"))
            .execute(&state.storage.db)
            .await?;

            debug!("Successfully migrated file from {} to {}", old_path, path);

            Ok(())
        })
    }
}
