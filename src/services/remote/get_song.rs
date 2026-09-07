use std::borrow::Cow;

use anyhow::Result;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;

use super::{RemoteError, RemoteOperation};
use crate::models::{Id, Song};
use crate::providers::MusicProvider;

#[derive(Debug, Deserialize)]
pub struct GetSong<'op> {
    #[serde(borrow)]
    pub id: Id<'op>,
}

#[async_trait]
impl<'op> RemoteOperation for GetSong<'op> {
    type Output = Song<'op>;

    fn target(&self) -> &str {
        self.id.service().unwrap_or("")
    }

    fn broadcast(&self) -> bool {
        false
    }

    async fn execute(&self, provider: &dyn MusicProvider) -> Result<Self::Output> {
        provider.get_song(&self.id).await
    }

    fn process(&self, mut items: Vec<Self::Output>) -> Result<Self::Output, RemoteError> {
        if items.is_empty() {
            return Err(RemoteError::EmptyDataset);
        }

        Ok(items.remove(0))
    }

    fn cache_key(&self) -> Option<Cow<'_, str>> {
        Some(Cow::Borrowed(self.id.raw()))
    }

    fn deserialize(&self, value: &Value) -> Option<Self::Output> {
        Self::Output::deserialize(value).ok()
    }

    fn serialize(&self, output: &Self::Output) -> Option<Value> {
        serde_json::to_value(output).ok()
    }
}
