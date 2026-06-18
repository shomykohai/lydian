use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

use super::{AlbumID3, CowStr};
use crate::models::{FuzzyMatch, IdType, Mergeable, SubsonicEntity, TagOrigin};
use crate::utils;

#[skip_serializing_none]
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtistID3<'a> {
    pub id: CowStr<'a>,
    pub name: CowStr<'a>,
    pub cover_art: Option<CowStr<'a>>,
    pub artist_image_url: Option<CowStr<'a>>,
    pub album_count: Option<u32>,
    pub starred: Option<DateTime<Utc>>,
    pub music_brainz_id: Option<CowStr<'a>>,
    pub sort_name: Option<CowStr<'a>>,
    pub roles: Option<Vec<CowStr<'a>>>,
}

impl FuzzyMatch for ArtistID3<'_> {
    fn is_similar_to(&self, other: &Self) -> bool {
        utils::string::is_similar(&self.name, &other.name)
    }
}

impl Mergeable for ArtistID3<'_> {
    type Item = Self;

    fn merge(&mut self, other: Self) {
        if self.cover_art.is_none() {
            self.cover_art = other.cover_art;
        }
        if self.artist_image_url.is_none() {
            self.artist_image_url = other.artist_image_url;
        }
        if self.album_count.is_none() {
            self.album_count = other.album_count;
        }
        if self.starred.is_none() {
            self.starred = other.starred;
        }
        if self.music_brainz_id.is_none() {
            self.music_brainz_id = other.music_brainz_id;
        }
        if self.sort_name.is_none() {
            self.sort_name = other.sort_name;
        }
        if self.roles.is_none() {
            self.roles = other.roles;
        }
    }
}

/*
 * Represent an artist.
 * The difference between this and `ArtistID3`
 * is mainly that this is used for a single
 * `getArtist` response, while `ArtistID3` is used for
 * a list of artists (like in search3)
 */
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtistWithAlbumsID3<'a> {
    #[serde(flatten)]
    pub base: ArtistID3<'a>,
    pub album: Option<Vec<AlbumID3<'a>>>,
}

impl SubsonicEntity for ArtistWithAlbumsID3<'_> {
    const FIELD_NAME: &'static str = "artist";
}

impl FuzzyMatch for ArtistWithAlbumsID3<'_> {
    fn is_similar_to(&self, other: &Self) -> bool {
        self.base.is_similar_to(&other.base)
    }
}

impl Mergeable for ArtistWithAlbumsID3<'_> {
    type Item = Self;

    fn merge(&mut self, other: Self) {
        self.base.merge(other.base);
        if let Some(other_album) = other.album {
            let this_album = self.album.get_or_insert_with(Vec::new);
            this_album.merge(other_album);
        }
    }
}

impl<'a> TagOrigin for ArtistID3<'a> {
    fn tag_origin(&mut self, origin: &str, id_type: IdType) {
        self.id.tag_origin(origin, id_type);
        self.cover_art.tag_origin(origin, IdType::CoverArt);
    }
}

impl<'a> TagOrigin for ArtistWithAlbumsID3<'a> {
    fn tag_origin(&mut self, origin: &str, id_type: IdType) {
        self.base.tag_origin(origin, id_type);
        self.album.tag_origin(origin, IdType::Album);
    }
}

#[skip_serializing_none]
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtistInfo<'a> {
    pub biography: Option<CowStr<'a>>,
    pub music_brainz_id: Option<CowStr<'a>>,
    pub last_fm_url: Option<CowStr<'a>>,
    pub small_image_url: Option<CowStr<'a>>,
    pub medium_image_url: Option<CowStr<'a>>,
    pub large_image_url: Option<CowStr<'a>>,
    pub similar_artist: Option<Vec<ArtistID3<'a>>>,
}

impl SubsonicEntity for ArtistInfo<'_> {
    const FIELD_NAME: &'static str = "artistInfo";
}

impl TagOrigin for ArtistInfo<'_> {
    fn tag_origin(&mut self, origin: &str, id_type: IdType) {
        if let Some(similar_artist) = &mut self.similar_artist {
            for artist in similar_artist {
                artist.tag_origin(origin, id_type);
            }
        }
    }
}

impl Mergeable for ArtistInfo<'_> {
    type Item = Self;

    fn merge(&mut self, other: Self) {
        if self.biography.is_none() {
            self.biography = other.biography;
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

        if self.similar_artist.is_none() {
            self.similar_artist = other.similar_artist;
        }
    }
}
