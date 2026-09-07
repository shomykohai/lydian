use std::borrow::Cow;
use std::fmt::Display;

use bitvec::bitvec;
use serde::{Deserialize, Serialize};

pub const ORIGIN_SEP: &str = "___";
/// A type alias for a `Cow` that holds a string slice with a generic lifetime.
pub(super) type CowStr<'a> = Cow<'a, str>;

pub fn by_music_brainz(a: Option<&str>, b: Option<&str>) -> Option<bool> {
    let (a, b) = (a?.trim(), b?.trim());

    (!a.is_empty() && !b.is_empty()).then(|| a.eq_ignore_ascii_case(b))
}

/// What Subsonic returns when the client names no count.
pub const SEARCH_LIMIT: u32 = 20;
pub const SIMILAR_ARTIST_LIMIT: u32 = 20;
pub const TOP_SONG_LIMIT: u32 = 50;

pub trait Limit {
    fn limit(&mut self, count: u32);
}

impl<T> Limit for Vec<T> {
    fn limit(&mut self, count: u32) {
        self.truncate(count as usize);
    }
}

impl<T> Limit for Option<Vec<T>> {
    fn limit(&mut self, count: u32) {
        if let Some(items) = self.as_mut() {
            items.limit(count);
        }
    }
}

pub trait Mergeable {
    type Item;

    fn matches(&self, other: &Self::Item) -> bool {
        let _ = other;
        false
    }

    fn merge(&mut self, other: Self::Item);
}

impl<T> Mergeable for Vec<T>
where
    T: Mergeable<Item = T>,
{
    type Item = Self;

    fn merge(&mut self, other: Self) {
        let mut absorbed = bitvec![0; self.len()];

        for incoming in other {
            let slot = absorbed.iter_zeros().find(|&at| self[at].matches(&incoming));

            match slot {
                Some(at) => {
                    absorbed.set(at, true);
                    self[at].merge(incoming);
                }
                None => self.push(incoming),
            }
        }
    }
}

/// Inherit values from another model
pub trait Inherit {
    type From;

    fn inherit(&mut self, from: &Self::From);
}

impl<T: Inherit> Inherit for Option<T> {
    type From = T::From;

    fn inherit(&mut self, from: &Self::From) {
        if let Some(item) = self {
            item.inherit(from);
        }
    }
}

impl<T: Inherit> Inherit for Vec<T> {
    type From = T::From;

    fn inherit(&mut self, from: &Self::From) {
        for item in self {
            item.inherit(from);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IdType {
    Artist,
    Album,
    Song,
    Playlist,
    CoverArt,
    Generic,
}

impl IdType {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "artist" => Some(Self::Artist),
            "album" => Some(Self::Album),
            "song" => Some(Self::Song),
            "playlist" => Some(Self::Playlist),
            "coverart" => Some(Self::CoverArt),
            "generic" => Some(Self::Generic),
            _ => None,
        }
    }

    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Artist => "artist",
            Self::Album => "album",
            Self::Song => "song",
            Self::Playlist => "playlist",
            Self::CoverArt => "coverart",
            Self::Generic => "generic",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
#[serde(from = "CowStr<'a>")]
pub enum Id<'a> {
    Local { raw: CowStr<'a> },
    External { origin: CowStr<'a>, id_type: IdType, key: CowStr<'a>, raw: CowStr<'a> },
}

impl<'a> Id<'a> {
    pub fn parse<S>(raw: S) -> Self
    where
        S: Into<CowStr<'a>>,
    {
        let raw = raw.into();
        if let Some(f_idx) = raw.find(ORIGIN_SEP) {
            let remainder = &raw[f_idx + ORIGIN_SEP.len()..];
            if let Some(s_idx) = remainder.find(ORIGIN_SEP) {
                let id_type_str = &remainder[..s_idx];
                if let Some(id_type) = IdType::from_str(id_type_str) {
                    return match raw {
                        Cow::Borrowed(s) => {
                            let origin = &s[..f_idx];
                            let key = &s[f_idx + ORIGIN_SEP.len() + s_idx + ORIGIN_SEP.len()..];
                            Self::External {
                                origin: Cow::Borrowed(origin),
                                id_type,
                                key: Cow::Borrowed(key),
                                raw: Cow::Borrowed(s),
                            }
                        }
                        Cow::Owned(s) => {
                            let origin = s[..f_idx].to_string();
                            let key = s[f_idx + ORIGIN_SEP.len() + s_idx + ORIGIN_SEP.len()..]
                                .to_string();
                            Self::External {
                                origin: Cow::Owned(origin),
                                id_type,
                                key: Cow::Owned(key),
                                raw: Cow::Owned(s),
                            }
                        }
                    };
                }
            }
        }
        Self::Local { raw }
    }

    pub const fn is_external(&self) -> bool {
        matches!(self, Self::External { .. })
    }

    pub fn raw(&self) -> &str {
        match self {
            Self::Local { raw, .. } => raw,
            Self::External { raw, .. } => raw,
        }
    }

    pub fn is_empty(&self) -> bool {
        match self {
            Self::Local { raw } => raw.is_empty(),
            Self::External { key, .. } => key.is_empty(),
        }
    }

    pub fn service(&self) -> Option<&str> {
        match self {
            Self::Local { .. } => None,
            Self::External { origin, .. } => Some(origin.as_ref()),
        }
    }

    pub const fn id_type(&self) -> IdType {
        match self {
            Self::Local { .. } => IdType::Generic,
            Self::External { id_type, .. } => *id_type,
        }
    }

    pub fn key(&self) -> &str {
        match self {
            Self::Local { raw } => raw,
            Self::External { key, .. } => key,
        }
    }
}

impl<'a> Default for Id<'a> {
    fn default() -> Self {
        Self::Local { raw: Cow::Borrowed("") }
    }
}

impl<'a> From<CowStr<'a>> for Id<'a> {
    fn from(raw: CowStr<'a>) -> Self {
        Self::parse(raw)
    }
}

impl<'a> From<u64> for Id<'a> {
    fn from(value: u64) -> Self {
        Self::parse(Cow::Owned(value.to_string()))
    }
}

impl<'a> From<u32> for Id<'a> {
    fn from(value: u32) -> Self {
        Self::parse(Cow::Owned(value.to_string()))
    }
}

impl<'a> From<String> for Id<'a> {
    fn from(value: String) -> Self {
        Self::parse(Cow::Owned(value))
    }
}

impl<'a> From<&'a str> for Id<'a> {
    fn from(value: &'a str) -> Self {
        Self::parse(Cow::Borrowed(value))
    }
}

impl<'a, T> From<Option<T>> for Id<'a>
where
    T: Into<CowStr<'a>>,
{
    fn from(value: Option<T>) -> Self {
        value.map_or_else(Self::default, |v| Self::parse(v.into()))
    }
}

impl<'a> From<Id<'a>> for String {
    fn from(id: Id<'a>) -> Self {
        id.raw().to_string()
    }
}

impl Serialize for Id<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.raw())
    }
}

impl Display for Id<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.raw())
    }
}

pub trait TagOrigin {
    fn tag_origin(&mut self, origin: &str, id_type: IdType);
}

impl<'a> TagOrigin for CowStr<'a> {
    fn tag_origin(&mut self, origin: &str, id_type: IdType) {
        let prefix = format!("{}{}{}{}", origin, ORIGIN_SEP, id_type.as_str(), ORIGIN_SEP);

        if !self.starts_with(&prefix) {
            *self = Cow::Owned(format!("{}{}", prefix, self));
        }
    }
}

impl<T: TagOrigin> TagOrigin for Option<T> {
    fn tag_origin(&mut self, origin: &str, id_type: IdType) {
        if let Some(val) = self {
            val.tag_origin(origin, id_type);
        }
    }
}

impl<T: TagOrigin> TagOrigin for Vec<T> {
    fn tag_origin(&mut self, origin: &str, id_type: IdType) {
        for item in self {
            item.tag_origin(origin, id_type);
        }
    }
}

impl<'a> TagOrigin for Id<'a> {
    fn tag_origin(&mut self, origin: &str, id_type: IdType) {
        if let Self::Local { raw } = self {
            let new_key = std::mem::replace(raw, Cow::Borrowed(""));
            let new_raw =
                format!("{}{}{}{}", origin, ORIGIN_SEP, id_type.as_str(), ORIGIN_SEP) + &new_key;
            *self = Self::External {
                origin: Cow::Owned(origin.to_string()),
                id_type,
                key: new_key,
                raw: Cow::Owned(new_raw),
            };
        }
    }
}
