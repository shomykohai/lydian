use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

use super::Song;
use crate::models::{
    ArtistID3,
    CowStr,
    FuzzyMatch,
    IdType,
    ItemDate,
    ItemGenre,
    Mergeable,
    SubsonicEntity,
    TagOrigin,
};
use crate::utils;

#[derive(Debug, Deserialize, Serialize)]
pub struct RecordLabel<'a> {
    pub name: CowStr<'a>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscTitle<'a> {
    pub disc: u32,
    pub title: CowStr<'a>,
    pub cover_art: Option<CowStr<'a>>,
}

#[skip_serializing_none]
#[derive(Debug, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct AlbumID3<'a> {
    pub id: CowStr<'a>,
    pub name: CowStr<'a>,
    pub version: Option<CowStr<'a>>,
    pub artist: Option<CowStr<'a>>,
    pub artist_id: Option<CowStr<'a>>,
    pub cover_art: Option<CowStr<'a>>,
    pub song_count: u32,
    pub duration: u32,
    pub play_count: Option<u64>,
    pub created: DateTime<Utc>,
    //#[serde(skip_serializing_if = "Option::is_none")]
    pub starred: Option<DateTime<Utc>>,
    pub year: Option<u32>,
    pub genre: Option<CowStr<'a>>,
    pub played: Option<DateTime<Utc>>,
    pub user_rating: Option<u8>,
    pub record_labels: Option<Vec<RecordLabel<'a>>>,
    pub music_brainz_id: Option<CowStr<'a>>,
    pub genres: Option<Vec<ItemGenre<'a>>>,
    pub artists: Option<Vec<ArtistID3<'a>>>,
    pub display_artist: Option<CowStr<'a>>,
    pub release_types: Option<Vec<CowStr<'a>>>,
    pub moods: Option<Vec<CowStr<'a>>>,
    pub sort_name: Option<CowStr<'a>>,
    pub original_release_date: Option<ItemDate>,
    pub release_date: Option<ItemDate>,
    pub is_compilation: Option<bool>,
    pub explicit_status: Option<CowStr<'a>>,
    pub disc_titles: Option<Vec<DiscTitle<'a>>>,
}

impl FuzzyMatch for AlbumID3<'_> {
    fn is_similar_to(&self, other: &Self) -> bool {
        utils::string::is_similar(&self.name, &other.name)
            && utils::string::is_opt_similar(self.artist.as_deref(), other.artist.as_deref())
            && self.song_count == other.song_count
            && self.duration.abs_diff(other.duration) <= 5
    }
}

impl Mergeable for AlbumID3<'_> {
    type Item = Self;

    fn merge(&mut self, other: Self) {
        if self.version.is_none() {
            self.version = other.version;
        }
        if self.artist.is_none() {
            self.artist = other.artist;
        }
        if self.artist_id.is_none() {
            self.artist_id = other.artist_id;
        }
        if self.cover_art.is_none() {
            self.cover_art = other.cover_art;
        }
        if self.year.is_none() {
            self.year = other.year;
        }
        if self.genre.is_none() {
            self.genre = other.genre;
        }
        if self.record_labels.is_none() {
            self.record_labels = other.record_labels;
        }
        if self.music_brainz_id.is_none() {
            self.music_brainz_id = other.music_brainz_id;
        }
        if self.genres.is_none() {
            self.genres = other.genres;
        }
        if self.artists.is_none() {
            self.artists = other.artists;
        }
        if self.display_artist.is_none() {
            self.display_artist = other.display_artist;
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct AlbumWithSongsID3<'a> {
    #[serde(flatten)]
    pub base: AlbumID3<'a>,
    pub song: Vec<Song<'a>>,
}

impl FuzzyMatch for AlbumWithSongsID3<'_> {
    fn is_similar_to(&self, other: &Self) -> bool {
        self.base.is_similar_to(&other.base)
    }
}

impl Mergeable for AlbumWithSongsID3<'_> {
    type Item = Self;

    fn merge(&mut self, other: Self) {
        self.base.merge(other.base);
        self.song.merge(other.song);
    }
}

impl SubsonicEntity for AlbumWithSongsID3<'_> {
    const FIELD_NAME: &'static str = "album";
}

impl<'a> TagOrigin for DiscTitle<'a> {
    fn tag_origin(&mut self, origin: &str, _id_type: IdType) {
        self.cover_art.tag_origin(origin, IdType::CoverArt);
    }
}

impl<'a> TagOrigin for AlbumID3<'a> {
    fn tag_origin(&mut self, origin: &str, id_type: IdType) {
        self.id.tag_origin(origin, id_type);
        self.artist_id.tag_origin(origin, IdType::Artist);
        self.cover_art.tag_origin(origin, IdType::CoverArt);
        self.disc_titles.tag_origin(origin, IdType::Generic);
        self.artists.tag_origin(origin, IdType::Artist);
    }
}

impl<'a> TagOrigin for AlbumWithSongsID3<'a> {
    fn tag_origin(&mut self, origin: &str, id_type: IdType) {
        self.base.tag_origin(origin, id_type);
        self.song.tag_origin(origin, IdType::Song);
    }
}

#[skip_serializing_none]
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlbumInfo<'a> {
    pub notes: Option<CowStr<'a>>,
    pub music_brainz_id: Option<CowStr<'a>>,
    pub last_fm_url: Option<CowStr<'a>>,
    pub small_image_url: Option<CowStr<'a>>,
    pub medium_image_url: Option<CowStr<'a>>,
    pub large_image_url: Option<CowStr<'a>>,
}

impl SubsonicEntity for AlbumInfo<'_> {
    const FIELD_NAME: &'static str = "albumInfo";
}

impl Mergeable for AlbumInfo<'_> {
    type Item = Self;

    fn merge(&mut self, other: Self) {
        if self.notes.is_none() {
            self.notes = other.notes;
        }

        if self.music_brainz_id.is_none() {
            self.music_brainz_id = other.music_brainz_id;
        }

        if self.last_fm_url.is_none() {
            self.last_fm_url = other.last_fm_url;
        }

        if self.small_image_url.is_none() {
            self.small_image_url = other.small_image_url;
        }

        if self.medium_image_url.is_none() {
            self.medium_image_url = other.medium_image_url;
        }

        if self.large_image_url.is_none() {
            self.large_image_url = other.large_image_url;
        }
    }
}
