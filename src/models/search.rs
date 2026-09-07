use serde::{Deserialize, Serialize};

use crate::models::{
    AlbumID3,
    ArtistID3,
    IdType,
    Limit,
    Mergeable,
    SEARCH_LIMIT,
    Song,
    SubsonicEntity,
    TagOrigin,
};
use crate::providers::SearchRequest;
use crate::utils::relevance::order_by_relevance;

pub trait Searchable {
    fn label(&self) -> &str;
    fn artist(&self) -> Option<&str>;
    fn sort_key(&self) -> &str;
}

impl Searchable for ArtistID3<'_> {
    fn label(&self) -> &str {
        &self.name
    }

    fn artist(&self) -> Option<&str> {
        None
    }

    fn sort_key(&self) -> &str {
        self.sort_name.as_deref().unwrap_or(&self.name)
    }
}

impl Searchable for AlbumID3<'_> {
    fn label(&self) -> &str {
        &self.name
    }

    fn artist(&self) -> Option<&str> {
        self.artist.as_deref()
    }

    fn sort_key(&self) -> &str {
        self.sort_name.as_deref().unwrap_or(&self.name)
    }
}

impl Searchable for Song<'_> {
    fn label(&self) -> &str {
        &self.title
    }

    fn artist(&self) -> Option<&str> {
        self.artist.as_deref()
    }

    fn sort_key(&self) -> &str {
        self.sort_name.as_deref().unwrap_or(&self.title)
    }
}

#[derive(Default, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult<'a> {
    #[serde(default)]
    pub artist: Vec<ArtistID3<'a>>,
    #[serde(default)]
    pub album: Vec<AlbumID3<'a>>,
    #[serde(default)]
    pub song: Vec<Song<'a>>,
}

impl SubsonicEntity for SearchResult<'_> {
    const FIELD_NAME: &'static str = "searchResult3";
}

impl<'a> TagOrigin for SearchResult<'a> {
    fn tag_origin(&mut self, origin: &str, _id_type: IdType) {
        self.artist.tag_origin(origin, IdType::Artist);
        self.album.tag_origin(origin, IdType::Album);
        self.song.tag_origin(origin, IdType::Song);
    }
}

impl Mergeable for SearchResult<'_> {
    type Item = Self;

    fn merge(&mut self, other: Self) {
        self.artist.merge(other.artist);
        self.album.merge(other.album);
        self.song.merge(other.song);
    }
}

impl SearchResult<'_> {
    pub fn order_by(&mut self, query: &str, limits: &SearchRequest<'_>) {
        order_by_relevance(&mut self.artist, query);
        order_by_relevance(&mut self.album, query);
        order_by_relevance(&mut self.song, query);

        self.artist.limit(limits.artist_count.unwrap_or(SEARCH_LIMIT));
        self.album.limit(limits.album_count.unwrap_or(SEARCH_LIMIT));
        self.song.limit(limits.song_count.unwrap_or(SEARCH_LIMIT));
    }
}
