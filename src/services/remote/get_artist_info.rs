use std::borrow::Cow;

use anyhow::Result;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;

use super::{RemoteError, RemoteOperation};
use crate::MusicProvider;
use crate::models::{ArtistInfo, Id, Mergeable};
use crate::providers::ArtistRef;

#[derive(Debug, Deserialize)]
pub struct GetArtistInfo<'op> {
    #[serde(borrow)]
    pub id: Id<'op>,
    #[serde(default, deserialize_with = "crate::models::empty_as_none")]
    pub count: Option<u32>,
    #[serde(skip)]
    pub reference: Option<ArtistRef>,
}

#[async_trait]
impl<'op> RemoteOperation for GetArtistInfo<'op> {
    type Output = ArtistInfo<'op>;

    fn target(&self) -> &str {
        self.id.service().unwrap_or("")
    }

    async fn execute(&self, provider: &dyn MusicProvider) -> Result<Self::Output> {
        if provider.is_own_id(&self.id) {
            return provider.get_artist_info(&self.id).await;
        }

        let Some(reference) = &self.reference else {
            return Err(RemoteError::EmptyDataset.into());
        };

        let found = provider
            .find_artist(reference)
            .await
            .ok_or_else(|| anyhow::Error::from(RemoteError::EmptyDataset))?;

        Ok(provider.get_artist_info(&found.base.id).await?)
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

    fn deserialize(&self, value: &Value) -> Option<Self::Output> {
        Self::Output::deserialize(value).ok()
    }

    fn serialize(&self, output: &Self::Output) -> Option<Value> {
        serde_json::to_value(output).ok()
    }
}
