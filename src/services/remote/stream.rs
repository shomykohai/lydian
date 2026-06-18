use anyhow::Result;
use async_trait::async_trait;

use super::{RemoteError, RemoteOperation};
use crate::models::ORIGIN_SEP;
use crate::providers::{MusicProvider, StreamRequest};

#[derive(Debug)]
pub struct StreamMedia {
    pub request: StreamRequest,
    pub range_header: Option<String>,
}

#[async_trait]
impl RemoteOperation for StreamMedia {
    type Output = reqwest::Response;

    fn target(&self) -> &str {
        if let Some((prefix, _)) = self.request.id.split_once(ORIGIN_SEP) { prefix } else { "" }
    }

    fn broadcast(&self) -> bool {
        false
    }

    async fn execute(&self, provider: &dyn MusicProvider) -> Result<Self::Output> {
        provider.stream(self).await
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
