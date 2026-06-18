use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

use crate::models::{
    ArtistID3,
    CowStr,
    FuzzyMatch,
    IdType,
    ItemGenre,
    Mergeable,
    SubsonicEntity,
    TagOrigin,
};
use crate::utils;

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Contributor<'a> {
    pub role: CowStr<'a>,
    pub sub_role: Option<CowStr<'a>>,
    pub artist: ArtistID3<'a>,
}

#[skip_serializing_none]
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Song<'a> {
    pub id: CowStr<'a>,
    pub parent: Option<CowStr<'a>>,
    pub is_dir: bool,
    pub title: CowStr<'a>,
    pub album: Option<CowStr<'a>>,
    pub artist: Option<CowStr<'a>>,
    pub track: Option<u32>,
    pub year: Option<u32>,
    pub genre: Option<CowStr<'a>>,
    pub cover_art: Option<CowStr<'a>>,
    pub size: Option<u64>,
    pub content_type: Option<CowStr<'a>>,
    pub suffix: Option<CowStr<'a>>,
    pub transcoded_content_type: Option<CowStr<'a>>,
    pub transcoded_suffix: Option<CowStr<'a>>,
    pub duration: Option<u32>,
    pub bit_rate: Option<u32>,
    pub bit_depth: Option<u32>,
    pub sampling_rate: Option<u32>,
    pub channel_count: Option<u32>,
    pub path: Option<CowStr<'a>>,
    pub is_video: Option<bool>,
    pub user_rating: Option<u32>,
    pub average_rating: Option<f32>,
    pub play_count: Option<u64>,
    pub disc_number: Option<u32>,
    pub created: Option<CowStr<'a>>,
    pub starred: Option<CowStr<'a>>,
    pub album_id: Option<CowStr<'a>>,
    pub artist_id: Option<CowStr<'a>>,
    #[serde(rename = "type")]
    pub ty: Option<CowStr<'a>>,
    pub media_type: Option<CowStr<'a>>,
    pub bookmark_position: Option<u64>,
    pub original_width: Option<u32>,
    pub original_height: Option<u32>,
    pub played: Option<CowStr<'a>>,
    pub bpm: Option<u32>,
    pub comment: Option<CowStr<'a>>,
    pub sort_name: Option<CowStr<'a>>,
    pub music_brainz_id: Option<CowStr<'a>>,
    pub isrc: Option<Vec<CowStr<'a>>>,
    pub genres: Option<Vec<ItemGenre<'a>>>,
    pub artists: Option<Vec<ArtistID3<'a>>>,
    pub display_artist: Option<CowStr<'a>>,
    pub album_artists: Option<Vec<ArtistID3<'a>>>,
    pub display_album_artist: Option<CowStr<'a>>,
    pub contributors: Option<Vec<Contributor<'a>>>,
    pub display_composer: Option<CowStr<'a>>,
    pub moods: Option<Vec<CowStr<'a>>>,
    pub replay_gain: Option<ReplayGain>,
    pub explicit_status: Option<CowStr<'a>>,
    pub works: Option<Vec<Work<'a>>>,
    pub movements: Option<Vec<Movement<'a>>>,
    pub groupings: Option<Vec<CowStr<'a>>>,
}

impl SubsonicEntity for Song<'_> {
    const FIELD_NAME: &'static str = "song";
}

impl Mergeable for Song<'_> {
    type Item = Self;

    fn merge(&mut self, other: Self) {
        if self.parent.is_none() {
            self.parent = other.parent;
        }
        if self.album.is_none() {
            self.album = other.album;
        }
        if self.artist.is_none() {
            self.artist = other.artist;
        }
        if self.track.is_none() {
            self.track = other.track;
        }
        if self.year.is_none() {
            self.year = other.year;
        }
        if self.genre.is_none() {
            self.genre = other.genre;
        }
        if self.cover_art.is_none() {
            self.cover_art = other.cover_art;
        }
        if self.size.is_none() {
            self.size = other.size;
        }
        if self.suffix.is_none() {
            self.suffix = other.suffix;
        }
    }
}

impl FuzzyMatch for Song<'_> {
    fn is_similar_to(&self, other: &Self) -> bool {
        utils::string::is_similar(&self.title, &other.title)
            && utils::string::is_opt_similar(self.artist.as_deref(), other.artist.as_deref())
            && utils::string::is_opt_similar(self.album.as_deref(), other.album.as_deref())
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayGain {
    pub track_gain: Option<f32>,
    pub album_gain: Option<f32>,
    pub track_peak: Option<f32>,
    pub album_peak: Option<f32>,
    pub base_gain: Option<f32>,
    pub fallback_gain: Option<f32>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Work<'a> {
    pub name: CowStr<'a>,
    pub music_brainz_id: Option<CowStr<'a>>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Movement<'a> {
    pub name: CowStr<'a>,
    pub number: Option<u32>,
    pub count: Option<u32>,
}

impl<'a> TagOrigin for Contributor<'a> {
    fn tag_origin(&mut self, origin: &str, _id_type: IdType) {
        self.artist.tag_origin(origin, IdType::Artist);
    }
}

impl<'a> TagOrigin for Song<'a> {
    fn tag_origin(&mut self, origin: &str, _id_type: IdType) {
        self.id.tag_origin(origin, IdType::Song);
        self.parent.tag_origin(origin, IdType::Album);
        self.cover_art.tag_origin(origin, IdType::CoverArt);
        self.album_id.tag_origin(origin, IdType::Album);
        self.artist_id.tag_origin(origin, IdType::Artist);
        self.artists.tag_origin(origin, IdType::Artist);
        self.album_artists.tag_origin(origin, IdType::Artist);
        self.contributors.tag_origin(origin, IdType::Artist);
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TopSongs<'a> {
    pub song: Vec<Song<'a>>,
}

impl<'a> TagOrigin for TopSongs<'a> {
    fn tag_origin(&mut self, origin: &str, _id_type: IdType) {
        for song in &mut self.song {
            song.tag_origin(origin, IdType::Song);
        }
    }
}

impl SubsonicEntity for TopSongs<'_> {
    const FIELD_NAME: &'static str = "topSongs";
}

impl Mergeable for TopSongs<'_> {
    type Item = Self;

    fn merge(&mut self, other: Self) {
        for song in other.song {
            if !self.song.iter().any(|s| s.is_similar_to(&song)) {
                self.song.push(song);
            }
        }
    }
}
