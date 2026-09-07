use std::str::FromStr;

use chrono::{DateTime, Datelike, NaiveDate, Utc};
use serde::{Deserialize, Deserializer, Serialize};
use serde_with::skip_serializing_none;

use crate::models::CowStr;

/// When deserializing a query parameter, treat an empty string as `None`.
/// Some subsonic endpoints return only the data marked as "required" in the schema,
/// so optional query parameters are sent as empty strings rather than being omitted.
///
/// This ensures that it won't happen.
pub fn empty_as_none<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: std::str::FromStr,
{
    let Some(raw) = Option::<String>::deserialize(deserializer)? else {
        return Ok(None);
    };

    Ok(raw.trim().parse().ok())
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ItemGenre<'a> {
    pub name: CowStr<'a>,
}

#[skip_serializing_none]
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ItemDate {
    pub year: Option<u16>,
    pub month: Option<u8>,
    pub day: Option<u8>,
}

impl FromStr for ItemDate {
    type Err = chrono::ParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        NaiveDate::parse_from_str(value.trim(), "%Y-%m-%d").map(Self::from)
    }
}

impl ItemDate {
    pub fn midnight_utc(&self) -> Option<DateTime<Utc>> {
        NaiveDate::from_ymd_opt(
            i32::from(self.year?),
            u32::from(self.month.unwrap_or(1)),
            u32::from(self.day.unwrap_or(1)),
        )?
        .and_hms_opt(0, 0, 0)
        .map(|dt| dt.and_utc())
    }
}

impl From<NaiveDate> for ItemDate {
    fn from(date: NaiveDate) -> Self {
        Self {
            year: u16::try_from(date.year()).ok(),
            month: u8::try_from(date.month()).ok(),
            day: u8::try_from(date.day()).ok(),
        }
    }
}

pub fn parse_timestamp(value: &str) -> Option<DateTime<Utc>> {
    let value = value.trim();

    DateTime::parse_from_rfc3339(value)
        .or_else(|_| DateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S%.f%z"))
        .map(|dt| dt.with_timezone(&Utc))
        .ok()
}
