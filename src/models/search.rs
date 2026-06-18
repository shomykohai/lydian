use serde::{Deserialize, Serialize};

use crate::models::{AlbumID3, ArtistID3, IdType, Mergeable, Song, SubsonicEntity, TagOrigin};

#[derive(Debug, Deserialize, Serialize)]
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
