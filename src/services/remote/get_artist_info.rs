use std::borrow::Cow;

use anyhow::Result;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;

use super::{RemoteError, RemoteOperation};
use crate::MusicProvider;
use crate::models::{ArtistInfo, Id, Mergeable};

#[derive(Debug, Deserialize)]
pub struct GetArtistInfo<'op> {
    #[serde(borrow)]
    pub id: Id<'op>,
}

#[async_trait]
impl<'op> RemoteOperation for GetArtistInfo<'op> {
    type Output = ArtistInfo<'op>;

    fn target(&self) -> &str {
        self.id.service().unwrap_or("")
    }

    async fn execute(&self, provider: &dyn MusicProvider) -> Result<Self::Output> {
        Ok(provider.get_artist_info(&self.id).await?)
    }

    fn process(&self, items: Vec<Self::Output>) -> Result<Self::Output, RemoteError> {
        if items.is_empty() {
            return Err(RemoteError::EmptyDataset);
        }

        let mut iter = items.into_iter();

        let mut main_artist = iter.next().ok_or(RemoteError::ProcessingError)?;

        for artist in iter {
            main_artist.merge(artist);
        }

        Ok(main_artist)
    }

    fn cache_key(&self) -> Option<Cow<'_, str>> {
        Some(format!("get_artist_info:{}", self.id.raw()).into())
    }

    fn deserialize(&self, value: Value) -> Option<Self::Output> {
        serde_json::from_value(value).ok()
    }

    fn serialize(&self, output: &Self::Output) -> Option<Value> {
        serde_json::to_value(output).ok()
    }
}
