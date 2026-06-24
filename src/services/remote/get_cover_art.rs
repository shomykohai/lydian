use anyhow::Result;
use async_trait::async_trait;
use reqwest::Response;
use serde::Deserialize;

use super::{RemoteError, RemoteOperation};
use crate::MusicProvider;
use crate::models::Id;

#[derive(Debug, Deserialize)]
pub struct GetCoverArt<'op> {
    #[serde(borrow)]
    pub id: Id<'op>,
    pub size: Option<u32>,
}

#[async_trait]
impl<'op> RemoteOperation for GetCoverArt<'op> {
    type Output = Response;

    fn target(&self) -> &str {
        self.id.service().unwrap_or("")
    }

    fn broadcast(&self) -> bool {
        false
    }

    async fn execute(&self, provider: &dyn MusicProvider) -> Result<Self::Output> {
        Ok(provider.get_cover_art(&self.id, self.size).await?)
    }

    fn process(&self, mut items: Vec<Self::Output>) -> Result<Self::Output, RemoteError> {
        if items.is_empty() {
            return Err(RemoteError::EmptyDataset);
        }
        Ok(items.remove(0))
    }

    fn cache(&self) -> bool {
        false
    }
}
