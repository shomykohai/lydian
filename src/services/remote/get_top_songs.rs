use std::borrow::Cow;

use anyhow::Result;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;

use super::{RemoteError, RemoteOperation};
use crate::models::{Mergeable, TopSongs};
use crate::providers::MusicProvider;

#[derive(Debug, Deserialize)]
pub struct GetTopSongs<'op> {
    // The artist name might have a space, so
    // so using just &'op str is not feasible, or we would
    // get a parse error.
    pub artist: Cow<'op, str>,
    #[serde(default, deserialize_with = "crate::models::empty_as_none")]
    pub count: Option<u32>,
}

#[async_trait]
impl<'op> RemoteOperation for GetTopSongs<'op> {
    type Output = TopSongs<'op>;

    fn target(&self) -> &str {
        ""
    }

    async fn execute(&self, provider: &dyn MusicProvider) -> Result<Self::Output> {
        provider.get_top_songs(&self.artist, self.count).await
    }

    fn process(&self, mut items: Vec<Self::Output>) -> Result<Self::Output, RemoteError> {
        /* Some providers might return an empty set, which caused to cache empty results
         * for as long as the TTL.
         */
        items.retain(|top| top.song.as_ref().is_some_and(|songs| !songs.is_empty()));

        if items.is_empty() {
            return Err(RemoteError::EmptyDataset);
        }

        items.sort_by_key(|top| std::cmp::Reverse(top.song.as_ref().map_or(0, Vec::len)));

        let mut merged = items.remove(0);

        for rest in items {
            merged.merge(rest);
        }

        Ok(merged)
    }

    fn cache_key(&self) -> Option<Cow<'static, str>> {
        let count = self.count.map_or_else(|| "default".to_string(), |count| count.to_string());

        Some(format!("topsongs:{}_{count}", self.artist.trim()).into())
    }

    fn deserialize(&self, value: &Value) -> Option<Self::Output> {
        Self::Output::deserialize(value).ok()
    }

    fn serialize(&self, output: &Self::Output) -> Option<Value> {
        serde_json::to_value(output).ok()
    }
}
