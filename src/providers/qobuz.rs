use std::borrow::Cow;
use std::vec::IntoIter;

use anyhow::{Result, anyhow, bail};
use async_trait::async_trait;
use chrono::{Datelike, NaiveDate, Utc};
use reqwest::header::RANGE;
use reqwest::{Client, Response};
use serde::{Deserialize, Deserializer};

use crate::models::{
    AlbumID3,
    AlbumInfo,
    AlbumWithSongsID3,
    ArtistID3,
    ArtistInfo,
    ArtistWithAlbumsID3,
    IdType,
    ItemDate,
    ItemGenre,
    RecordLabel,
    ReplayGain,
    SearchResult,
    Song,
    TagOrigin,
    TopSongs,
    Work,
};
use crate::providers::MusicProvider;
use crate::providers::base::SearchRequest;
use crate::services::{RemoteError, StreamMedia};

#[derive(Debug, Deserialize)]
pub struct QobuzResponse<T> {
    pub data: Option<T>,
}

#[derive(Debug, Deserialize, Default)]
pub struct QobuzList<T> {
    pub items: Option<Vec<T>>,
}

impl<T> IntoIterator for QobuzList<T> {
    type IntoIter = IntoIter<Self::Item>;
    type Item = T;

    fn into_iter(self) -> Self::IntoIter {
        self.items.unwrap_or_default().into_iter()
    }
}

#[derive(Debug, Deserialize, Default)]
pub struct QobuzArtist {
    pub id: u64,
    #[serde(default, deserialize_with = "deserialize_qobuz_name")]
    pub name: Option<String>,
    pub image: Option<QobuzImage>,
    pub albums_count: Option<u32>,
    pub releases: Option<Vec<QobuzReleaseItem>>,
}

#[derive(Debug, Deserialize)]
pub struct QobuzReleaseItem {
    #[serde(rename = "type")]
    pub ty: String,
    pub has_more: bool,
    pub items: Vec<QobuzAlbum>,
}

#[derive(Debug, Deserialize, Default)]
pub struct QobuzAlbum {
    pub id: Option<String>,
    pub title: Option<String>,
    pub artist: Option<QobuzArtist>,
    pub image: Option<QobuzImage>,
    pub tracks_count: Option<u32>,
    pub duration: Option<u32>,
    pub parental_warning: bool,
    pub description: Option<String>,
    pub release_date_original: Option<String>,
    pub genre: Option<QobuzGenre>,
    pub label: Option<QobuzLabel>,
    pub release_type: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct QobuzAlbumWithTracks {
    #[serde(flatten)]
    pub album: QobuzAlbum,
    pub tracks: QobuzList<QobuzTrack>,
}

#[derive(Debug, Deserialize, Default)]
pub struct QobuzTrack {
    pub id: Option<u64>,
    pub title: Option<String>,
    pub album: Option<QobuzAlbum>,
    pub performer: Option<QobuzArtist>,
    pub composer: Option<QobuzArtist>,
    pub track_number: Option<i32>,
    pub media_number: Option<i32>,
    pub duration: Option<i32>,
    pub genre: Option<QobuzGenre>,
    pub isrc: Option<String>,
    pub parental_warning: bool,
    pub audio_info: Option<QobuzAudioInfo>,
    pub work: Option<String>,
    pub copyright: Option<String>,
    pub maximum_channel_count: Option<u32>,
    pub maximum_sampling_rate: Option<f32>,
    pub maximum_bit_depth: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct QobuzImage {
    pub small: Option<String>,
    pub thumbnail: Option<String>,
    pub large: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct QobuzGenre {
    pub id: i32,
    pub name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct QobuzLabel {
    pub id: i64,
    pub name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct QobuzAudioInfo {
    pub replaygain_track_peak: Option<f32>,
    pub replaygain_track_gain: Option<f32>,
}

#[derive(Debug, Deserialize, Default)]
pub struct QobuzSearchData {
    pub albums: QobuzList<QobuzAlbum>,
    pub tracks: QobuzList<QobuzTrack>,
    pub artists: QobuzList<QobuzArtist>,
}

#[derive(Debug, Deserialize)]
pub struct QobuzDownloadData {
    pub url: String,
}

#[derive(Debug, Deserialize)]
pub struct QobuzArtistWrapper {
    pub artist: QobuzArtist,
}

fn deserialize_qobuz_name<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Name {
        String(String),
        Object { display: String },
    }

    Ok(Option::<Name>::deserialize(deserializer)?.map(|n| match n {
        Name::String(s) => s,
        Name::Object { display } => display,
    }))
}

fn parse_qobuz_date(date_str: &Option<String>) -> Option<ItemDate> {
    date_str.as_ref().and_then(|s| {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").ok().map(|d| ItemDate {
            year: Some(d.year() as u16),
            month: Some(d.month() as u8),
            day: Some(d.day() as u8),
        })
    })
}

fn encode_cover_id<'a>(image_opt: &Option<QobuzImage>, prefix: &str) -> Option<Cow<'a, str>> {
    let url = image_opt.as_ref()?.large.as_ref()?;
    let path = if prefix == "al" {
        url.split("images/covers/").nth(1)
    } else {
        url.split("images/artists/covers/").nth(1)
    };
    path.map(|p| Cow::Owned(format!("{}_{}", prefix, p.replace("/", "-"))))
}

fn map_qobuz_genres<'a>(genre_opt: Option<QobuzGenre>) -> Option<Vec<ItemGenre<'a>>> {
    genre_opt.and_then(|g| g.name.map(|name| vec![ItemGenre { name: Cow::Owned(name) }]))
}

impl<'a> TryFrom<QobuzArtist> for ArtistID3<'a> {
    type Error = &'static str;

    fn try_from(art: QobuzArtist) -> Result<Self, Self::Error> {
        let name = art.name.ok_or("Missing artist name")?;
        let artist_cover_id = encode_cover_id(&art.image, "ar");

        Ok(ArtistID3 {
            id: Cow::Owned(art.id.to_string()),
            name: Cow::Owned(name),
            cover_art: artist_cover_id.clone(),
            artist_image_url: artist_cover_id,
            album_count: art.albums_count,
            starred: None,
            music_brainz_id: None,
            sort_name: None,
            roles: None,
        })
    }
}

impl<'a> TryFrom<QobuzArtist> for ArtistWithAlbumsID3<'a> {
    type Error = &'static str;

    fn try_from(mut art: QobuzArtist) -> Result<Self, Self::Error> {
        let releases = art.releases.take().unwrap_or_default();
        let artist = ArtistID3::try_from(art)?;

        let albums = releases
            .into_iter()
            .filter(|r| r.ty == "album")
            .flat_map(|r| r.items)
            .filter_map(|alb| AlbumID3::try_from(alb).ok())
            .collect();

        Ok(ArtistWithAlbumsID3 { base: artist, album: Some(albums) })
    }
}

impl<'a> TryFrom<QobuzAlbum> for AlbumID3<'a> {
    type Error = &'static str;

    fn try_from(alb: QobuzAlbum) -> Result<Self, Self::Error> {
        let parsed_date = parse_qobuz_date(&alb.release_date_original);
        let genre_name = alb.genre.as_ref().and_then(|g| g.name.clone()).map(Cow::Owned);
        let cover_art_id = encode_cover_id(&alb.image, "al");

        let artist_list = alb.artist.and_then(|a| {
            a.name.map(|name| {
                vec![ArtistID3 {
                    id: Cow::Owned(a.id.to_string()),
                    name: Cow::Owned(name),
                    cover_art: None,
                    artist_image_url: None,
                    album_count: None,
                    starred: None,
                    music_brainz_id: None,
                    sort_name: None,
                    roles: None,
                }]
            })
        });

        let display_name =
            artist_list.as_ref().and_then(|list| list.first()).map(|art| art.name.clone());

        let record_labels =
            alb.label.and_then(|l| l.name.map(|name| vec![RecordLabel { name: Cow::Owned(name) }]));

        let explicit = if alb.parental_warning { Some(Cow::Borrowed("explicit")) } else { None };

        Ok(AlbumID3 {
            id: Cow::Owned(alb.id.unwrap_or_default()),
            name: Cow::Owned(alb.title.unwrap_or_default()),
            version: None,
            artist: display_name.clone(),
            artist_id: artist_list.as_ref().and_then(|list| list.first()).map(|art| art.id.clone()),
            cover_art: cover_art_id,
            song_count: alb.tracks_count.unwrap_or(0),
            duration: alb.duration.unwrap_or(0),
            play_count: None,
            created: Utc::now(),
            starred: None,
            year: parsed_date.as_ref().and_then(|d| d.year.map(|y| y as u32)),
            genre: genre_name,
            played: None,
            user_rating: None,
            record_labels,
            music_brainz_id: None,
            genres: map_qobuz_genres(alb.genre),
            artists: artist_list,
            display_artist: display_name,
            release_types: alb.release_type.map(|t| vec![Cow::Owned(t)]),
            moods: None,
            sort_name: None,
            original_release_date: parsed_date.clone(),
            release_date: parsed_date,
            is_compilation: None,
            explicit_status: explicit,
            disc_titles: None,
        })
    }
}

impl<'a> TryFrom<QobuzAlbum> for AlbumInfo<'a> {
    type Error = &'static str;

    fn try_from(alb: QobuzAlbum) -> Result<Self, Self::Error> {
        Ok(AlbumInfo {
            notes: alb.description.map(Cow::Owned),
            music_brainz_id: None,
            large_image_url: alb.image.as_ref().and_then(|img| img.large.clone()).map(Cow::Owned),
            medium_image_url: alb
                .image
                .as_ref()
                .and_then(|img| img.thumbnail.clone())
                .map(Cow::Owned),
            small_image_url: alb.image.as_ref().and_then(|img| img.small.clone()).map(Cow::Owned),
            last_fm_url: None,
        })
    }
}

impl<'a> TryFrom<QobuzTrack> for Song<'a> {
    type Error = &'static str;

    fn try_from(trk: QobuzTrack) -> Result<Self, Self::Error> {
        let genre_name = trk.genre.as_ref().and_then(|g| g.name.clone()).map(Cow::Owned);
        let cover_art_id = trk.album.as_ref().and_then(|a| encode_cover_id(&a.image, "al"));

        let performer_list = trk.performer.and_then(|p| {
            p.name.map(|name| {
                vec![ArtistID3 {
                    id: Cow::Owned(p.id.to_string()),
                    name: Cow::Owned(name),
                    cover_art: None,
                    artist_image_url: None,
                    album_count: None,
                    starred: None,
                    music_brainz_id: None,
                    sort_name: None,
                    roles: None,
                }]
            })
        });

        let performer_name =
            performer_list.as_ref().and_then(|list| list.first()).map(|art| art.name.clone());

        let explicit = if trk.parental_warning { Some(Cow::Borrowed("explicit")) } else { None };

        let replay_gain = ReplayGain {
            base_gain: None,
            track_gain: trk.audio_info.as_ref().and_then(|info| info.replaygain_track_gain),
            track_peak: trk.audio_info.as_ref().and_then(|info| info.replaygain_track_peak),
            album_gain: None,
            album_peak: None,
            fallback_gain: None,
        };

        let work = trk.work.map(|work_name| {
            let works_vec = vec![Work { name: Cow::Owned(work_name), music_brainz_id: None }];
            works_vec
        });

        let sampling_rate = trk.maximum_sampling_rate.map(|r| r * 1000.0).map(|r| r as u32);

        Ok(Song {
            id: Cow::Owned(trk.id.unwrap_or_default().to_string()),
            parent: None,
            is_dir: false,
            title: Cow::Owned(trk.title.unwrap_or_default()),
            album: trk.album.as_ref().and_then(|a| a.title.clone()).map(Cow::Owned),
            artist: performer_name.clone(),
            track: trk.track_number.map(|n| n as u32),
            year: trk.album.as_ref().and_then(|a| {
                parse_qobuz_date(&a.release_date_original).and_then(|d| d.year.map(|y| y as u32))
            }),
            genre: genre_name,
            cover_art: cover_art_id,
            size: None,
            content_type: None,
            suffix: Some(Cow::Borrowed("Remote (Qobuz)")),
            transcoded_content_type: None,
            transcoded_suffix: None,
            duration: trk.duration.map(|d| d as u32),
            bit_rate: None,
            bit_depth: trk.maximum_bit_depth,
            sampling_rate,
            channel_count: trk.maximum_channel_count,
            path: None,
            is_video: Some(false),
            user_rating: None,
            average_rating: None,
            play_count: None,
            disc_number: trk.media_number.map(|m| m as u32),
            created: None,
            starred: None,
            album_id: trk.album.as_ref().and_then(|a| a.id.clone()).map(Cow::Owned),
            artist_id: performer_list
                .as_ref()
                .and_then(|list| list.first())
                .map(|art| art.id.clone()),
            ty: None,
            media_type: Some(Cow::Borrowed("song")),
            bookmark_position: None,
            original_width: None,
            original_height: None,
            played: None,
            bpm: None,
            comment: trk.copyright.as_ref().map(|c| Cow::Owned(c.clone())),
            sort_name: None,
            music_brainz_id: None,
            isrc: trk.isrc.map(|code| vec![Cow::Owned(code)]),
            genres: map_qobuz_genres(trk.genre),
            artists: performer_list,
            display_artist: performer_name,
            album_artists: Some(Vec::new()),
            display_album_artist: None,
            contributors: Some(Vec::new()),
            display_composer: trk.composer.as_ref().and_then(|c| c.name.clone()).map(Cow::Owned),
            moods: None,
            replay_gain: Some(replay_gain),
            explicit_status: explicit,
            works: work,
            movements: None,
            groupings: None,
        })
    }
}

impl TryFrom<QobuzAlbumWithTracks> for AlbumWithSongsID3<'_> {
    type Error = &'static str;

    fn try_from(alb: QobuzAlbumWithTracks) -> Result<Self, Self::Error> {
        let album_id3 = AlbumID3::try_from(alb.album)?;

        let songs = alb
            .tracks
            .items
            .unwrap_or_default()
            .into_iter()
            .filter_map(|trk| Song::try_from(trk).ok())
            .collect();

        Ok(AlbumWithSongsID3 { base: album_id3, song: songs })
    }
}

pub struct QobuzDlClient {
    client: Client,
    url: String,
    provider_id: String,
}

impl QobuzDlClient {
    pub fn new(client: Client, base_url: &str, provider_id: &str) -> Self {
        Self { client, url: base_url.into(), provider_id: provider_id.into() }
    }
}

// TODO: Extract common logic
#[async_trait]
impl MusicProvider for QobuzDlClient {
    fn id(&self) -> &str {
        &self.provider_id
    }

    async fn get_artist<'a>(&self, id: &str) -> Result<ArtistWithAlbumsID3<'a>> {
        let url = format!("{}get-artist", self.url);
        let raw_id = self.get_raw_id(id);

        let response = self.client.get(&url).query(&[("artist_id", raw_id)]).send().await?;

        if !response.status().is_success() {
            return Err(RemoteError::Status(response.status()).into());
        }

        let qobuz_resp = response.json::<QobuzResponse<QobuzArtistWrapper>>().await?;
        let data = qobuz_resp.data.ok_or(RemoteError::ProcessingError)?;

        let mut artist =
            ArtistWithAlbumsID3::try_from(data.artist).map_err(|_| RemoteError::ProcessingError)?;

        artist.tag_origin(self.id(), IdType::Artist);
        Ok(artist)
    }

    async fn get_artist_info<'a>(&self, _id: &str) -> Result<ArtistInfo<'a>> {
        unimplemented!()
    }

    async fn get_album_info<'a>(&self, id: &str) -> Result<AlbumInfo<'a>> {
        let url = format!("{}get-album", self.url);
        let raw_id = self.get_raw_id(id);

        let response = self.client.get(&url).query(&[("album_id", raw_id)]).send().await?;

        if !response.status().is_success() {
            return Err(RemoteError::Status(response.status()).into());
        }

        let qobuz_resp = response.json::<QobuzResponse<QobuzAlbum>>().await?;
        let data = qobuz_resp.data.ok_or(RemoteError::ProcessingError)?;

        AlbumInfo::try_from(data).map_err(|_| RemoteError::ProcessingError.into())
    }

    async fn get_album<'a>(&self, id: &str) -> Result<AlbumWithSongsID3<'a>> {
        let url = format!("{}get-album", self.url);
        let raw_id = self.get_raw_id(id);

        let response = self.client.get(&url).query(&[("album_id", raw_id)]).send().await?;

        if !response.status().is_success() {
            return Err(RemoteError::Status(response.status()).into());
        }

        let qobuz_resp = response.json::<QobuzResponse<QobuzAlbumWithTracks>>().await?;
        let data = qobuz_resp.data.ok_or(RemoteError::EmptyDataset)?;

        let mut album =
            AlbumWithSongsID3::try_from(data).map_err(|_| RemoteError::ProcessingError)?;

        album.tag_origin(self.id(), IdType::Album);
        Ok(album)
    }

    async fn get_song<'a>(&self, id: &str) -> Result<Song<'a>> {
        let url = format!("{}get-music", self.url);
        let raw_id = self.get_raw_id(id);

        let response =
            self.client.get(&url).query(&[("q", raw_id)]).query(&[("offset", 0)]).send().await?;

        if !response.status().is_success() {
            return Err(RemoteError::Status(response.status()).into());
        }

        let qobuz_resp = response.json::<QobuzResponse<QobuzSearchData>>().await?;
        let data = qobuz_resp.data.ok_or(RemoteError::EmptyDataset)?;

        let mut song = data
            .tracks
            .into_iter()
            .filter_map(|item| Song::try_from(item).ok())
            .next()
            .ok_or(RemoteError::EmptyDataset)?;

        song.tag_origin(self.id(), IdType::Song);
        Ok(song)
    }

    async fn get_top_songs<'a>(&self, _artist: &str, _count: Option<u32>) -> Result<TopSongs<'a>> {
        Err(RemoteError::EmptyDataset.into())
    }

    async fn search<'a>(&self, query: &SearchRequest) -> Result<SearchResult<'a>> {
        let url = format!("{}get-music", self.url);

        let response = self
            .client
            .get(&url)
            .query(&[("q", &query.query)])
            .query(&[("offset", query.artist_offset.unwrap_or(0))])
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(RemoteError::Status(response.status()).into());
        }

        let qobuz_resp = response.json::<QobuzResponse<QobuzSearchData>>().await?;

        let data = qobuz_resp.data.unwrap_or_default();

        let artist =
            data.artists.into_iter().filter_map(|item| ArtistID3::try_from(item).ok()).collect();

        let album =
            data.albums.into_iter().filter_map(|item| AlbumID3::try_from(item).ok()).collect();

        let song = data.tracks.into_iter().filter_map(|item| Song::try_from(item).ok()).collect();

        let mut search_result = SearchResult { artist, album, song };
        search_result.tag_origin(self.id(), IdType::Generic);

        Ok(search_result)
    }

    async fn get_cover_art(&self, id: &str, _size: Option<u32>) -> Result<Response> {
        let raw_id = self.get_raw_id(id);

        let target_url = if let Some(remaining) = raw_id.strip_prefix("al_") {
            format!("https://static.qobuz.com/images/covers/{}", remaining.replace("-", "/"))
        } else if let Some(remaining) = id.strip_prefix("ar_") {
            format!(
                "https://static.qobuz.com/images/artists/covers/{}",
                remaining.replace("-", "/")
            )
        } else {
            bail!("Invalid cover art ID format prefix");
        };

        let response = self.client.get(&target_url).send().await?;
        Ok(response)
    }

    async fn stream(&self, req: &StreamMedia) -> Result<Response> {
        let raw_id = self.get_raw_id(&req.request.id);
        let url = format!("{}download-music?track_id={}&quality=27", self.url, raw_id);

        let response = self.client.get(&url).send().await?;

        if !response.status().is_success() {
            return Err(RemoteError::EmptyDataset.into());
        }

        let qobuz_resp = response.json::<QobuzResponse<QobuzDownloadData>>().await?;
        let download_data = qobuz_resp.data.ok_or(RemoteError::EmptyDataset)?;

        let mut media_req = self.client.get(&download_data.url);

        if let Some(ref range) = req.range_header {
            media_req = media_req.header(RANGE, range);
        }

        let res = media_req.send().await?;

        if !res.status().is_success() {
            return Err(anyhow!(RemoteError::EmptyDataset));
        }

        Ok(res)
    }
}
