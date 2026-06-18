use std::borrow::Cow;

pub const ORIGIN_SEP: &str = "___";
/// A type alias for a `Cow` that holds a string slice with a generic lifetime.
pub(super) type CowStr<'a> = Cow<'a, str>;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
