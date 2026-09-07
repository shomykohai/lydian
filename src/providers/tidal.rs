use std::collections::HashSet;
use std::fmt::Display;
use std::str::FromStr;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use async_trait::async_trait;
use axum::body::Bytes;
use chrono::Utc;
use futures::StreamExt;
use log::{debug, warn};
use moka::future::Cache;
use regex::Regex;
use reqwest::header::{
    ACCEPT_RANGES,
    AUTHORIZATION,
    CONTENT_LENGTH,
    CONTENT_RANGE,
    CONTENT_TYPE,
    RANGE,
};
use reqwest::{Body, Client, Response, StatusCode};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use thiserror::Error as TError;
use tokio::sync::{Mutex, OnceCell, RwLock};

use crate::config::TidalProviderConfig;
use crate::models::{
    AlbumID3,
    AlbumInfo,
    AlbumWithSongsID3,
    ArtistID3,
    ArtistInfo,
    ArtistWithAlbumsID3,
    Id,
    IdType,
    Inherit,
    ItemDate,
    SearchResult,
    Song,
    SubsonicStreamFormat,
    TagOrigin,
    TopSongs,
    parse_timestamp,
};
use crate::providers::MusicProvider;
use crate::providers::base::{ProviderError, SearchRequest};
use crate::services::{RemoteError, StreamMedia};
use crate::utils::fmp4::Fmp4;
use crate::utils::mime::MediaType;
use crate::utils::string::normalize;
use crate::workers::download::{DownloadSource, SegmentLayout};

#[derive(TError, Debug)]
pub enum TidalError {
    #[error("Token request returned status code: {0}")]
    Token(StatusCode),
    #[error("Manifest response for track {0} carried no uri")]
    MissingManifestUri(String),
    #[error("DASH manifest has no audio representation")]
    MissingRepresentation,
    #[error("DASH manifest has no SegmentTemplate")]
    MissingSegmentTemplate,
    #[error("Unsupported DASH media template: {0}")]
    UnsupportedTemplate(String),
    #[error("Live DASH manifests are not supported")]
    LiveManifest,
    #[error("DASH manifest declares no segments")]
    MissingSegments,
    #[error("Segment {0} reported no length")]
    MissingSegmentLength(String),
    #[error("Segment {0} is shorter than the boxes it declares")]
    TruncatedSegment(String),
    #[error("Unsupported resource type")]
    UnsupportedResourceType,
    #[error("{0:#}")]
    Source(Arc<anyhow::Error>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Hash)]
enum TidalQuality {
    Low,
    High,
    #[default]
    Lossless,
    HiRes,
}

impl TidalQuality {
    const fn as_str(self) -> &'static str {
        match self {
            Self::HiRes => "HI_RES",
            Self::Lossless => "LOSSLESS",
            Self::High => "HIGH",
            Self::Low => "LOW",
        }
    }

    const fn is_lossless(self) -> bool {
        matches!(self, Self::HiRes | Self::Lossless)
    }

    const fn content_type(self) -> &'static str {
        if self.is_lossless() { "audio/flac" } else { "audio/mp4" }
    }

    fn suffix(self) -> &'static str {
        MediaType::extension(self.content_type()).unwrap_or("flac")
    }

    const fn bitrate_kbps(self) -> u32 {
        match self {
            Self::HiRes => 5979,
            Self::Lossless => 898,
            Self::High => 320,
            Self::Low => 97,
        }
    }

    const fn sampling_rate(self) -> u32 {
        match self {
            Self::HiRes => 176_400,
            _ => 44_100,
        }
    }

    const fn bit_depth(self) -> Option<u32> {
        match self {
            Self::HiRes => Some(24),
            Self::Lossless => Some(16),
            _ => None,
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_uppercase().replace(['-', ' '], "_").as_str() {
            "HI_RES" | "HIRES" => Some(Self::HiRes),
            "LOSSLESS" | "FLAC" => Some(Self::Lossless),
            "HIGH" | "AAC" => Some(Self::High),
            "LOW" => Some(Self::Low),
            _ => None,
        }
    }

    fn resolve(format: SubsonicStreamFormat, cap: Option<Self>) -> Self {
        let wanted = match format {
            SubsonicStreamFormat::Mp3 | SubsonicStreamFormat::Ogg | SubsonicStreamFormat::Aac => {
                Self::High
            }
            SubsonicStreamFormat::Flac | SubsonicStreamFormat::Raw => cap.unwrap_or_default(),
        };

        cap.map_or(wanted, |cap| wanted.min(cap))
    }

    fn estimated_size(self, duration_secs: u32) -> Option<u64> {
        (!self.is_lossless() && duration_secs > 0)
            .then(|| u64::from(self.bitrate_kbps()) * 125 * u64::from(duration_secs))
    }
}

#[derive(Debug, Deserialize)]
struct TidalToken {
    access_token: String,
    expires_in: u64,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
struct TidalList<T> {
    items: Vec<T>,
    #[serde(rename = "totalNumberOfItems")]
    total: Option<u32>,
}

impl<T> Default for TidalList<T> {
    fn default() -> Self {
        Self { items: Vec::new(), total: None }
    }
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct TidalAlbumItem {
    item: TidalTrack,
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Debug, Deserialize, Default, Clone)]
#[serde(default)]
struct TidalArtist {
    id: u64,
    name: String,
    picture: Option<String>,
}

#[derive(Debug, Deserialize, Default, Clone)]
#[serde(default, rename_all = "camelCase")]
struct TidalAlbum {
    id: u64,
    title: String,
    version: Option<String>,
    cover: Option<String>,
    duration: Option<u32>,
    number_of_tracks: Option<u32>,
    release_date: Option<String>,
    stream_start_date: Option<String>,
    copyright: Option<String>,
    explicit: Option<bool>,
    artists: Vec<TidalArtist>,
    artist: Option<TidalArtist>,
}

impl TidalAlbum {
    fn title_key(&self) -> String {
        let version = self.version.as_deref().unwrap_or_default();

        format!("{}\u{1}{}", normalize(self.title.trim()), normalize(version.trim()))
    }
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct TidalTrack {
    id: u64,
    title: String,
    version: Option<String>,
    duration: Option<u32>,
    track_number: Option<u32>,
    volume_number: Option<u32>,
    isrc: Option<String>,
    explicit: Option<bool>,
    copyright: Option<String>,
    bpm: Option<u32>,
    artists: Vec<TidalArtist>,
    album: Option<TidalAlbum>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct TidalSearch {
    artists: TidalList<TidalArtist>,
    albums: TidalList<TidalAlbum>,
    tracks: TidalList<TidalTrack>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct TidalEditorial {
    text: String,
}

struct EditorialFilters {
    links: Regex,
    breaks: Regex,
}

fn editorial_filters() -> &'static EditorialFilters {
    static INSTANCE: OnceLock<EditorialFilters> = OnceLock::new();

    INSTANCE.get_or_init(|| EditorialFilters {
        links: Regex::new(r"\[wimpLink[^\]]*\]|\[/wimpLink\]").unwrap(),
        breaks: Regex::new(r"(?i)<br\s*/?>").unwrap(),
    })
}

impl TidalEditorial {
    fn plain_text(&self) -> Option<String> {
        let filters = editorial_filters();

        let text = filters.links.replace_all(&self.text, "");
        let text = filters.breaks.replace_all(&text, "\n");
        let text = text.trim();

        (!text.is_empty()).then(|| text.to_string())
    }
}

/* Artists in tidal are in 3 different keys */
fn map_tidal_artists<'a, 'b>(
    artists: impl IntoIterator<Item = &'b TidalArtist>,
) -> Vec<ArtistID3<'a>> {
    let mut seen = HashSet::new();

    artists
        .into_iter()
        .filter(|artist| seen.insert(artist.id))
        .filter_map(|artist| ArtistID3::try_from(artist.clone()).ok())
        .collect()
}

impl<'a> TryFrom<TidalArtist> for ArtistID3<'a> {
    type Error = ProviderError;

    fn try_from(artist: TidalArtist) -> Result<Self, Self::Error> {
        if artist.id == 0 {
            return Err(ProviderError::MissingId(IdType::Artist));
        }

        Ok(Self {
            id: artist.id.into(),
            name: artist.name.into(),
            cover_art: artist.picture.filter(|p| !p.is_empty()).map(Into::into),
            ..Default::default()
        })
    }
}

impl<'a> TryFrom<TidalAlbum> for AlbumID3<'a> {
    type Error = ProviderError;

    fn try_from(album: TidalAlbum) -> Result<Self, Self::Error> {
        if album.id == 0 {
            return Err(ProviderError::MissingId(IdType::Album));
        }

        let artists = map_tidal_artists(album.artists.iter().chain(album.artist.iter()));
        let display = artists.first().map(|artist| artist.name.to_string());
        let release = album.release_date.as_deref().and_then(|d| ItemDate::from_str(d).ok());
        let explicit = album.explicit.map(|flag| if flag { "explicit" } else { "clean" });

        let created = album
            .stream_start_date
            .as_deref()
            .and_then(parse_timestamp)
            .or_else(|| release.as_ref().and_then(ItemDate::midnight_utc))
            .unwrap_or_else(Utc::now);

        Ok(Self {
            id: album.id.into(),
            name: album.title.into(),
            version: album.version.map(Into::into),
            artist: display.clone().map(Into::into),
            artist_id: artists.first().map(|artist| artist.id.clone()),
            cover_art: album.cover.filter(|c| !c.is_empty()).map(Into::into),
            song_count: album.number_of_tracks.unwrap_or_default(),
            duration: album.duration.unwrap_or_default(),
            created,
            year: release.as_ref().and_then(|d| d.year.map(u32::from)),
            original_release_date: release.clone(),
            release_date: release,
            display_artist: display.map(Into::into),
            explicit_status: explicit.map(Into::into),
            artists: (!artists.is_empty()).then_some(artists),
            ..Default::default()
        })
    }
}

impl<'a> TryFrom<TidalAlbum> for AlbumInfo<'a> {
    type Error = ProviderError;

    fn try_from(album: TidalAlbum) -> Result<Self, Self::Error> {
        let cover = album.cover.filter(|c| !c.is_empty());
        let image = |size: u32| cover.as_deref().map(|uuid| TidalImage::url(uuid, size).into());

        Ok(Self {
            notes: album.copyright.map(Into::into),
            music_brainz_id: None,
            last_fm_url: None,
            small_image_url: image(160),
            medium_image_url: image(320),
            large_image_url: image(1280),
        })
    }
}

impl<'a> TryFrom<TidalTrack> for Song<'a> {
    type Error = ProviderError;

    fn try_from(track: TidalTrack) -> Result<Self, Self::Error> {
        if track.id == 0 {
            return Err(ProviderError::MissingId(IdType::Song));
        }

        let artists = map_tidal_artists(&track.artists);
        let display = artists.first().map(|artist| artist.name.to_string());
        let explicit = track.explicit.map(|flag| if flag { "explicit" } else { "clean" });

        let title = match track.version.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
            Some(version) => format!("{} ({version})", track.title),
            None => track.title,
        };

        let album = track.album;
        let album_id = album.as_ref().filter(|a| a.id != 0).map(|a| Id::from(a.id));

        Ok(Self {
            id: track.id.into(),
            is_dir: false,
            title: title.into(),
            album: album.as_ref().map(|a| a.title.clone().into()),
            album_id: album_id.clone(),
            parent: album_id,
            artist: display.clone().map(Into::into),
            display_artist: display.map(Into::into),
            artist_id: artists.first().map(|artist| artist.id.clone()),
            cover_art: album
                .as_ref()
                .and_then(|a| a.cover.clone())
                .filter(|c| !c.is_empty())
                .map(Into::into),
            track: track.track_number,
            disc_number: track.volume_number,
            duration: track.duration,
            year: album
                .as_ref()
                .and_then(|a| a.release_date.as_deref().and_then(|d| ItemDate::from_str(d).ok()))
                .and_then(|d| d.year.map(u32::from)),
            isrc: track.isrc.map(|code| vec![code.into()]),
            bpm: track.bpm,
            comment: track.copyright.map(Into::into),
            explicit_status: explicit.map(Into::into),
            is_video: Some(false),
            media_type: Some("song".into()),
            ty: Some("music".into()),
            artists: (!artists.is_empty()).then_some(artists),
            ..Default::default()
        })
    }
}

struct TidalImage;

impl TidalImage {
    const MAX_ATTEMPTS: usize = 3;
    /* The sizes `resources.tidal.com` serves for *both* kinds of image. */
    const SIZES: [u32; 4] = [160, 320, 750, 1280];

    fn url(uuid: &str, size: u32) -> String {
        format!("https://resources.tidal.com/images/{}/{size}x{size}.jpg", uuid.replace('-', "/"))
    }

    fn sizes_for(wanted: u32) -> impl Iterator<Item = u32> {
        Self::SIZES
            .into_iter()
            .filter(move |size| *size >= wanted)
            .chain(Self::SIZES.into_iter().rev().filter(move |size| *size < wanted))
            .take(Self::MAX_ATTEMPTS)
    }
}

#[derive(Debug, Deserialize)]
struct TidalManifestResponse {
    data: TidalManifestData,
}

#[derive(Debug, Deserialize)]
struct TidalManifestData {
    attributes: TidalManifestAttributes,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct TidalManifestAttributes {
    track_presentation: Option<String>,
    uri: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Mpd {
    #[serde(rename = "Period", default)]
    periods: Vec<MpdPeriod>,
}

#[derive(Debug, Deserialize)]
struct MpdPeriod {
    #[serde(rename = "AdaptationSet", default)]
    adaptation_sets: Vec<MpdAdaptationSet>,
}

#[derive(Debug, Deserialize)]
struct MpdAdaptationSet {
    #[serde(rename = "Representation", default)]
    representations: Vec<MpdRepresentation>,
}

#[derive(Debug, Deserialize)]
struct MpdRepresentation {
    #[serde(rename = "SegmentTemplate")]
    segment_template: Option<MpdSegmentTemplate>,
}

#[derive(Debug, Deserialize)]
struct MpdSegmentTemplate {
    #[serde(rename = "@initialization")]
    initialization: String,
    #[serde(rename = "@media")]
    media: String,
    #[serde(rename = "@startNumber", default = "MpdSegmentTemplate::first_segment")]
    start_number: u64,
    #[serde(rename = "SegmentTimeline")]
    segment_timeline: Option<MpdSegmentTimeline>,
}

impl MpdSegmentTemplate {
    const fn first_segment() -> u64 {
        1
    }
}

#[derive(Debug, Deserialize)]
struct MpdSegmentTimeline {
    #[serde(rename = "S", default)]
    entries: Vec<MpdSegment>,
}

#[derive(Debug, Deserialize)]
struct MpdSegment {
    #[serde(rename = "@r", default)]
    repeats: i64,
}

#[derive(Debug)]
struct DashManifest {
    urls: Arc<[Arc<str>]>,
}

impl DashManifest {
    fn parse(xml: &str) -> Result<Self> {
        let mpd: Mpd = quick_xml::de::from_str(xml)?;

        let Some(representation) = mpd
            .periods
            .into_iter()
            .flat_map(|period| period.adaptation_sets)
            .find_map(|set| set.representations.into_iter().next())
        else {
            return Err(TidalError::MissingRepresentation.into());
        };

        let Some(template) = representation.segment_template else {
            return Err(TidalError::MissingSegmentTemplate.into());
        };

        if !template.media.contains("$Number$") {
            return Err(TidalError::UnsupportedTemplate(template.media).into());
        }

        let entries = template.segment_timeline.map(|t| t.entries).unwrap_or_default();
        let mut count: u64 = 0;

        for entry in entries {
            if entry.repeats < 0 {
                return Err(TidalError::LiveManifest.into());
            }

            count += 1 + entry.repeats as u64;
        }

        if count == 0 {
            return Err(TidalError::MissingSegments.into());
        }

        let numbers = template.start_number..template.start_number + count;
        let mut urls: Vec<Arc<str>> = Vec::with_capacity(count as usize + 1);

        urls.push(Arc::from(template.initialization));
        urls.extend(numbers.map(|n| Arc::from(template.media.replace("$Number$", &n.to_string()))));

        Ok(Self { urls: urls.into() })
    }
}

#[derive(Debug)]
struct Segment {
    url: Arc<str>,
    start: u64,
    len: u64,
    skip: u64,
}

impl Segment {
    const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

    async fn probe(client: &Client, url: &str, strip: bool) -> Result<(u64, u64)> {
        let resp = client
            .get(url)
            .header(RANGE, "bytes=0-7")
            .timeout(Self::PROBE_TIMEOUT)
            .send()
            .await?
            .error_for_status()?;

        let total = resp
            .headers()
            .get(CONTENT_RANGE)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.rsplit_once('/'))
            .and_then(|(_, total)| total.trim().parse::<u64>().ok())
            .ok_or_else(|| TidalError::MissingSegmentLength(url.to_string()))?;

        if !strip {
            return Ok((total, 0));
        }

        let skip = Fmp4::payload_at(&resp.bytes().await?)? as u64;

        if skip > total {
            return Err(TidalError::TruncatedSegment(url.to_string()).into());
        }

        Ok((total - skip, skip))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Chunk {
    Head(Bytes),
    Part { url: Arc<str>, skip: u64, take: u64 },
}

#[derive(Debug)]
struct TrackPlan {
    header: Bytes,
    segments: Vec<Segment>,
    total: u64,
    content_type: &'static str,
}

impl TrackPlan {
    const PROBE_CONCURRENCY: usize = 32;

    async fn flac_header(client: &Client, url: &str) -> Result<Bytes> {
        let init = client.get(url).timeout(Segment::PROBE_TIMEOUT).send().await?;

        Fmp4::flac_header(&init.error_for_status()?.bytes().await?)
    }

    async fn build(
        client: &Client,
        manifest: &DashManifest,
        quality: TidalQuality,
    ) -> Result<Self> {
        let rebuild = quality.is_lossless();

        let (head, media) = manifest.urls.split_first().ok_or(TidalError::MissingSegments)?;

        let probed: &[Arc<str>] = if rebuild { media } else { &manifest.urls };

        let header = async {
            if rebuild { Self::flac_header(client, head).await } else { Ok(Bytes::new()) }
        };

        let sizes = futures::stream::iter(probed.to_vec())
            .map(|url| {
                let client = client.clone();

                async move { Segment::probe(&client, &url, rebuild).await }
            })
            .buffered(Self::PROBE_CONCURRENCY)
            .collect::<Vec<Result<(u64, u64)>>>();

        let (header, sizes) = futures::join!(header, sizes);
        let header = header?;

        let mut segments = Vec::with_capacity(probed.len());
        let mut offset = header.len() as u64;

        for (url, probe) in probed.iter().zip(sizes) {
            let (len, skip) = probe?;

            segments.push(Segment { url: url.clone(), start: offset, len, skip });
            offset += len;
        }

        Ok(Self { header, segments, total: offset, content_type: quality.content_type() })
    }

    fn chunks(&self, start: u64, end: u64) -> Vec<Chunk> {
        let head = self.header.len() as u64;

        let leading = (start < head).then(|| {
            let from = start as usize;
            let to = ((end + 1).min(head)) as usize;

            Chunk::Head(self.header.slice(from..to))
        });

        let parts = self
            .segments
            .iter()
            .filter(|seg| seg.len > 0 && seg.start <= end && seg.start + seg.len > start)
            .map(|seg| {
                let from = start.saturating_sub(seg.start);
                let to = (end - seg.start + 1).min(seg.len);

                Chunk::Part { url: seg.url.clone(), skip: seg.skip + from, take: to - from }
            });

        leading.into_iter().chain(parts).collect()
    }
}

#[derive(Debug)]
struct TrackSource {
    manifest: DashManifest,
    plan: OnceCell<Arc<TrackPlan>>,
}

impl TrackSource {
    async fn plan(&self, client: &Client, quality: TidalQuality) -> Result<Arc<TrackPlan>> {
        self.plan
            .get_or_try_init(|| async {
                TrackPlan::build(client, &self.manifest, quality).await.map(Arc::new)
            })
            .await
            .cloned()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ByteRange {
    Full,
    Partial(u64, u64),
    Unsatisfiable,
}

impl ByteRange {
    fn parse(header: Option<&str>, total: u64) -> Self {
        let Some(spec) = header.and_then(|h| h.trim().strip_prefix("bytes=")) else {
            return Self::Full;
        };

        let Some((from, to)) = spec.split_once('-').filter(|_| !spec.contains(',')) else {
            return Self::Full;
        };

        if total == 0 {
            return Self::Unsatisfiable;
        }

        let last = total - 1;
        let (from, to) = (from.trim(), to.trim());

        let bounds = match (from.parse::<u64>(), to.parse::<u64>()) {
            (Ok(start), Ok(end)) => Some((start, end.min(last))),
            (Ok(start), Err(_)) if to.is_empty() => Some((start, last)),
            (Err(_), Ok(suffix)) if from.is_empty() && suffix > 0 => {
                Some((total.saturating_sub(suffix), last))
            }
            (Err(_), Ok(_)) if from.is_empty() => return Self::Unsatisfiable,
            _ => None,
        };

        let Some((start, end)) = bounds else {
            return Self::Full;
        };

        if start > last || start > end {
            return Self::Unsatisfiable;
        }

        Self::Partial(start, end)
    }
}

struct CachedToken {
    value: Arc<str>,
    expires_at: u64,
}

impl CachedToken {
    const RENEW_BEFORE: u64 = 60;

    fn new(value: &str, expires_in: u64) -> Self {
        Self { value: Arc::from(value), expires_at: Self::now() + expires_in }
    }

    fn valid(&self) -> bool {
        self.expires_at > Self::now() + Self::RENEW_BEFORE
    }

    fn now() -> u64 {
        SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TidalResourceType {
    Artist,
    Album,
    Track,
    Playlist,
}

impl TidalResourceType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Artist => "ARTISTS",
            Self::Album => "ALBUMS",
            Self::Track => "TRACKS",
            Self::Playlist => "PLAYLISTS",
        }
    }
}

impl FromStr for TidalResourceType {
    type Err = TidalError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "artist" | "artists" => Ok(Self::Artist),
            "album" | "albums" => Ok(Self::Album),
            "track" | "tracks" => Ok(Self::Track),
            "playlist" | "playlists" => Ok(Self::Playlist),
            _ => Err(TidalError::UnsupportedResourceType),
        }
    }
}

impl Display for TidalResourceType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

pub struct TidalProvider {
    client: Client,
    name: String,
    api_base: String,
    stream_api: String,
    country: String,
    client_id: String,
    client_secret: String,
    quality: Option<TidalQuality>,
    download_quality: TidalQuality,
    token: RwLock<Option<CachedToken>>,
    mint: Mutex<()>,
    sources: Cache<(String, TidalQuality), Arc<TrackSource>>,
}

impl TidalProvider {
    const ALBUM_PAGE_SIZE: u32 = 50;
    const AUTH_BASE: &'static str = "https://auth.tidal.com";
    const DEFAULT_API_BASE: &'static str = "https://api.tidal.com";
    const DEFAULT_CLIENT_ID: &'static str = "txNoH4kkV41MfH25";
    const DEFAULT_CLIENT_SECRET: &'static str = "dQjy0MinCEvxi1O4UmxvxWnDjt4cgHBPw8ll6nYBk98=";
    const DEFAULT_COUNTRY: &'static str = "US";
    const DEFAULT_SEARCH_COUNT: u32 = 20;
    const HI_RES_READAHEAD: usize = 2;
    const ITEM_PAGE_SIZE: u32 = 100;
    const MANIFEST_TIMEOUT: Duration = Duration::from_secs(20);
    const MAX_ALBUMS: usize = 500;
    const MAX_SEARCH_COUNT: u32 = 50;
    const MAX_TRACKS: usize = 1000;
    const PAGE_CONCURRENCY: usize = 4;
    const PROVIDER: &'static str = "tidal";
    /* How many segments we fetch ahead of the one the client is reading, because
     * latency is bad!! */
    const SEGMENT_READAHEAD: usize = 4;
    const SIMILAR_ARTIST_COUNT: u32 = 20;
    const SOURCE_CACHE_CAPACITY: u64 = 512;
    const SOURCE_CACHE_TTL: Duration = Duration::from_secs(1800);

    pub fn new(
        client: Client,
        name: &str,
        cfg: &TidalProviderConfig,
        download_quality: SubsonicStreamFormat,
    ) -> Self {
        let quality = cfg.quality.as_deref().and_then(|raw| {
            let parsed = TidalQuality::parse(raw);

            if parsed.is_none() {
                warn!(
                    "Unknown quality {:?} for TIDAL remote [{}]; use HI_RES, LOSSLESS, HIGH or LOW",
                    raw, name
                );
            }

            parsed
        });

        Self {
            client,
            name: name.to_string(),
            api_base: cfg.url.clone().unwrap_or_else(|| Self::DEFAULT_API_BASE.to_string()),
            stream_api: cfg.stream_api.trim_end_matches('/').to_string(),
            country: cfg.country.as_deref().unwrap_or(Self::DEFAULT_COUNTRY).to_string(),
            client_id: cfg.client_id.as_deref().unwrap_or(Self::DEFAULT_CLIENT_ID).to_string(),
            client_secret: cfg
                .client_secret
                .as_deref()
                .unwrap_or(Self::DEFAULT_CLIENT_SECRET)
                .to_string(),
            quality,
            download_quality: TidalQuality::resolve(download_quality, quality),
            token: RwLock::new(None),
            mint: Mutex::new(()),
            sources: Cache::builder()
                .time_to_live(Self::SOURCE_CACHE_TTL)
                .max_capacity(Self::SOURCE_CACHE_CAPACITY)
                .build(),
        }
    }

    async fn cached_token(&self) -> Option<Arc<str>> {
        self.token.read().await.as_ref().filter(|t| t.valid()).map(|t| t.value.clone())
    }

    async fn token(&self) -> Result<Arc<str>> {
        if let Some(value) = self.cached_token().await {
            return Ok(value);
        }

        let _guard = self.mint.lock().await;

        if let Some(value) = self.cached_token().await {
            return Ok(value);
        }

        let resp = self
            .client
            .post(format!("{}/v1/oauth2/token", Self::AUTH_BASE))
            .basic_auth(&self.client_id, Some(&self.client_secret))
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body("grant_type=client_credentials")
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(TidalError::Token(resp.status()).into());
        }

        let grant = resp.json::<TidalToken>().await?;
        let token = CachedToken::new(&grant.access_token, grant.expires_in);
        let value = token.value.clone();

        debug!("Got a new app token for [{}], valid for {}s", self.name, grant.expires_in);

        *self.token.write().await = Some(token);

        Ok(value)
    }

    async fn get<T: DeserializeOwned>(&self, path: &str, query: &[(&str, String)]) -> Result<T> {
        let token = self.token().await?;

        let resp = self
            .client
            .get(format!("{}{path}", self.api_base))
            .header(AUTHORIZATION, format!("Bearer {token}"))
            .query(&[("countryCode", self.country.as_str())])
            .query(query)
            .send()
            .await?;

        if !resp.status().is_success() {
            debug!(
                "Request to {} on [{}] returned status code: {}",
                path,
                self.name,
                resp.status()
            );
            return Err(RemoteError::Status(resp.status()).into());
        }

        Ok(resp.json::<T>().await?)
    }

    async fn fetch_paged<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
        page: u32,
        max: usize,
    ) -> Result<Vec<T>> {
        let fetch = |offset: u32| {
            let mut params = query.to_vec();
            params.push(("limit", page.to_string()));
            params.push(("offset", offset.to_string()));

            async move { self.get::<TidalList<T>>(path, &params).await }
        };

        let first = fetch(0).await?;
        let wanted = (first.total.unwrap_or_default() as usize).min(max);
        let mut collected = first.items;

        if collected.is_empty() || collected.len() >= wanted {
            collected.truncate(max);

            return Ok(collected);
        }

        let offsets: Vec<u32> = (1u32..)
            .map_while(|n| n.checked_mul(page))
            .take_while(|offset| (*offset as usize) < wanted)
            .collect();

        let pages: Vec<Result<TidalList<T>>> = futures::stream::iter(offsets)
            .map(fetch)
            .buffered(Self::PAGE_CONCURRENCY)
            .collect()
            .await;

        for batch in pages {
            collected.extend(batch?.items);
        }

        collected.truncate(max);

        Ok(collected)
    }

    async fn artist_releases(&self, artist: &str, filter: &str) -> Result<Vec<TidalAlbum>> {
        self.fetch_paged(
            &format!("/v1/artists/{artist}/albums"),
            &[("filter", filter.to_string())],
            Self::ALBUM_PAGE_SIZE,
            Self::MAX_ALBUMS,
        )
        .await
    }

    async fn search_type(
        &self,
        query: &str,
        kind: &[TidalResourceType],
        count: Option<u32>,
        offset: Option<u32>,
    ) -> Result<TidalSearch> {
        let limit = count.unwrap_or(Self::DEFAULT_SEARCH_COUNT);

        if limit == 0 {
            return Ok(TidalSearch::default());
        }

        self.get::<TidalSearch>("/v1/search", &[
            ("query", query.to_string()),
            ("types", kind.iter().map(|k| k.to_string()).collect::<Vec<_>>().join(",")),
            ("limit", limit.clamp(1, Self::MAX_SEARCH_COUNT).to_string()),
            ("offset", offset.unwrap_or_default().to_string()),
        ])
        .await
    }

    fn tagged<T: TagOrigin>(&self, mut value: T, id_type: IdType) -> T {
        value.tag_origin(&self.name, id_type);
        value
    }

    fn map_song(&self, track: TidalTrack) -> Option<Song<'static>> {
        let mut song = Song::try_from(track).ok()?;
        let quality = self.quality_for(None);

        song.suffix = Some(self.tagged_suffix(quality.suffix()).into());
        song.content_type = Some(quality.content_type().into());
        song.bit_rate = Some(quality.bitrate_kbps());
        song.sampling_rate = Some(quality.sampling_rate());
        song.bit_depth = quality.bit_depth();
        song.channel_count = Some(2);
        song.size = song.duration.and_then(|secs| quality.estimated_size(secs));

        Some(self.tagged(song, IdType::Song))
    }

    async fn album_of(&self, song: &Song<'_>) -> Option<AlbumID3<'static>> {
        if song.year.is_some() {
            return None;
        }

        let key = song.album_id.as_ref().map(Id::key).filter(|key| !key.is_empty())?;
        let album = self.get::<TidalAlbum>(&format!("/v1/albums/{key}"), &[]).await.ok()?;

        self.map_albums([album]).into_iter().next()
    }

    fn map_songs<I: IntoIterator<Item = TidalTrack>>(&self, tracks: I) -> Vec<Song<'static>> {
        tracks.into_iter().filter_map(|track| self.map_song(track)).collect()
    }

    fn map_albums<I: IntoIterator<Item = TidalAlbum>>(&self, albums: I) -> Vec<AlbumID3<'static>> {
        albums
            .into_iter()
            .filter_map(|album| AlbumID3::try_from(album).ok())
            .map(|album| self.tagged(album, IdType::Album))
            .collect()
    }

    fn map_artists<I: IntoIterator<Item = TidalArtist>>(
        &self,
        artists: I,
    ) -> Vec<ArtistID3<'static>> {
        artists
            .into_iter()
            .filter_map(|artist| ArtistID3::try_from(artist).ok())
            .map(|artist| self.tagged(artist, IdType::Artist))
            .collect()
    }

    fn quality_for(&self, format: Option<SubsonicStreamFormat>) -> TidalQuality {
        TidalQuality::resolve(format.unwrap_or_default(), self.quality)
    }

    async fn fetch_manifest(&self, track_id: &str, quality: TidalQuality) -> Result<DashManifest> {
        let resp = self
            .client
            .get(format!("{}/manifests", self.stream_api))
            .timeout(Self::MANIFEST_TIMEOUT)
            .query(&[("id", track_id), ("quality", quality.as_str())])
            .send()
            .await?;

        if !resp.status().is_success() {
            debug!(
                "Manifest request for track {} returned status code: {}",
                track_id,
                resp.status()
            );
            return Err(RemoteError::Status(resp.status()).into());
        }

        let attributes = resp.json::<TidalManifestResponse>().await?.data.attributes;
        let presentation = attributes.track_presentation.as_deref().unwrap_or("missing");

        if presentation != "FULL" {
            return Err(RemoteError::Unsupported(Self::PROVIDER).into());
        }

        let Some(uri) = attributes.uri.filter(|u| !u.is_empty()) else {
            return Err(TidalError::MissingManifestUri(track_id.to_string()).into());
        };

        let resp = self.client.get(&uri).timeout(Self::MANIFEST_TIMEOUT).send().await?;

        if !resp.status().is_success() {
            debug!("Manifest fetch for track {} returned status code: {}", track_id, resp.status());
            return Err(RemoteError::Status(resp.status()).into());
        }

        DashManifest::parse(&resp.text().await?)
    }

    async fn track_source(
        &self,
        track_id: &str,
        quality: TidalQuality,
    ) -> Result<Arc<TrackSource>> {
        if track_id.is_empty() || !track_id.chars().all(|c| c.is_ascii_digit()) {
            return Err(RemoteError::EmptyDataset.into());
        }

        self.sources
            .try_get_with((track_id.to_string(), quality), async {
                let manifest = self.fetch_manifest(track_id, quality).await?;

                debug!(
                    "Resolved track {} at {} into {} segments",
                    track_id,
                    quality.as_str(),
                    manifest.urls.len()
                );

                Ok::<_, anyhow::Error>(Arc::new(TrackSource { manifest, plan: OnceCell::new() }))
            })
            .await
            .map_err(|err| TidalError::Source(err).into())
    }

    fn download_segments(manifest: &DashManifest, quality: TidalQuality) -> DownloadSource {
        DownloadSource::Segments {
            urls: Arc::clone(&manifest.urls),
            content_type: quality.content_type().to_string(),
            layout: if quality.is_lossless() {
                SegmentLayout::FlacInMp4
            } else {
                SegmentLayout::Contiguous
            },
        }
    }

    async fn download_source_for(
        &self,
        track_id: &str,
        fallback: TidalQuality,
    ) -> Option<DownloadSource> {
        let wanted = self.download_quality;

        match self.track_source(track_id, wanted).await {
            Ok(source) => return Some(Self::download_segments(&source.manifest, wanted)),
            Err(_) if wanted == fallback => return None,
            Err(_) => debug!(
                "Falling back to {} for track {} because {} is unavailable",
                fallback.as_str(),
                track_id,
                wanted.as_str()
            ),
        }

        let source = self.track_source(track_id, fallback).await.ok()?;

        Some(Self::download_segments(&source.manifest, fallback))
    }

    fn stream_body(&self, chunks: Vec<Chunk>, quality: TidalQuality) -> Body {
        let client = self.client.clone();

        let readahead = if quality == TidalQuality::HiRes {
            Self::HI_RES_READAHEAD
        } else {
            Self::SEGMENT_READAHEAD
        };

        let stream = futures::stream::iter(chunks)
            .map(move |chunk| {
                let client = client.clone();

                async move {
                    match chunk {
                        Chunk::Head(head) => Ok::<_, reqwest::Error>(head),
                        Chunk::Part { url, skip, take } => {
                            let resp = client.get(&*url).send().await?.error_for_status()?;
                            let bytes = resp.bytes().await?;
                            let from = (skip as usize).min(bytes.len());
                            let to = from.saturating_add(take as usize).min(bytes.len());

                            Ok(bytes.slice(from..to))
                        }
                    }
                }
            })
            .buffered(readahead);

        Body::wrap_stream(stream)
    }

    fn range_not_satisfiable(total: u64) -> Result<Response> {
        let response = axum::http::Response::builder()
            .status(StatusCode::RANGE_NOT_SATISFIABLE)
            .header(ACCEPT_RANGES, "bytes")
            .header(CONTENT_LENGTH, "0")
            .header(CONTENT_RANGE, format!("bytes */{total}"))
            .body(Body::from(Vec::new()))?;

        Ok(Response::from(response))
    }
}

#[async_trait]
impl MusicProvider for TidalProvider {
    fn id(&self) -> &str {
        &self.name
    }

    async fn get_artist<'a>(&self, id: &Id<'_>) -> Result<ArtistWithAlbumsID3<'a>> {
        let key = id.key();
        let path = format!("/v1/artists/{key}");

        let (artist, albums, singles) = futures::try_join!(
            self.get::<TidalArtist>(&path, &[]),
            // TODO: Check if we can merge in one call
            self.artist_releases(key, "ALBUMS"),
            self.artist_releases(key, "EPSANDSINGLES"),
        )?;

        let mut seen_ids = HashSet::new();
        let mut seen_titles = HashSet::new();

        let unique: Vec<TidalAlbum> = albums
            .into_iter()
            .chain(singles)
            .filter(|album| seen_ids.insert(album.id) && seen_titles.insert(album.title_key()))
            .take(Self::MAX_ALBUMS)
            .collect();

        let mut album = self.map_albums(unique);

        album.sort_by(|a, b| b.created.cmp(&a.created).then_with(|| a.name.cmp(&b.name)));

        let base = ArtistID3 {
            album_count: Some(album.len() as u32),
            ..self.map_artists([artist]).into_iter().next().ok_or(RemoteError::EmptyDataset)?
        };

        Ok(ArtistWithAlbumsID3 { base, album: Some(album) })
    }

    async fn get_artist_info<'a>(&self, id: &Id<'_>) -> Result<ArtistInfo<'a>> {
        let key = id.key();
        let artist = self.get::<TidalArtist>(&format!("/v1/artists/{key}"), &[]).await?;

        let bio_path = format!("/v1/artists/{key}/bio");
        let similar_path = format!("/v1/artists/{key}/similar");
        let similar_query = [("limit", Self::SIMILAR_ARTIST_COUNT.to_string())];

        let (bio, similar) = futures::join!(
            self.get::<TidalEditorial>(&bio_path, &[]),
            self.get::<TidalList<TidalArtist>>(&similar_path, &similar_query),
        );

        let picture = artist.picture.filter(|p| !p.is_empty());
        let image = |size: u32| picture.as_deref().map(|uuid| TidalImage::url(uuid, size).into());

        Ok(ArtistInfo {
            biography: bio.ok().and_then(|bio| bio.plain_text()).map(Into::into),
            music_brainz_id: None,
            last_fm_url: None,
            small_image_url: image(160),
            medium_image_url: image(320),
            large_image_url: image(750),
            similar_artist: similar.ok().map(|list| self.map_artists(list.items)),
        })
    }

    async fn get_album_info<'a>(&self, id: &Id<'_>) -> Result<AlbumInfo<'a>> {
        let key = id.key();

        let album = self.get::<TidalAlbum>(&format!("/v1/albums/{key}"), &[]).await?;
        let review = self.get::<TidalEditorial>(&format!("/v1/albums/{key}/review"), &[]).await;

        let mut info = AlbumInfo::try_from(album).map_err(|_| RemoteError::ProcessingError)?;

        if let Some(notes) = review.ok().and_then(|review| review.plain_text()) {
            info.notes = Some(notes.into());
        }

        Ok(info)
    }

    async fn get_album<'a>(&self, id: &Id<'_>) -> Result<AlbumWithSongsID3<'a>> {
        let key = id.key();
        let album_path = format!("/v1/albums/{key}");
        let items_path = format!("/v1/albums/{key}/items");

        let (album, items) = futures::try_join!(
            self.get::<TidalAlbum>(&album_path, &[]),
            self.fetch_paged::<TidalAlbumItem>(
                &items_path,
                &[],
                Self::ITEM_PAGE_SIZE,
                Self::MAX_TRACKS
            ),
        )?;

        let base = self.map_albums([album]).into_iter().next().ok_or(RemoteError::EmptyDataset)?;

        let tracks = items
            .into_iter()
            .filter(|entry| entry.kind.eq_ignore_ascii_case("track"))
            .map(|entry| entry.item);

        let mut album = AlbumWithSongsID3 { base, song: self.map_songs(tracks) };
        album.song.inherit(&album.base);

        Ok(album)
    }

    async fn get_song<'a>(&self, id: &Id<'_>) -> Result<Song<'a>> {
        let track = self.get::<TidalTrack>(&format!("/v1/tracks/{}", id.key()), &[]).await?;
        let mut song = self.map_song(track).ok_or(RemoteError::EmptyDataset)?;

        if let Some(album) = self.album_of(&song).await {
            song.inherit(&album);
        }

        Ok(song)
    }

    async fn get_top_songs<'a>(&self, artist: &str, count: Option<u32>) -> Result<TopSongs<'a>> {
        let found = self.search_type(artist, &[TidalResourceType::Artist], Some(1), None).await?;

        let Some(found) = found.artists.items.into_iter().next() else {
            return Err(RemoteError::EmptyDataset.into());
        };

        let tracks = self
            .fetch_paged::<TidalTrack>(
                &format!("/v1/artists/{}/toptracks", found.id),
                &[],
                Self::ALBUM_PAGE_SIZE,
                count.unwrap_or(Self::DEFAULT_SEARCH_COUNT) as usize,
            )
            .await?;

        Ok(TopSongs { song: Some(self.map_songs(tracks)) })
    }

    async fn search<'a>(&self, query: &SearchRequest<'a>) -> Result<SearchResult<'a>> {
        let types: Vec<TidalResourceType> = [
            (query.album_count, TidalResourceType::Album),
            (query.song_count, TidalResourceType::Track),
            (query.artist_count, TidalResourceType::Artist),
        ]
        .into_iter()
        .filter_map(|(count, resource_type)| count.is_some_and(|c| c > 0).then_some(resource_type))
        .collect();

        if types.is_empty() {
            return Ok(SearchResult::default());
        }

        let limit = [query.album_count, query.song_count, query.artist_count]
            .into_iter()
            .flatten()
            .max()
            .unwrap_or(Self::DEFAULT_SEARCH_COUNT);

        let Ok(search_res) = self.search_type(&query.query, &types, Some(limit), None).await else {
            debug!("Search for {} failed on [{}]", query.query, self.name);
            return Err(RemoteError::EmptyDataset.into());
        };

        let artists = search_res.artists;
        let albums = search_res.albums;
        let tracks = search_res.tracks;

        Ok(SearchResult {
            artist: self.map_artists(artists.items),
            album: self.map_albums(albums.items),
            song: self.map_songs(tracks.items),
        })
    }

    async fn download_source(&self, id: &Id<'_>) -> Option<DownloadSource> {
        self.download_source_for(id.key(), self.quality_for(None)).await
    }

    async fn get_cover_art(&self, id: &Id<'_>, size: Option<u32>) -> Result<Response> {
        let uuid = id.key();

        if uuid.is_empty() {
            return Err(RemoteError::EmptyDataset.into());
        }

        let mut last = StatusCode::NOT_FOUND;

        for candidate in TidalImage::sizes_for(size.unwrap_or(640)) {
            let resp = self.client.get(TidalImage::url(uuid, candidate)).send().await?;

            if resp.status().is_success() {
                return Ok(resp);
            }

            last = resp.status();
        }

        Err(RemoteError::Status(last).into())
    }

    async fn stream(&self, req: &StreamMedia) -> Result<Response> {
        let track_id = req.request.id.key();
        let quality = self.quality_for(req.request.format);
        let source = self.track_source(track_id, quality).await?;
        let plan = source.plan(&self.client, quality).await?;
        let total = plan.total;

        if total == 0 {
            return Err(RemoteError::EmptyDataset.into());
        }

        let (status, start, end) = match ByteRange::parse(req.range_header.as_deref(), total) {
            ByteRange::Full => (StatusCode::OK, 0, total - 1),
            ByteRange::Partial(start, end) => (StatusCode::PARTIAL_CONTENT, start, end),
            ByteRange::Unsatisfiable => return Self::range_not_satisfiable(total),
        };

        let chunks = plan.chunks(start, end);

        debug!(
            "Streaming track {} at {} as {}, bytes {}-{}/{} in {} requests",
            track_id,
            quality.as_str(),
            plan.content_type,
            start,
            end,
            total,
            chunks.len()
        );

        let mut builder = axum::http::Response::builder()
            .status(status)
            .header(CONTENT_TYPE, plan.content_type)
            .header(CONTENT_LENGTH, (end - start + 1).to_string())
            .header(ACCEPT_RANGES, "bytes");

        if status == StatusCode::PARTIAL_CONTENT {
            builder = builder.header(CONTENT_RANGE, format!("bytes {start}-{end}/{total}"));
        }

        if let Some(cached) = self.download_source_for(track_id, quality).await {
            builder = builder.extension(cached);
        }

        Ok(Response::from(builder.body(self.stream_body(chunks, quality))?))
    }
}
