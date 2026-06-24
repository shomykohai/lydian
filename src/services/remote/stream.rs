use anyhow::Result;
use async_trait::async_trait;

use super::{RemoteError, RemoteOperation};
use crate::providers::{MusicProvider, StreamRequest};

#[derive(Debug)]
pub struct StreamMedia<'op> {
    pub request: StreamRequest<'op>,
    pub range_header: Option<String>,
}

#[async_trait]
impl<'op> RemoteOperation for StreamMedia<'op> {
    type Output = reqwest::Response;

    fn target(&self) -> &str {
        self.request.id.service().unwrap_or("")
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
