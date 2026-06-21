mod config;
mod error;
mod logger;
mod middleware;
mod models;
mod providers;
mod routes;
mod services;
mod storage;
mod utils;
mod workers;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use axum::serve;
pub use error::Error;
use log::{debug, error, info};
use moka::future::Cache;
use serde_json::Value;
#[cfg(feature = "jemalloc")]
use tikv_jemallocator::Jemalloc;
use tokio::net::TcpListener;
use tokio::select;
use tokio::signal::unix::{SignalKind, signal};

use crate::config::LydianConfig;
use crate::logger::init_logger;
use crate::providers::MusicProvider;
use crate::routes::AppRouter;
use crate::services::ServiceManager;
use crate::storage::engine::StorageEngine;

#[cfg(feature = "jemalloc")]
#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

const LOGO: &str = include_str!("logo.txt");
const VERSION: &str = env!("CARGO_PKG_VERSION");
const SUBSONIC_API_VERSION: &str = "1.16.1";

pub struct AppState {
    pub config: LydianConfig,
    pub http_client: reqwest::Client,
    pub remotes: Vec<Arc<dyn MusicProvider>>,
    pub storage: Arc<StorageEngine>,
    pub cache: Arc<Cache<String, Value>>,
    pub services: ServiceManager,
}

impl AppState {
    pub async fn init(config: LydianConfig) -> Result<Arc<Self>> {
        let http_client = reqwest::Client::builder()
            .user_agent(format!("Lydian/{}", VERSION))
            .tcp_nodelay(true)
            .connect_timeout(Duration::from_secs(10))
            .pool_idle_timeout(Duration::from_secs(90))
            .tcp_keepalive(Some(Duration::from_secs(60)))
            .http2_keep_alive_interval(Some(Duration::from_secs(30)))
            .http2_keep_alive_timeout(Duration::from_secs(5))
            .http2_keep_alive_while_idle(false)
            .use_rustls_tls()
            .build()?;

        let storage_engine = StorageEngine::new(&config).await?;
        let cache = Arc::new(
            Cache::builder()
                .time_to_live(Duration::from_secs(config.cache.ttl_seconds))
                .max_capacity(10000)
                .build(),
        );

        let services =
            ServiceManager::init(&config, http_client.clone(), storage_engine.disk.clone());

        let remotes = config.providers.build(&http_client)?;

        Ok(Arc::new(Self {
            config,
            http_client,
            remotes,
            storage: Arc::new(storage_engine),
            cache,
            services,
        }))
    }

    pub fn get_remote_by_id(&self, id: &str) -> Option<Arc<dyn MusicProvider>> {
        self.remotes.iter().find(|r| r.id() == id).cloned()
    }

    pub fn queue_download(&self, file_key: String, source_url: impl ToString) {
        self.services.queue_download(file_key, source_url);
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let config = LydianConfig::load()?;

    init_logger(config.server.log_level);

    println!("{}", LOGO.replace("{VERSION}", VERSION));

    let state = AppState::init(config).await?;

    state.services.start_services(state.clone()).await;

    let app = AppRouter::init(state.clone()).await?;

    let addr: SocketAddr =
        format!("{}:{}", state.config.server.host, state.config.server.port).parse()?;

    info!("Listening on {}", addr);

    let listener = TcpListener::bind(&addr).await?;
    let server = serve(listener, app);

    /* Axum / Hyper might keep client connections opens,
     * but we don't really want to wait for them to close,
     * since we're shutting down the server anyway, and
     * new requests will be rejected.
     * So, better than having an infinite wait, just let
     * the client handle themselves
     */
    tokio::select! {
        res = server => {
            if let Err(e) = res {
                error!("Web server execution error: {}", e);
            }
        }
        _ = shutdown_signal() => {}
    }

    /* SHUTDOWN BELOW!! */

    state.services.shutdown().await;

    /* Wait for database and storage operations to complete */
    state.storage.close().await?;

    info!("See ya!");

    Ok(())
}

async fn shutdown_signal() {
    let mut sigterm = signal(SignalKind::terminate()).unwrap();
    let mut sigint = signal(SignalKind::interrupt()).unwrap();

    select! {
        _ = sigint.recv() => debug!("Received SIGINT."),
        _ = sigterm.recv() => debug!("Received SIGTERM."),
    };

    info!("Shutting down...");
}
