use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Result, bail};
use futures::StreamExt;
use log::{debug, error, info};
use tokio::io::AsyncWriteExt;
use tokio::sync::{Mutex, mpsc, watch};
use tokio::task::JoinHandle;

use crate::config::LydianConfig;
use crate::storage::DiskDriver;

#[derive(Debug)]
pub struct DownloadJob {
    pub source_url: String,
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

                    let mut active = manager_clone.active_downloads.lock().await;
                    if active.contains(&job_key) || manager_clone.disk.exists(&job_key).await {
                        continue;
                    }
                    active.insert(job_key.clone());
                    drop(active);

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
        debug!("Starting download for: {}", job.source_url);

        let response = self.http_client.get(&job.source_url).send().await?;
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
            .unwrap_or("");

        let ext = match content_type {
            "audio/flac" | "audio/x-flac" => "flac",
            "audio/ogg" | "application/ogg" => "ogg",
            "audio/mp4" | "audio/x-m4a" => "m4a",
            "audio/wav" | "audio/x-wav" => "wav",

            "image/png" => "png",
            "image/webp" => "webp",
            "image/gif" => "gif",
            "image/jpeg" | "image/jpg" => "jpg",

            _ => {
                if job.file_key.starts_with("image") {
                    "jpg"
                } else {
                    "mp3"
                }
            }
        };

        let final_key = format!("{}.{}", job.file_key, ext);
        let path = Path::new(&final_key);
        let parent = path.parent().unwrap_or_else(|| Path::new(""));
        let name = path.file_name().ok_or_else(|| anyhow::anyhow!("Invalid file name"))?;
        let tmp_key = parent.join("tmp").join(name).to_string_lossy().to_string();

        let mut file = self.disk.create_file(&tmp_key).await?;
        let mut stream = response.bytes_stream();

        while let Some(chunk_res) = stream.next().await {
            let chunk = chunk_res?;
            file.write_all(&chunk).await?;
        }

        file.flush().await?;
        drop(file);

        self.disk.rename_file(&tmp_key, &final_key).await?;

        debug!("Background download concluded: {}", final_key);
        Ok(())
    }
}
