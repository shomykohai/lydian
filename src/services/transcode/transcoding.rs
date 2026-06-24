// src/services/transcode/cache.rs

use std::sync::Arc;
use std::task::{Context, Poll};

use axum::body::Body;
use futures::future::BoxFuture;
use tower::Service;

use super::TranscodeRequest;
use crate::services::stream::DiskStreamRequest;
use crate::storage::DiskDriver;
use crate::storage::keys::StorageKey;

#[derive(Clone)]
pub struct TranscodeService<S> {
    inner: S,
    disk: Arc<DiskDriver>,
}

impl<S> TranscodeService<S> {
    pub const fn new(inner: S, disk: Arc<DiskDriver>) -> Self {
        Self { inner, disk }
    }
}

impl<S> Service<TranscodeRequest> for TranscodeService<S>
where
    S: Service<DiskStreamRequest<TranscodeRequest>, Response = Body, Error = anyhow::Error>
        + Clone
        + Send
        + 'static,
    S::Future: Send + 'static,
{
    type Error = anyhow::Error;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;
    type Response = Body;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: TranscodeRequest) -> Self::Future {
        let mut inner = self.inner.clone();
        let disk = self.disk.clone();

        Box::pin(async move {
            let cache_key = StorageKey::TranscodedMedia {
                id: req.id.clone(),
                format: req.format,
                bitrate: req.bitrate,
                offset: req.offset,
            };

            if disk.has_key(&cache_key).await {
                let file = disk.read_stream_by_prefix(&cache_key.to_file_name()).await?;
                return Ok(Body::from_stream(file.stream));
            }

            let stream_req = DiskStreamRequest { inner_req: req, cache_key: cache_key.clone() };

            inner.call(stream_req).await
        })
    }
}
