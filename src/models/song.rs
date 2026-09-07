use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

use crate::models::{
    AlbumID3,
    ArtistID3,
    CowStr,
    Id,
    IdType,
    Inherit,
    ItemGenre,
    Mergeable,
    SubsonicEntity,
    TagOrigin,
    by_music_brainz,
};
use crate::utils::metadata::{compare_artists, compare_titles};

#[skip_serializing_none]
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Contributor<'a> {
    pub role: CowStr<'a>,
    pub sub_role: Option<CowStr<'a>>,
    pub artist: ArtistID3<'a>,
}

#[skip_serializing_none]
#[derive(Debug, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Song<'a> {
    pub id: Id<'a>,
    pub parent: Option<Id<'a>>,
    pub is_dir: bool,
    pub title: CowStr<'a>,
    pub album: Option<CowStr<'a>>,
    pub artist: Option<CowStr<'a>>,
    pub track: Option<u32>,
    pub year: Option<u32>,
    pub genre: Option<CowStr<'a>>,
    pub cover_art: Option<Id<'a>>,
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
    pub created: Option<DateTime<Utc>>,
    pub starred: Option<DateTime<Utc>>,
    pub album_id: Option<Id<'a>>,
    pub artist_id: Option<Id<'a>>,
    #[serde(rename = "type")]
    pub ty: Option<CowStr<'a>>,
    pub media_type: Option<CowStr<'a>>,
    pub bookmark_position: Option<u64>,
    pub original_width: Option<u32>,
    pub original_height: Option<u32>,
    pub played: Option<DateTime<Utc>>,
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
    pub is_external: Option<bool>,
}

impl SubsonicEntity for Song<'_> {
    const FIELD_NAME: &'static str = "song";
}

impl Mergeable for Song<'_> {
    type Item = Self;

    fn matches(&self, other: &Self::Item) -> bool {
        if let Some(verdict) =
            by_music_brainz(self.music_brainz_id.as_deref(), other.music_brainz_id.as_deref())
        {
            return verdict;
        }

        if self.track != other.track {
            return false;
        }

        if let (Some(d1), Some(d2)) = (self.disc_number, other.disc_number)
            && d1 != d2
        {
            return false;
        }

        if let (Some(album1), Some(album2)) = (self.album.as_deref(), other.album.as_deref())
            && !compare_titles(album1, album2)
        {
            return false;
        }

        compare_titles(self.title.as_ref(), other.title.as_ref())
            && compare_artists(self.artist.as_deref(), other.artist.as_deref())
    }

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

#[skip_serializing_none]
#[derive(Debug, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ReplayGain {
    pub track_gain: Option<f32>,
    pub album_gain: Option<f32>,
    pub track_peak: Option<f32>,
    pub album_peak: Option<f32>,
    pub base_gain: Option<f32>,
    pub fallback_gain: Option<f32>,
}

#[skip_serializing_none]
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Work<'a> {
    pub name: CowStr<'a>,
    pub music_brainz_id: Option<CowStr<'a>>,
}

#[skip_serializing_none]
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Movement<'a> {
    pub name: CowStr<'a>,
    pub number: Option<u32>,
    pub count: Option<u32>,
}

impl<'a> Inherit for Song<'a> {
    type From = AlbumID3<'a>;

    fn inherit(&mut self, album: &AlbumID3<'a>) {
        if self.album.is_none() {
            self.album = Some(album.name.clone());
        }
        if self.album_id.is_none() {
            self.album_id = Some(album.id.clone());
        }
        if self.parent.is_none() {
            self.parent = self.album_id.clone();
        }
        if self.cover_art.is_none() {
            self.cover_art = album.cover_art.clone();
        }
        if self.year.is_none() {
            self.year = album.year;
        }
        if self.genre.is_none() {
            self.genre = album.genre.clone();
        }
        if self.genres.is_none() {
            self.genres = album.genres.clone();
        }
        if self.album_artists.is_none() {
            self.album_artists = album.artists.clone();
        }
        if self.display_album_artist.is_none() {
            self.display_album_artist =
                album.display_artist.clone().or_else(|| album.artist.clone());
        }
    }
}

impl<'a> TagOrigin for Contributor<'a> {
    fn tag_origin(&mut self, origin: &str, _id_type: IdType) {
        self.artist.tag_origin(origin, IdType::Artist);
    }
}

impl<'a> TagOrigin for Song<'a> {
    fn tag_origin(&mut self, origin: &str, _id_type: IdType) {
        self.is_external = Some(true);
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

#[skip_serializing_none]
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TopSongs<'a> {
    #[serde(default)]
    pub song: Option<Vec<Song<'a>>>,
}

impl<'a> TagOrigin for TopSongs<'a> {
    fn tag_origin(&mut self, origin: &str, _id_type: IdType) {
        self.song.tag_origin(origin, IdType::Song);
    }
}

impl SubsonicEntity for TopSongs<'_> {
    const FIELD_NAME: &'static str = "topSongs";
}

impl Mergeable for TopSongs<'_> {
    type Item = Self;

    fn merge(&mut self, other: Self) {
        match (&mut self.song, other.song) {
            (Some(self_songs), Some(other_songs)) => {
                self_songs.merge(other_songs);
            }
            (self_songs @ None, Some(other_songs)) => {
                *self_songs = Some(other_songs);
            }
            _ => {}
        }
    }
}
