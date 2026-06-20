use std::borrow::Cow;
use std::fmt::Display;

use serde::{Deserialize, Serialize};

pub const ORIGIN_SEP: &str = "___";
///// A type alias for a `Cow` that holds a string slice with a static lifetime.
//pub(super) type StaticCowStr = Cow<'static, str>;
/// A type alias for a `Cow` that holds a string slice with a generic lifetime.
pub(super) type CowStr<'a> = Cow<'a, str>;

pub(super) type OptCowStr<'a> = Option<CowStr<'a>>;

pub type OptId<'a> = Option<Id<'a>>;

pub trait FuzzyMatch {
    fn is_similar_to(&self, other: &Self) -> bool;
}

pub trait Mergeable {
    type Item;

    fn merge(&mut self, other: Self::Item);
}

impl<T> Mergeable for Vec<T>
where
    T: Mergeable<Item = T> + FuzzyMatch,
{
    type Item = Self;

    fn merge(&mut self, other: Self) {
        for other_item in other {
            if let Some(existing_item) =
                self.iter_mut().find(|item| item.is_similar_to(&other_item))
            {
                existing_item.merge(other_item);
            } else {
                self.push(other_item);
            }
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

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(from = "CowStr<'a>", into = "String")]
pub enum Id<'a> {
    Local { key: CowStr<'a> },
    External { origin: CowStr<'a>, id_type: IdType, key: CowStr<'a> },
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
                            }
                        }
                    };
                }
            }
        }
        Self::Local { key: raw }
    }

    pub const fn is_external(&self) -> bool {
        matches!(self, Self::External { .. })
    }

    pub fn to_raw_string(&self) -> String {
        match self {
            Self::Local { key } => key.to_string(),
            Self::External { origin, id_type, key } => {
                format!("{}{}{}{}{}", origin, ORIGIN_SEP, id_type.as_str(), ORIGIN_SEP, key)
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        match self {
            Self::Local { key } => key.is_empty(),
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
}

impl<'a> Default for Id<'a> {
    fn default() -> Self {
        Self::Local { key: Cow::Borrowed("") }
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
        id.to_raw_string()
    }
}

impl Display for Id<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_raw_string())
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
        if let Self::Local { key } = self {
            let new_key = std::mem::replace(key, Cow::Borrowed(""));
            *self =
                Self::External { origin: Cow::Owned(origin.to_string()), id_type, key: new_key };
        }
    }
}
