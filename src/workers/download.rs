use std::collections::HashSet;
use std::fmt::Display;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Result, bail};
use axum::body::Bytes;
use futures::StreamExt;
use log::{debug, error, info};
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::sync::{Mutex, mpsc, watch};
use tokio::task::JoinHandle;

use crate::config::LydianConfig;
use crate::storage::DiskDriver;
use crate::storage::keys::StorageKey;
use crate::utils::fmp4::Fmp4;

const WRITE_BUFFER_SIZE: usize = 512 * 1024;
/// Look ahead because latency on the client is bad
const SEGMENT_READAHEAD: usize = 4;
/// From reqwests
const PLACEHOLDER_HOST: &str = "no.url.provided.local";

/// The layout the received segments.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SegmentLayout {
    /// Contiguous stream of bytes segments.
    #[default]
    Contiguous,
    /// Each segment is a fragment of an fMP4 file.
    FlacInMp4,
}

impl SegmentLayout {
    /// Converts a segment to the layout we expect to store on disk or send to the client.
    pub fn rewrite(self, index: usize, segment: Bytes) -> Result<Bytes> {
        let Self::FlacInMp4 = self else {
            return Ok(segment);
        };

        if index == 0 {
            return Fmp4::flac_header(&segment);
        }

        let at = Fmp4::payload_at(&segment)?;

        if at > segment.len() {
            bail!("segment {index} is shorter than the boxes it declares");
        }

        Ok(segment.slice(at..))
    }
}

/// How the bytes behind a `DownloadJob` are fetched.
#[derive(Debug, Clone)]
pub enum DownloadSource {
    /// Full payload in one URL
    Url(String),
    /// Multiple segments in multiple URLs
    Segments {
        urls: Arc<[Arc<str>]>,
        content_type: String,
        /// What type of segments these are
        layout: SegmentLayout,
    },
}

impl DownloadSource {
    pub fn from_response(resp: &reqwest::Response) -> Option<Self> {
        if let Some(source) = resp.extensions().get::<Self>() {
            return Some(source.clone());
        }

        let url = resp.url();

        (url.host_str() != Some(PLACEHOLDER_HOST)).then(|| Self::Url(url.to_string()))
    }

    pub fn content_type(&self) -> Option<&str> {
        match self {
            Self::Url(_) => None,
            Self::Segments { content_type, .. } => Some(content_type),
        }
    }
}

impl Display for DownloadSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Url(url) => write!(f, "{}", url),
            Self::Segments { urls, .. } => write!(f, "{} segments", urls.len()),
        }
    }
}

// TODO: Probably can be dropped
enum Fetched {
    Single(reqwest::Response),
    Segments { urls: Arc<[Arc<str>]>, layout: SegmentLayout },
}

#[derive(Debug)]
pub struct DownloadJob {
    pub source: DownloadSource,
    pub file_key: String,
    pub complete_tx: Option<Arc<watch::Sender<bool>>>,
}

pub struct DownloadManager {
    http_client: reqwest::Client,
    disk: Arc<DiskDriver>,
    active_downloads: Arc<Mutex<HashSet<String>>>,
}

impl DownloadManager {
    pub fn init(
        cfg: &LydianConfig,
        http_client: reqwest::Client,
        disk: Arc<DiskDriver>,
    ) -> (mpsc::UnboundedSender<DownloadJob>, Vec<JoinHandle<()>>) {
        let (tx, rx) = mpsc::unbounded_channel::<DownloadJob>();

        let shared_rx = Arc::new(Mutex::new(rx));

        let manager = Arc::new(Self {
            http_client,
            disk,
            active_downloads: Arc::new(Mutex::new(HashSet::new())),
        });

        let workers = cfg.download.max_workers;
        info!("Spawning {} download workers", workers);

        let mut handles = Vec::new();

        for _ in 0..workers {
            let manager_clone = manager.clone();
            let rx_clone = shared_rx.clone();

            let handle = tokio::spawn(async move {
                loop {
                    let mut lock = rx_clone.lock().await;
                    let job_opt = lock.recv().await;
                    drop(lock);

                    let Some(job) = job_opt else {
                        break;
                    };

                    let job_key = job.file_key.clone();

                    let claimed = {
                        let mut active = manager_clone.active_downloads.lock().await;
                        active.insert(job_key.clone())
                    };

                    let already_on_disk =
                        !claimed || manager_clone.disk.exists_by_prefix(&job_key).await;

                    if already_on_disk {
                        if claimed {
                            manager_clone.active_downloads.lock().await.remove(&job_key);
                        }

                        /*
                         * We don't need to download it, but we stil need to tell the caller
                         * that the download is "complete".
                         */
                        if let Some(tx) = job.complete_tx {
                            let _ = tx.send(true);
                        }

                        continue;
                    }

                    if let Err(e) = manager_clone.execute_download(&job).await {
                        error!("Background download failed for '{}': {:?}", job_key, e);
                    }

                    manager_clone.active_downloads.lock().await.remove(&job_key);

                    if let Some(tx) = job.complete_tx {
                        let _ = tx.send(true);
                    }
                }
            });

            handles.push(handle);
        }

        (tx, handles)
    }

    async fn execute_download(&self, job: &DownloadJob) -> Result<()> {
        debug!("Starting download for: {}", job.source);

        let (ext, body) = match &job.source {
            DownloadSource::Url(url) => {
                let response = self.http_client.get(url).send().await?;

                if !response.status().is_success() {
                    bail!(
                        "Upstream server rejected download connection with status: {}",
                        response.status()
                    );
                }

                let content_type = response
                    .headers()
                    .get(reqwest::header::CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
                    .to_owned();

                (StorageKey::extension_for(&job.file_key, &content_type), Fetched::Single(response))
            }
            DownloadSource::Segments { urls, content_type, layout } => {
                (StorageKey::extension_for(&job.file_key, content_type), Fetched::Segments {
                    urls: Arc::clone(urls),
                    layout: *layout,
                })
            }
        };

        let final_key = format!("{}.{}", job.file_key, ext);
        let path = Path::new(&final_key);
        let parent = path.parent().unwrap_or_else(|| Path::new(""));
        let name = path.file_name().ok_or_else(|| anyhow::anyhow!("Invalid file name"))?;
        let tmp_key = parent.join("tmp").join(name).to_string_lossy().to_string();

        let file = self.disk.create_file(&tmp_key).await?;

        let mut writer = BufWriter::with_capacity(WRITE_BUFFER_SIZE, file);

        match body {
            Fetched::Single(response) => {
                let mut stream = response.bytes_stream();

                while let Some(chunk_res) = stream.next().await {
                    writer.write_all(&chunk_res?).await?;
                }
            }
            Fetched::Segments { urls, layout } => {
                let mut stream = Box::pin(
                    self.segments(urls)
                        .enumerate()
                        .map(move |(at, segment)| layout.rewrite(at, segment?)),
                );

                while let Some(chunk) = stream.next().await {
                    writer.write_all(&chunk?).await?;
                }
            }
        }

        writer.flush().await?;

        let file = writer.into_inner();
        file.sync_all().await?;
        drop(file);

        self.disk.rename_file(&tmp_key, &final_key).await?;

        debug!("Background download concluded: {}", final_key);
        Ok(())
    }

    fn segments(
        &self,
        urls: Arc<[Arc<str>]>,
    ) -> impl futures::Stream<Item = Result<Bytes>> + Send + 'static {
        let client = self.http_client.clone();

        futures::stream::iter(urls.to_vec())
            .map(move |url| {
                let client = client.clone();

                async move {
                    Ok(client.get(&*url).send().await?.error_for_status()?.bytes().await?)
                }
            })
            .buffered(SEGMENT_READAHEAD)
    }
}
