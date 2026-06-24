mod cache;
mod library_migration;
mod remote;
mod stale_migrations_cleanup;
mod stream;
mod transcode;

use std::sync::Arc;

pub use library_migration::*;
use log::{debug, info};
pub use remote::*;
use reqwest::Client;
use thiserror::Error as TError;
use tokio::sync::{Mutex, mpsc};
use tokio::task::JoinHandle;
use tower::limit::ConcurrencyLimit;
use tower::{ServiceBuilder, ServiceExt};
pub use transcode::TranscodeRequest;

use crate::AppState;
use crate::config::LydianConfig;
use crate::services::cache::CacheCleanupService;
use crate::services::stale_migrations_cleanup::StaleMigrationsCleanupService;
use crate::services::stream::DiskStreamService;
use crate::services::transcode::ffmpeg::FfmpegService;
use crate::services::transcode::transcoding::TranscodeService;
use crate::storage::DiskDriver;
use crate::workers::download::{DownloadJob, DownloadManager};

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

pub type Transcoder = ConcurrencyLimit<TranscodeService<DiskStreamService<FfmpegService>>>;

pub struct ServiceManager {
    download_queue: Mutex<Option<mpsc::UnboundedSender<DownloadJob>>>,
    worker_tasks: Arc<Mutex<Vec<JoinHandle<()>>>>,
    pub transcode: Transcoder,
}

impl ServiceManager {
    pub fn init(cfg: &LydianConfig, http_client: Client, disk: Arc<DiskDriver>) -> Self {
        let (tx, handles) = DownloadManager::init(cfg, http_client, disk.clone());

        let transcode_service = DiskStreamService::new(FfmpegService, disk.clone());

        let transcode_service = TranscodeService::new(transcode_service, disk);

        let transcode_service = ServiceBuilder::new()
            .concurrency_limit(cfg.transcoding.max_concurrent_transcodes)
            .service(transcode_service);

        Self {
            download_queue: Mutex::new(Some(tx)),
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

    pub fn queue_download(&self, file_key: String, source_url: impl ToString) {
        let job = DownloadJob { file_key, source_url: source_url.to_string(), complete_tx: None };

        if let Ok(lock) = self.download_queue.try_lock()
            && let Some(tx) = &*lock
        {
            let _ = tx.send(job);
        }
    }

    pub fn queue_download_with_completion(
        &self,
        file_key: String,
        source_url: impl ToString,
        tx: Arc<tokio::sync::watch::Sender<bool>>,
    ) {
        let job =
            DownloadJob { file_key, source_url: source_url.to_string(), complete_tx: Some(tx) };

        if let Ok(lock) = self.download_queue.try_lock()
            && let Some(tx) = &*lock
        {
            let _ = tx.send(job);
        }
    }

    pub async fn shutdown(&self) {
        info!("Stopping background workers and services...");

        let mut queue_lock = self.download_queue.lock().await;
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
