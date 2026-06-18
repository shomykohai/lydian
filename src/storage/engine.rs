use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use log::info;
use moka::future::Cache;
use serde_json::Value;
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use tokio::fs::create_dir_all;

use crate::config::LydianConfig;
use crate::storage::disk::DiskDriver;

pub struct StorageEngine {
    /* In memory cache, shorter TTL */
    pub cache: Arc<Cache<String, Value>>,
    /* Database cache, longer TTL */
    pub db: SqlitePool,
    /* Disk cache for storing covert arts etc... */
    pub disk: Arc<DiskDriver>,
}

impl StorageEngine {
    pub async fn new(cfg: &LydianConfig) -> Result<Self> {
        info!("Initializing storage...");

        let disk = Arc::new(DiskDriver::new(&cfg.download.download_path).await?);

        let sub_path = PathBuf::from(cfg.subsonic.music_folder.clone());

        // Subsonic server should create the folder for us, but just to be sure
        // we create it here.
        if !sub_path.exists() {
            create_dir_all(&sub_path).await?;
        }

        let cache = Arc::new(
            Cache::builder().time_to_live(Duration::from_secs(cfg.cache.ttl_seconds)).build(),
        );

        let db_url = &cfg.server.database_url;
        let db_opt = SqliteConnectOptions::new()
            .filename(db_url.strip_prefix("sqlite://").unwrap_or(db_url))
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal);

        let db = SqlitePoolOptions::new().max_connections(10).connect_with(db_opt).await?;

        info!("Running database migrations...");
        sqlx::migrate!("./migrations").run(&db).await?;

        Ok(Self { cache, db, disk })
    }

    pub async fn close(&self) -> Result<()> {
        self.db.close().await;
        Ok(())
    }
}
