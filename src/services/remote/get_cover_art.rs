use anyhow::Result;
use async_trait::async_trait;
use reqwest::Response;
use serde::Deserialize;

use super::{RemoteError, RemoteOperation};
use crate::MusicProvider;
use crate::models::ORIGIN_SEP;

#[derive(Debug, Deserialize)]
pub struct GetCoverArt {
    pub id: String,
    pub size: Option<u32>,
}

#[async_trait]
impl RemoteOperation for GetCoverArt {
    type Output = Response;

    fn target(&self) -> &str {
        if let Some((prefix, _)) = self.id.split_once(ORIGIN_SEP) { prefix } else { "" }
    }

    fn broadcast(&self) -> bool {
        false
    }

    async fn execute(&self, provider: &dyn MusicProvider) -> Result<Self::Output> {
        let upstream_id = if let Some((_, raw_id)) = self.id.split_once(ORIGIN_SEP) {
            raw_id
        } else {
            self.id.as_str()
        };

        Ok(provider.get_cover_art(upstream_id, self.size).await?)
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
