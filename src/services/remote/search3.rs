use std::borrow::Cow;

use anyhow::Result;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;

use super::{RemoteError, RemoteOperation};
use crate::MusicProvider;
use crate::models::{Mergeable, SearchResult};
use crate::providers::SearchRequest;

#[derive(Debug, Deserialize)]
#[serde(bound(deserialize = "'de: 'op"))]
pub struct Search3<'op> {
    #[serde(flatten)]
    pub request: SearchRequest<'op>,
}

#[async_trait]
impl<'op> RemoteOperation for Search3<'op> {
    type Output = SearchResult<'op>;

    fn target(&self) -> &str {
        ""
    }

    async fn execute(&self, provider: &dyn MusicProvider) -> Result<Self::Output> {
        Ok(provider.search(&self.request).await?)
    }

    fn process(&self, items: Vec<Self::Output>) -> Result<Self::Output, RemoteError> {
        if items.is_empty() {
            return Err(RemoteError::EmptyDataset);
        }

        let mut iter = items.into_iter();

        let mut first_res = iter.next().ok_or(RemoteError::ProcessingError)?;

        for res in iter {
            // Todo: Make sure we don't return more than asked
            first_res.merge(res);
        }

        Ok(first_res)

        // TODO: Merge and dedup
    }

    fn cache_key(&self) -> Option<Cow<'static, str>> {
        Some(
            format!(
                "search3:{}_{}_{}_{}_{}_{}_{}",
                self.request.query.trim(),
                self.request.album_count.unwrap_or(10),
                self.request.album_offset.unwrap_or(0),
                self.request.artist_count.unwrap_or(10),
                self.request.artist_offset.unwrap_or(0),
                self.request.song_count.unwrap_or(10),
                self.request.song_offset.unwrap_or(0),
            )
            .into(),
        )
    }

    fn deserialize(&self, value: Value) -> Option<Self::Output> {
        serde_json::from_value(value).ok()
    }

    fn serialize(&self, output: &Self::Output) -> Option<Value> {
        serde_json::to_value(output).ok()
    }
}
