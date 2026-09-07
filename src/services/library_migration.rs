use std::path::Path;
use std::sync::Arc;
use std::task::{Context, Poll};

use anyhow::anyhow;
use futures::future::BoxFuture;
use lofty::config::WriteOptions;
use lofty::picture::{MimeType as PictureMime, Picture, PictureType};
use lofty::prelude::{Accessor, ItemKey, TagExt, TaggedFileExt};
use lofty::probe::Probe;
use lofty::tag::Tag;
use lofty::tag::items::Timestamp;
use log::debug;
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
use crate::services::ServiceError;
use crate::storage::keys::StorageKey;
use crate::utils::string::path_segment;

struct Cover {
    data: Vec<u8>,
    mime: String,
}

struct SongTags {
    title: String,
    artist: Option<String>,
    album: Option<String>,
    album_artist: Option<String>,
    composer: Option<String>,
    genre: Option<String>,
    comment: Option<String>,
    isrc: Option<String>,
    music_brainz_id: Option<String>,
    year: Option<u16>,
    track: Option<u32>,
    disc_number: Option<u32>,
    bpm: Option<u32>,
    track_gain: Option<f32>,
    track_peak: Option<f32>,
    cover: Option<Cover>,
}

impl SongTags {
    fn of(song: &Song<'_>) -> Self {
        Self {
            title: song.title.to_string(),
            artist: song.artist.as_deref().map(String::from),
            album: song.album.as_deref().map(String::from),
            album_artist: song
                .display_album_artist
                .as_deref()
                .or(song.artist.as_deref())
                .map(String::from),
            composer: song.display_composer.as_deref().map(String::from),
            genre: song
                .genre
                .as_deref()
                .map(String::from)
                .or_else(|| song.genres.iter().flatten().next().map(|g| g.name.to_string())),
            comment: song.comment.as_deref().map(String::from),
            isrc: song.isrc.iter().flatten().next().map(|code| code.to_string()),
            music_brainz_id: song.music_brainz_id.as_deref().map(String::from),
            year: song.year.and_then(|year| u16::try_from(year).ok()),
            track: song.track,
            disc_number: song.disc_number,
            bpm: song.bpm,
            track_gain: song.replay_gain.as_ref().and_then(|gain| gain.track_gain),
            track_peak: song.replay_gain.as_ref().and_then(|gain| gain.track_peak),
            cover: None,
        }
    }

    fn write(self, path: &Path) -> Result<(), ServiceError> {
        let file = Probe::open(path)?.read()?;
        let mut tag =
            file.primary_tag().cloned().unwrap_or_else(|| Tag::new(file.primary_tag_type()));

        tag.set_title(self.title);

        let text = [
            (ItemKey::AlbumArtist, self.album_artist),
            (ItemKey::Composer, self.composer),
            (ItemKey::Isrc, self.isrc),
            (ItemKey::MusicBrainzRecordingId, self.music_brainz_id),
            (ItemKey::Bpm, self.bpm.map(|bpm| bpm.to_string())),
            (ItemKey::ReplayGainTrackGain, self.track_gain.map(|gain| format!("{gain:.2} dB"))),
            (ItemKey::ReplayGainTrackPeak, self.track_peak.map(|peak| format!("{peak:.6}"))),
        ];

        for (key, value) in text {
            if let Some(value) = value {
                tag.insert_text(key, value);
            }
        }

        if let Some(artist) = self.artist {
            tag.set_artist(artist);
        }
        if let Some(album) = self.album {
            tag.set_album(album);
        }
        if let Some(genre) = self.genre {
            tag.set_genre(genre);
        }
        if let Some(comment) = self.comment {
            tag.set_comment(comment);
        }
        if let Some(year) = self.year {
            tag.set_date(Timestamp { year, ..Timestamp::default() });
        }
        if let Some(track) = self.track {
            tag.set_track(track);
        }
        if let Some(disc) = self.disc_number {
            tag.set_disk(disc);
        }

        if let Some(cover) = self.cover
            && tag.pictures().is_empty()
        {
            tag.push_picture(
                Picture::unchecked(cover.data)
                    .pic_type(PictureType::CoverFront)
                    .mime_type(PictureMime::from_str(&cover.mime))
                    .build(),
            );
        }

        tag.save_to_path(path, WriteOptions::default())?;

        Ok(())
    }
}

async fn load_cover(state: &AppState, song: &Song<'_>) -> Option<Cover> {
    let cover_art = song.cover_art.as_ref()?;
    let key = StorageKey::CoverArt { id: cover_art.to_string(), size: None }.to_file_name();
    let disk = &state.storage.disk;

    let mut file = disk.get_file_by_prefix(&key).await.ok()?;
    let extension = disk.get_extension_by_prefix(&key).await?;
    let mime = MimeType::from_ext(&extension)?.to_string();

    let mut data = Vec::new();
    file.read_to_end(&mut data).await.ok()?;

    Some(Cover { data, mime })
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

        let artist = path_segment(song.artist.as_deref().unwrap_or("Unknown Artist"));
        let album = path_segment(song.album.as_deref().unwrap_or("Unknown Album"));
        let track = song.track.map_or_else(|| "00".to_string(), |t| t.to_string());
        let title = path_segment(&song.title);

        path = path.replace(TEMPLATE_ARTIST, &artist);
        path = path.replace(TEMPLATE_ALBUM, &album);
        path = path.replace(TEMPLATE_TRACK, &track);
        path = path.replace(TEMPLATE_TITLE, &title);

        if let Some(user) = req.user {
            path = path.replace(TEMPLATE_USERNAME, &path_segment(user));
        }

        let state = self.state.clone();
        Box::pin(async move {
            let migration_failed = |source: anyhow::Error| ServiceError::Migration {
                from: old_path.to_string(),
                to: path.clone(),
                source,
            };

            let Some(staged) = state.storage.disk.resolve_path_prefix(old_path).await else {
                return Err(migration_failed(anyhow!(std::io::ErrorKind::NotFound)));
            };

            let destination = match staged.extension() {
                Some(extension) => format!("{path}.{}", extension.to_string_lossy()),
                None => path.clone(),
            };

            if tokio::fs::try_exists(&destination).await.unwrap_or(false) {
                return Ok(());
            }

            let mut tags = SongTags::of(song);
            tags.cover = load_cover(&state, song).await;

            task::spawn_blocking(move || tags.write(&staged)).await??;

            let final_path = state
                .storage
                .disk
                .move_file_by_prefix(old_path, &path)
                .await
                .map_err(&migration_failed)?;

            sqlx::query(
                r#"
                INSERT INTO migrations (external_id, id_type, output_file_path, service)
                VALUES (?, ?, ?, ?)
                ON CONFLICT(external_id, id_type, service) DO NOTHING
                "#,
            )
            .bind(song.id.to_string())
            .bind(song.id.id_type().as_str())
            .bind(final_path.to_string_lossy())
            .bind(song.id.service().unwrap_or("unknown"))
            .execute(&state.storage.db)
            .await?;

            debug!("Successfully migrated file from {old_path} to {}", final_path.display());

            Ok(())
        })
    }
}
