use std::borrow::Cow;

use anyhow::Result;
use async_trait::async_trait;
//use axum::body::Bytes;
//use futures::stream::BoxStream;
use serde::{Deserialize, Serialize};

use crate::models::{
    AlbumID3,
    AlbumInfo,
    AlbumWithSongsID3,
    ArtistInfo,
    ArtistWithAlbumsID3,
    FuzzyMatch,
    IdType,
    ORIGIN_SEP,
    SearchResult,
    Song,
    TopSongs,
};
use crate::services::StreamMedia;

#[derive(Debug, Serialize, Deserialize)]
pub struct SearchRequest<'a> {
    pub query: Cow<'a, str>,
    pub artist_count: Option<u32>,
    pub artist_offset: Option<u32>,
    pub album_count: Option<u32>,
    pub album_offset: Option<u32>,
    pub song_count: Option<u32>,
    pub song_offset: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct StreamRequest {
    pub id: String,
    pub max_bit_rate: Option<u32>,
    pub format: Option<String>,
    pub time_offset: Option<u32>,
    pub size: Option<String>,
    pub estimate_content_length: Option<bool>,
    pub converted: Option<bool>,
}

#[async_trait]
pub trait MusicProvider: Send + Sync {
    fn id(&self) -> &str;

    /* Removes the self.id___type__ prefix */
    fn get_raw_id<'a>(&self, id: &'a str) -> &'a str {
        id.rmatch_indices(ORIGIN_SEP)
            .next()
            .map(|(i, _)| i)
            .map_or(id, |idx| &id[idx + ORIGIN_SEP.len()..])
    }

    fn to_id(&self, id: &str, id_type: IdType) -> String {
        format!("{}{}{}{}{}", self.id(), ORIGIN_SEP, id_type.as_str(), ORIGIN_SEP, id)
    }

    fn is_own_id(&self, id: &str) -> bool {
        id.strip_prefix(self.id()).and_then(|rem| rem.strip_prefix(ORIGIN_SEP)).is_some()
    }

    fn get_id_type(&self, id: &str) -> Option<IdType> {
        let remainder = id.strip_prefix(self.id())?.strip_prefix(ORIGIN_SEP)?;

        let type_part = remainder.split(ORIGIN_SEP).next()?;
        IdType::from_str(type_part)
    }

    async fn get_artist<'a>(&self, id: &str) -> Result<ArtistWithAlbumsID3<'a>>;
    async fn get_artist_info<'a>(&self, id: &str) -> Result<ArtistInfo<'a>>;
    async fn get_album_info<'a>(&self, id: &str) -> Result<AlbumInfo<'a>>;
    async fn get_album<'a>(&self, id: &str) -> Result<AlbumWithSongsID3<'a>>;
    async fn get_song<'a>(&self, id: &str) -> Result<Song<'a>>;
    async fn get_top_songs<'a>(&self, artist: &str, count: Option<u32>) -> Result<TopSongs<'a>>;

    async fn search<'a>(&self, query: &SearchRequest<'a>) -> Result<SearchResult<'a>>;
    async fn get_cover_art(&self, id: &str, size: Option<u32>) -> Result<reqwest::Response>;

    async fn stream(&self, req: &StreamMedia) -> Result<reqwest::Response>;

    /* Helpers that are not mapped to subsonic, but we need it for making our life easier */

    async fn get_albums_by_name<'a>(&self, name: &str) -> Vec<AlbumWithSongsID3<'a>> {
        let search_req = SearchRequest {
            query: Cow::Borrowed(name),
            artist_count: Some(0),
            artist_offset: None,
            album_count: Some(10),
            album_offset: None,
            song_count: Some(0),
            song_offset: None,
        };

        let Ok(search_result) = self.search(&search_req).await else {
            return vec![];
        };

        let mut albums = Vec::with_capacity(10);

        for album in &search_result.album {
            if let Ok(album) = self.get_album(&album.id.to_string()).await {
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
            if album.is_similar_to(&remote_album.base) {
                return Some(remote_album);
            }
        }

        None
    }
}
