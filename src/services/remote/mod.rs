mod get_album;
mod get_album_by_name;
mod get_album_info;
mod get_artist;
mod get_artist_info;
mod get_cover_art;
mod get_song;
mod get_top_songs;
mod search3;
mod stream;

use std::any::type_name;
use std::borrow::Cow;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use anyhow::Result;
use async_trait::async_trait;
use futures::StreamExt;
use futures::future::BoxFuture;
use futures::stream::FuturesUnordered;
pub use get_album::*;
pub use get_album_by_name::*;
pub use get_album_info::*;
pub use get_artist::*;
pub use get_artist_info::*;
pub use get_cover_art::*;
pub use get_song::*;
pub use get_top_songs::*;
use log::{debug, trace};
use moka::future::Cache;
use reqwest::StatusCode;
pub use search3::*;
use serde_json::Value;
pub use stream::*;
use thiserror::Error as TError;
use tower::Service;

use crate::models::SubsonicErrorCode;
use crate::providers::MusicProvider;

const PROVIDER_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(TError, Debug)]
pub enum RemoteError {
    #[error("Remote operation failed with status code: {0}")]
    Status(StatusCode),
    #[error("Failed to process remote dataset")]
    ProcessingError,
    #[error("Remote dataset is empty")]
    EmptyDataset,
    #[error("Operation is not supported by provider `{0}`")]
    Unsupported(&'static str),
}

#[async_trait]
pub trait RemoteOperation: Send + Sync {
    type Output: Send;

    /// Returns the unique identifier of a remote source, in case the operation
    /// needs to be performed on a single source.
    ///
    /// Example: "qobuz", "tidal"...
    fn target(&self) -> &str;

    /// Whether if the operation should or not be broadcasted to all remotes. If false, the
    /// operation will only be sent to the remote specified by `target()`.
    fn broadcast(&self) -> bool {
        true
    }

    async fn execute(&self, provider: &dyn MusicProvider) -> Result<Self::Output>;

    /// Processes a batch of items from a remote dataset, resolves any conflicts,
    /// removes duplicates, and returns a single result.
    fn process(&self, items: Vec<Self::Output>) -> Result<Self::Output, RemoteError>;

    /// Whether this remote operation supports caching of its results.
    fn cache(&self) -> bool {
        true
    }

    /// If caching is supported, returns the cache key used to store and retrieve `Self::Output`
    /// from the cache.
    fn cache_key(&self) -> Option<Cow<'_, str>> {
        None
    }

    /// If cache is supported, returns a deserialized value from the cached JSON value.
    fn deserialize(&self, _value: &Value) -> Option<Self::Output> {
        None
    }

    /// If cache is supported, returns a JSON value to be stored in cache.
    fn serialize(&self, _output: &Self::Output) -> Option<Value> {
        None
    }
}

fn is_missing(error: &anyhow::Error) -> bool {
    if let Some(error) = error.downcast_ref::<RemoteError>() {
        return match error {
            RemoteError::EmptyDataset | RemoteError::Unsupported(_) => true,
            RemoteError::Status(status) => status.is_client_error(),
            RemoteError::ProcessingError => false,
        };
    }

    matches!(error.downcast_ref::<SubsonicErrorCode>(), Some(SubsonicErrorCode::NotFound))
}

pub struct RemoteLookupService<'a> {
    pub remotes: &'a [Arc<dyn MusicProvider>],
    pub cache: Arc<Cache<String, Arc<Value>>>,
}

impl<'a, Op> Service<Op> for RemoteLookupService<'a>
where
    Op: RemoteOperation + 'a,
{
    type Error = RemoteError;
    type Future = BoxFuture<'a, Result<Self::Response, Self::Error>>;
    type Response = Op::Output;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, op: Op) -> Self::Future {
        let remotes = self.remotes;
        let cache = self.cache.clone();

        let op = Arc::new(op);
        Box::pin(async move {
            let operation = type_name::<Op>();
            let target_source = op.target();

            if op.cache()
                && let Some(cache_key) = op.cache_key()
                && let Some(cached_json) = cache.get(&*cache_key).await
            {
                if cached_json.is_null() {
                    return Err(RemoteError::EmptyDataset);
                }

                if let Some(deserialized) = op.deserialize(&cached_json) {
                    trace!("Cache hit for operation {operation} with key {cache_key}");
                    return Ok(deserialized);
                }
            }

            let mut workers: FuturesUnordered<_> = remotes
                .iter()
                .filter(|r| r.id() == target_source || (target_source.is_empty() && op.broadcast()))
                .map(|r| {
                    let op = op.clone();

                    async move {
                        tokio::time::timeout(PROVIDER_TIMEOUT, op.execute(r.as_ref()))
                            .await
                            .unwrap_or_else(|_| {
                                debug!("Remote {} timed out for {operation}", r.id());
                                Err(RemoteError::ProcessingError.into())
                            })
                    }
                })
                .collect();

            debug!("Spawned {} remote workers for operation {operation}", workers.len());

            let mut results = vec![];
            let mut unanswered = false;
            let start_time = Instant::now();

            while let Some(res) = workers.next().await {
                match res {
                    Ok(output) => results.push(output),
                    Err(err) => {
                        unanswered |= !is_missing(&err);
                        trace!("Remote worker failed for {operation}: {err:#}");
                    }
                }
            }

            let processed = match op.process(results) {
                Ok(processed) => processed,
                Err(err) => {
                    if !unanswered
                        && op.cache()
                        && let Some(cache_key) = op.cache_key()
                    {
                        cache.insert(cache_key.to_string(), Arc::new(Value::Null)).await;
                    }

                    return Err(err);
                }
            };

            debug!("Remote operation {operation} completed in {:.2?}", start_time.elapsed());

            if op.cache()
                && let Some(cache_key) = op.cache_key()
                && let Some(serialized_value) = op.serialize(&processed)
            {
                cache.insert(cache_key.to_string(), Arc::new(serialized_value)).await;
            }

            Ok(processed)
        })
    }
}
