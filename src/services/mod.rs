mod cache;
mod library_migration;
mod remote;
mod stale_migrations_cleanup;
mod stream;
pub mod transcode;

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
pub use library_migration::*;
use log::{debug, info, warn};
pub use remote::*;
use reqwest::Client;
use thiserror::Error as TError;
use tokio::sync::{Mutex, OwnedSemaphorePermit, RwLock, Semaphore, mpsc};
use tokio::task::JoinHandle;
use tower::ServiceExt;
pub use transcode::TranscodeRequest;

use crate::AppState;
use crate::config::LydianConfig;
use crate::services::cache::CacheCleanupService;
use crate::services::stale_migrations_cleanup::StaleMigrationsCleanupService;
use crate::services::stream::{DiskStreamRequest, DiskStreamService, ReadyBody};
use crate::services::transcode::ffmpeg::FfmpegService;
use crate::services::transcode::transcoding::TranscodeService;
use crate::storage::DiskDriver;
use crate::workers::download::{DownloadJob, DownloadManager, DownloadSource};

#[derive(Debug, TError)]
pub enum ServiceError {
    #[error("Remote service: {0}")]
    Remote(#[from] remote::RemoteError),
    #[error("Failed to migrate file from {from} to {to}: {source}")]
    Migration {
        from: String,
        to: String,
        #[source]
        source: anyhow::Error,
    },
    #[error("Audio tagging error: {0}")]
    TaggingError(#[from] audiotags::Error),
    #[error("Task join error: {0}")]
    TaskJoinError(#[from] tokio::task::JoinError),
    #[error("Database error: {0}")]
    DatabaseError(#[from] sqlx::Error),
    #[error("Early end of stream")]
    EarlyEndOfStream,
}

pub type Transcoder = TranscodeService<DiskStreamService<FfmpegService>>;

/// Don't let the client wait forever!!
const TRANSCODE_QUEUE_WAIT: Duration = Duration::from_secs(15);

pub struct ServiceManager {
    transcode_slots: Arc<Semaphore>,
    cache_tee: DiskStreamService<ReadyBody>,
    download_queue: RwLock<Option<mpsc::UnboundedSender<DownloadJob>>>,
    worker_tasks: Arc<Mutex<Vec<JoinHandle<()>>>>,
    pub transcode: Transcoder,
}

impl ServiceManager {
    pub fn init(cfg: &LydianConfig, http_client: Client, disk: Arc<DiskDriver>) -> Self {
        let (tx, handles) = DownloadManager::init(cfg, http_client, disk.clone());

        let cache_tee = DiskStreamService::new(ReadyBody, disk.clone());
        let transcode_service = DiskStreamService::new(FfmpegService, disk.clone());
        let transcode_service = TranscodeService::new(transcode_service, disk);

        Self {
            transcode_slots: Arc::new(Semaphore::new(cfg.transcoding.max_concurrent_transcodes)),
            cache_tee,
            download_queue: RwLock::new(Some(tx)),
            worker_tasks: Arc::new(Mutex::new(handles)),
            transcode: transcode_service,
        }
    }

    pub async fn start_services(&self, state: Arc<AppState>) {
        info!("Starting background services...");

        CacheCleanupService::start(state.clone());
        StaleMigrationsCleanupService::new(state).oneshot(()).await.ok();

        debug!("All background services started successfully.");
    }

    pub fn queue_download(&self, file_key: String, source: DownloadSource) {
        self.enqueue(DownloadJob { file_key, source, complete_tx: None });
    }

    pub fn queue_download_with_completion(
        &self,
        file_key: String,
        source: DownloadSource,
        tx: Arc<tokio::sync::watch::Sender<bool>>,
    ) {
        self.enqueue(DownloadJob { file_key, source, complete_tx: Some(tx) });
    }

    /// Hacky way to save 2x the space
    pub async fn cache_stream(&self, body: Body, cache_name: String) -> Option<Body> {
        self.cache_tee.clone().oneshot(DiskStreamRequest { inner_req: body, cache_name }).await.ok()
    }

    /// Returns a semaphore to transcode a track, or None if the queue is full.
    pub async fn transcode_slot(&self) -> Option<OwnedSemaphorePermit> {
        let slots = self.transcode_slots.clone();

        tokio::time::timeout(TRANSCODE_QUEUE_WAIT, slots.acquire_owned()).await.ok()?.ok()
    }

    fn enqueue(&self, job: DownloadJob) {
        let Ok(lock) = self.download_queue.try_read() else {
            warn!("Download queue is shutting down; dropping the job for '{}'", job.file_key);
            if let Some(tx) = job.complete_tx {
                let _ = tx.send(true);
            }
            return;
        };

        let Some(tx) = &*lock else {
            if let Some(tx) = job.complete_tx {
                let _ = tx.send(true);
            }
            return;
        };

        let _ = tx.send(job);
    }

    pub async fn shutdown(&self) {
        info!("Stopping background workers and services...");

        let mut queue_lock = self.download_queue.write().await;
        let active_tx = queue_lock.take();
        drop(active_tx);
        drop(queue_lock);

        let mut tasks_lock = self.worker_tasks.lock().await;
        let active_handles = std::mem::take(&mut *tasks_lock);
        drop(tasks_lock);

        for handle in active_handles {
            let _ = handle.await;
        }
    }
}
