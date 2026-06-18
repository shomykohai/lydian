use std::borrow::Cow;

use anyhow::Result;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;

use super::{RemoteError, RemoteOperation};
use crate::models::TopSongs;
use crate::providers::MusicProvider;

#[derive(Debug, Deserialize)]
pub struct GetTopSongs<'op> {
    // The artist name might have a space, so
    // so using just &'op str is not feasible, or we would
    // get a parse error.
    pub artist: Cow<'op, str>,
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
        if items.is_empty() {
            return Err(RemoteError::EmptyDataset);
        }

        Ok(items.remove(0))
    }

    fn cache_key(&self) -> Option<Cow<'static, str>> {
        Some(format!("topsongs:{}_{}", self.artist.trim(), self.count.unwrap_or(50),).into())
    }

    fn deserialize(&self, value: Value) -> Option<Self::Output> {
        serde_json::from_value(value).ok()
    }

    fn serialize(&self, output: &Self::Output) -> Option<Value> {
        serde_json::to_value(output).ok()
    }
}
