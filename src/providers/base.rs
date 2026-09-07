use std::borrow::Cow;

use anyhow::Result;
use async_trait::async_trait;
use futures::StreamExt;
use futures::stream::FuturesUnordered;
use serde::{Deserialize, Serialize};
use thiserror::Error as TError;

use crate::models::{
    AlbumID3,
    AlbumInfo,
    AlbumWithSongsID3,
    ArtistInfo,
    ArtistWithAlbumsID3,
    Id,
    IdType,
    Mergeable,
    SearchResult,
    Song,
    SubsonicStreamFormat,
    TopSongs,
    empty_as_none,
};
use crate::services::StreamMedia;
use crate::utils::metadata::{compare_artists, compare_titles};
use crate::utils::string::normalize;
use crate::workers::download::DownloadSource;

const ALBUM_LOOKUP_LIMIT: usize = 10;
const ARTIST_LOOKUP_LIMIT: usize = 5;

#[derive(TError, Debug)]
pub enum ProviderError {
    #[error("Missing id for {}", .0.as_str())]
    MissingId(IdType),
    #[error("Missing name for {}", .0.as_str())]
    MissingName(IdType),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ArtistRef {
    pub name: String,
    pub albums: Vec<String>,
}

impl ArtistRef {
    pub fn of(artist: &ArtistWithAlbumsID3<'_>) -> Self {
        Self {
            name: artist.base.name.to_string(),
            albums: artist.album.iter().flatten().map(|album| album.name.to_string()).collect(),
        }
    }

    /// How many of these albums the candidate also has.
    fn overlap(&self, candidate: &ArtistWithAlbumsID3<'_>) -> usize {
        let Some(theirs) = candidate.album.as_ref() else {
            return 0;
        };

        self.albums
            .iter()
            .filter(|mine| theirs.iter().any(|other| compare_titles(mine, &other.name)))
            .count()
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchRequest<'a> {
    #[serde(default)]
    pub query: Cow<'a, str>,
    #[serde(default, deserialize_with = "empty_as_none")]
    pub artist_count: Option<u32>,
    #[serde(default, deserialize_with = "empty_as_none")]
    pub artist_offset: Option<u32>,
    #[serde(default, deserialize_with = "empty_as_none")]
    pub album_count: Option<u32>,
    #[serde(default, deserialize_with = "empty_as_none")]
    pub album_offset: Option<u32>,
    #[serde(default, deserialize_with = "empty_as_none")]
    pub song_count: Option<u32>,
    #[serde(default, deserialize_with = "empty_as_none")]
    pub song_offset: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct StreamRequest<'a> {
    pub id: Id<'a>,
    #[serde(default, deserialize_with = "empty_as_none")]
    pub max_bit_rate: Option<u32>,
    pub format: Option<SubsonicStreamFormat>,
    #[serde(default, deserialize_with = "empty_as_none")]
    pub time_offset: Option<u32>,
    pub size: Option<String>,
    #[serde(default, deserialize_with = "empty_as_none")]
    pub estimate_content_length: Option<bool>,
    #[serde(default, deserialize_with = "empty_as_none")]
    pub converted: Option<bool>,
}

#[async_trait]
pub trait MusicProvider: Send + Sync {
    fn id(&self) -> &str;

    fn is_own_id(&self, id: &Id<'_>) -> bool {
        id.service().is_some_and(|service| service == self.id())
    }

    async fn get_artist<'a>(&self, id: &Id<'_>) -> Result<ArtistWithAlbumsID3<'a>>;
    async fn get_artist_info<'a>(&self, id: &Id<'_>) -> Result<ArtistInfo<'a>>;
    async fn get_album_info<'a>(&self, id: &Id<'_>) -> Result<AlbumInfo<'a>>;
    async fn get_album<'a>(&self, id: &Id<'_>) -> Result<AlbumWithSongsID3<'a>>;
    async fn get_song<'a>(&self, id: &Id<'_>) -> Result<Song<'a>>;
    async fn get_top_songs<'a>(&self, artist: &str, count: Option<u32>) -> Result<TopSongs<'a>>;

    async fn search<'a>(&self, query: &SearchRequest<'a>) -> Result<SearchResult<'a>>;
    async fn get_cover_art(&self, id: &Id<'_>, size: Option<u32>) -> Result<reqwest::Response>;

    async fn stream(&self, req: &StreamMedia) -> Result<reqwest::Response>;

    /* Helpers that are not mapped to subsonic, but we need it for making our life easier */

    async fn download_source(&self, id: &Id<'_>) -> Option<DownloadSource> {
        let probe = StreamMedia {
            request: StreamRequest { id: id.clone(), ..Default::default() },
            range_header: Some("bytes=0-9".to_string()),
        };

        DownloadSource::from_response(&self.stream(&probe).await.ok()?)
    }

    /* Find an artist matching against a reference */
    async fn find_artist<'a>(&self, reference: &ArtistRef) -> Option<ArtistWithAlbumsID3<'a>> {
        if reference.name.trim().is_empty() {
            return None;
        }

        let search = SearchRequest {
            query: Cow::Borrowed(&reference.name),
            artist_count: Some(ARTIST_LOOKUP_LIMIT as u32),
            artist_offset: None,
            album_count: Some(0),
            album_offset: None,
            song_count: Some(0),
            song_offset: None,
        };

        let found = self.search(&search).await.ok()?;
        let wanted = normalize(reference.name.trim());

        let mut lookups: FuturesUnordered<_> = found
            .artist
            .iter()
            .filter(|candidate| {
                if reference.albums.is_empty() {
                    normalize(candidate.name.trim()) == wanted
                } else {
                    compare_artists(Some(&candidate.name), Some(&reference.name))
                }
            })
            .take(ARTIST_LOOKUP_LIMIT)
            .map(|candidate| self.get_artist(&candidate.id))
            .collect();

        let mut best: Option<(usize, ArtistWithAlbumsID3<'a>)> = None;

        while let Some(result) = lookups.next().await {
            let Ok(candidate) = result else {
                continue;
            };

            let shared = reference.overlap(&candidate);

            if shared == 0 && !reference.albums.is_empty() {
                continue;
            }

            if best.as_ref().is_none_or(|(most, _)| shared > *most) {
                best = Some((shared, candidate));
            }
        }

        best.map(|(_, artist)| artist)
    }

    async fn get_albums_by_name<'a>(&self, name: &str) -> Vec<AlbumWithSongsID3<'a>> {
        let search_req = SearchRequest {
            query: Cow::Borrowed(name),
            artist_count: Some(0),
            artist_offset: None,
            album_count: Some(ALBUM_LOOKUP_LIMIT as u32),
            album_offset: None,
            song_count: Some(0),
            song_offset: None,
        };

        let Ok(search_result) = self.search(&search_req).await else {
            return vec![];
        };

        let mut lookups: FuturesUnordered<_> = search_result
            .album
            .iter()
            .take(ALBUM_LOOKUP_LIMIT)
            .map(|album| self.get_album(&album.id))
            .collect();

        let mut albums = Vec::with_capacity(lookups.len());

        while let Some(result) = lookups.next().await {
            if let Ok(album) = result {
                albums.push(album);
            }
        }

        albums
    }

    async fn get_matching_remote_album(
        &self,
        album: &AlbumID3<'_>,
    ) -> Option<AlbumWithSongsID3<'_>> {
        let albums = self.get_albums_by_name(&album.name).await;

        for remote_album in albums {
            if album.matches(&remote_album.base) {
                return Some(remote_album);
            }
        }

        None
    }
}
