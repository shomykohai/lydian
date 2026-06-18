use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

use crate::models::CowStr;

#[derive(Debug, Deserialize, Serialize)]
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
