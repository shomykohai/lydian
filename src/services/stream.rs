use std::io::SeekFrom;
use std::sync::Arc;
use std::task::{Context, Poll};

use axum::body::{Body, Bytes};
use futures::StreamExt;
use futures::future::BoxFuture;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::sync::broadcast;
use tower::Service;

use crate::Error;
use crate::storage::DiskDriver;
use crate::storage::keys::StorageKey;

pub struct DiskStreamRequest<R> {
    pub inner_req: R,
    pub cache_key: StorageKey,
}

/*
 * Wrapper that intercepts a stream request, and splits it into
 * two sinks, so writing to disk and streaming to an HTTP request
 * body isn't pain (hopefully).
 * A.k.a. we love unnecessary abstractions :3
 */
#[derive(Clone)]
pub struct DiskStreamService<S> {
    inner: S,
    disk: Arc<DiskDriver>,
}

impl<S> DiskStreamService<S> {
    pub const fn new(inner: S, disk: Arc<DiskDriver>) -> Self {
        Self { inner, disk }
    }
}

impl<S, R> Service<DiskStreamRequest<R>> for DiskStreamService<S>
where
    S: Service<R, Response = Body, Error = anyhow::Error> + Clone + Send + 'static,
    S::Future: Send + 'static,
    R: Send + 'static,
{
    type Error = anyhow::Error;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;
    type Response = Body;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    /*
     * So basically, what we do here is:
     * 1. Get the raw stream from the inner service (ffmpeg most likely)
     * 2. Create a temp key on disk
     * 3. Create a broadcast channel to send the stream to both disk and HTTP response
     * 4. Spawn a background task to write to disk and send to the broadcast channel
     * 5. Create a stream that reads from the broadcast channel and the disk file
     * 6. Return the stream as the response body
     *
     * But overcomplicated!!
     * Oh and also, handles writing to storage and then streaming from it once ffmpeg is done,
     * so ffmpeg doesn't have to be running for the whole stream (tested it with a 40m track and
     * ffmpeg was waiting for it to end before terminating the process hehe)
     */
    fn call(&mut self, req: DiskStreamRequest<R>) -> Self::Future {
        let mut inner = self.inner.clone();
        let disk = self.disk.clone();

        Box::pin(async move {
            let raw_body = inner.call(req.inner_req).await?;
            let mut body_stream = raw_body.into_data_stream();
            let temp_file = disk.create_temp_for_key(&req.cache_key).await.ok();

            let (tx, rx) = broadcast::channel::<Bytes>(32);

            let disk_clone = disk.clone();
            let cache_key = req.cache_key.clone();
            let tx_bg = tx.clone();

            tokio::spawn(async move {
                let mut temp_file = temp_file;
                let mut completed = true;

                while let Some(Ok(chunk)) = body_stream.next().await {
                    if let Some(file) = &mut temp_file
                        && file.write_all(&chunk).await.is_err()
                    {
                        temp_file = None;
                        completed = false;
                    }
                    let _ = tx_bg.send(chunk);
                }

                if completed
                    && let Some(mut file) = temp_file
                    && file.flush().await.is_ok()
                {
                    let _ = disk_clone.commit_temp_for_key(&cache_key).await;
                }
            });

            let file_path = Arc::new(disk.resolve_path(&req.cache_key.to_file_name()));

            let initial_state = (0u64, None::<File>, Some(rx));

            let response_stream = futures::stream::unfold(
                initial_state,
                move |(mut bytes_sent, disk_file, rx_opt)| {
                    let file_path = file_path.clone();
                    let tx = tx.clone();

                    async move {
                        let mut buf = vec![0u8; 0x4000];

                        if let Some(mut f) = disk_file {
                            match f.read(&mut buf).await {
                                Ok(0) => return None,
                                Ok(n) => {
                                    buf.truncate(n);
                                    let res: Result<Bytes, Error> = Ok(Bytes::from(buf));
                                    return Some((res, (bytes_sent, Some(f), rx_opt)));
                                }
                                Err(_) => return None,
                            }
                        }

                        let mut current_rx = rx_opt.unwrap_or_else(|| tx.subscribe());
                        match current_rx.recv().await {
                            Ok(chunk) => {
                                bytes_sent += chunk.len() as u64;
                                let res: Result<Bytes, Error> = Ok(chunk);
                                Some((res, (bytes_sent, None, Some(current_rx))))
                            }
                            Err(_) => {
                                if let Ok(mut f) = File::open(file_path.as_ref()).await
                                    && f.seek(SeekFrom::Start(bytes_sent)).await.is_ok()
                                    && let Ok(n) = f.read(&mut buf).await
                                    && n > 0
                                {
                                    buf.truncate(n);
                                    let res: Result<Bytes, Error> = Ok(Bytes::from(buf));
                                    return Some((res, (bytes_sent, Some(f), None)));
                                }
                                None
                            }
                        }
                    }
                },
            );

            Ok(Body::from_stream(response_stream))
        })
    }
}
