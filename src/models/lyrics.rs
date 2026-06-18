#![allow(dead_code)]

use serde::{Deserialize, Serialize};

use crate::models::CowStr;

#[derive(Debug, Deserialize, Serialize)]
pub struct Lyrics<'a> {
    pub value: CowStr<'a>,
    pub artist: Option<CowStr<'a>>,
    pub title: Option<CowStr<'a>>,
}

pub struct StructuredLyrics<'a> {
    pub lang: CowStr<'a>,
    pub synced: bool,
    pub line: Vec<LyricLine<'a>>,
    pub display_artist: Option<CowStr<'a>>,
    pub display_title: Option<CowStr<'a>>,
    pub offset: Option<u32>,
    pub kind: Option<CowStr<'a>>,
    pub agents: Vec<Agent<'a>>,
    pub cue_line: Vec<CueLine<'a>>,
}

pub struct LyricsList<'a> {
    pub structured_lyrics: Vec<StructuredLyrics<'a>>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct LyricLine<'a> {
    pub value: CowStr<'a>,
    pub start: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Agent<'a> {
    pub id: CowStr<'a>,
    pub role: CowStr<'a>,
    pub name: Option<CowStr<'a>>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Cue<'a> {
    pub start: u32,
    pub end: Option<u32>,
    pub byte_start: u32,
    pub byte_end: u32,
    pub value: CowStr<'a>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct CueLine<'a> {
    pub index: usize,
    pub start: Option<u32>,
    pub end: Option<u32>,
    pub value: CowStr<'a>,
    pub agent_id: Option<CowStr<'a>>,
    pub cue: Vec<Cue<'a>>,
}
