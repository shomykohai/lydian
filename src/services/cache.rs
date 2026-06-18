use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use log::{debug, error, info, warn};
use tokio::fs::{read_dir, remove_file};
use tokio::time::interval;

use crate::AppState;

pub struct CacheCleanupService;

impl CacheCleanupService {
    pub fn start(state: Arc<AppState>) {
        tokio::spawn(async move {
            let mut interval =
                interval(Duration::from_secs(state.config.cache.cleanup_service_interval_seconds));

            let ttl = Duration::from_secs(state.config.cache.temp_file_ttl_seconds);
            let path = &state.config.download.download_path;

            info!(
                "Starting background cache cleanup service. Target path: {}, TTL: {}s",
                path, state.config.cache.temp_file_ttl_seconds
            );

            loop {
                interval.tick().await;
                debug!("Triggering periodic temp file cleanup loop...");
                if let Err(e) = Self::cleanup_temp_dir(path, ttl).await {
                    error!("Error during temp file cache cleanup: {}", e);
                }
            }
        });
    }

    async fn cleanup_temp_dir(dir_path: &str, ttl: Duration) -> std::io::Result<()> {
        let start_path = PathBuf::from(dir_path);
        if !start_path.exists() || !start_path.is_dir() {
            return Ok(());
        }

        let mut stack = vec![start_path];
        let now = SystemTime::now();

        while let Some(current_dir) = stack.pop() {
            let mut entries = match read_dir(&current_dir).await {
                Ok(e) => e,
                Err(e) => {
                    warn!("Failed to read directory {:?}: {}", current_dir, e);
                    continue;
                }
            };

            while let Some(entry) = entries.next_entry().await? {
                let entry_path = entry.path();
                let Ok(metadata) = entry.metadata().await else {
                    continue;
                };

                if metadata.is_dir() {
                    stack.push(entry_path);
                } else if metadata.is_file() {
                    let file_time = metadata
                        .accessed()
                        .or_else(|_| metadata.modified())
                        .or_else(|_| metadata.created())
                        .unwrap_or(now);

                    if let Ok(duration) = now.duration_since(file_time)
                        && duration > ttl {
                            match remove_file(&entry_path).await {
                                Ok(_) => debug!("Removed expired temp file: {:?}", entry_path),
                                Err(e) => warn!(
                                    "Failed to remove expired temp file {:?}: {}",
                                    entry_path, e
                                ),
                            }
                        }
                }
            }
        }

        Ok(())
    }
}
