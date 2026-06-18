use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use config::{Config, Environment, File};
use ipnetwork::IpNetwork;
use log::{LevelFilter, info, warn};
use serde::{Deserialize, Serialize};

use crate::logger;
use crate::providers::{MusicProvider, QobuzDlClient, SubsonicProvider};

#[derive(Debug, Deserialize, Serialize)]
pub struct ServerConfig {
    /// The host address to bind the server to.
    pub host: String,
    /// The port to bind the server to.
    pub port: u16,
    /// The log level to use for the server.
    #[serde(with = "logger")]
    pub log_level: LevelFilter,
    /// The database URL to use for the server.
    pub database_url: String,
    #[serde(default)]
    pub trusted_proxies: Vec<IpNetwork>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct SubsonicConfig {
    /// The URL of the origin Subsonic server.
    pub url: String,
    /// The username to use for authentication with the Subsonic server.
    /// Note: This must be an admin user for allowing library scanning.
    pub username: Option<String>,
    /// The password to use for authentication with the Subsonic server.
    pub password: Option<String>,
    /// The template with which the downloaded files will be organized
    /// in the music folder. The template can contain the following placeholders:
    /// - {artist}: The artist name
    /// - {album}: The album name
    /// - {track}: The track number
    /// - {title}: The track title
    pub folder_template: String,
    /// The folder where the music files will be stored.
    /// This is the Subsonic music folder.
    pub music_folder: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct DownloadConfig {
    /// The path where downloaded files will be stored
    /// Note: This is where temp files will be stored.
    /// For permanent storage, use the subsonic.music_folder config.
    pub download_path: String,
    /// The maximum number of concurrent download workers.
    /// Default is 4.
    pub max_workers: usize,
    /// The quality of the downloaded files.
    /// Defaults to "flac". Currently not used.
    pub quality: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct CacheConfig {
    /// Time to live for in memory cache in seconds
    pub ttl_seconds: u64,
    /// The maximum size of the in memory cache in bytes.
    /// If None, the cache size is defaulted to 100 MB.
    pub max_cache_size: Option<u64>,
    /// Time to live for database cache in seconds
    pub db_ttl_seconds: u64,
    /// Time to live of the temp files in seconds.
    /// After this time, a temp file will be deleted.
    pub temp_file_ttl_seconds: u64,
    /// The interval in seconds at which the temp files
    /// cleanup service will run. Default is 30 minutes (1800 seconds).
    pub cleanup_service_interval_seconds: u64,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct LyricsConfig {
    /// Whether to enable lyrics fetching
    pub enable: bool,
    /// The default provider to use for lyrics fetching
    pub default_provider: String,
    /// Whether to force the use of the default provider
    pub force_provider: bool,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct SubsonicProviderConfig {
    /// The URL of the External (Open) Subsonic server.
    pub url: String,
    /// The username to use for authenticating with the External Subsonic server.
    pub username: Option<String>,
    /// The password to use for authenticating with the External Subsonic server.
    pub password: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct QobuzDlProviderConfig {
    /// The URL of the Qobuz DL API.
    pub url: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ProvidersConfig {
    #[serde(default)]
    pub subsonic: HashMap<String, SubsonicProviderConfig>,

    #[serde(default)]
    pub qobuz: HashMap<String, QobuzDlProviderConfig>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct LydianConfig {
    pub server: ServerConfig,
    pub subsonic: SubsonicConfig,
    pub cache: CacheConfig,
    pub download: DownloadConfig,
    pub lyrics: LyricsConfig,
    pub providers: ProvidersConfig,
}

impl LydianConfig {
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let _env = std::env::var("LYDIAN_ENV").unwrap_or_else(|_| "development".to_string());

        let config = Config::builder()
            .set_default("server.host", "localhost")?
            .set_default("server.port", 8080)?
            .set_default("server.log_level", "info")?
            .set_default("server.database_url", "sqlite://lydian.db")?
            .set_default("subsonic.url", "http://localhost:4533")?
            .set_default("subsonic.username", "")?
            .set_default("subsonic.password", "")?
            .set_default("subsonic.folder_template", "{artist}/{album}/{track} - {title}")?
            .set_default("subsonic.music_folder", "/app/music")?
            .set_default("cache.ttl_seconds", 3600)?
            .set_default("cache.max_cache_size", Some(100 * 1024 * 1024))?
            .set_default("cache.db_ttl_seconds", 86400)?
            .set_default("cache.temp_file_ttl_seconds", 10800)?
            .set_default("cache.cleanup_service_interval_seconds", 1800)?
            .set_default("download.download_path", "/app/downloads")?
            .set_default("download.max_workers", 4)?
            .set_default("download.quality", "flac")?
            .set_default("lyrics.enable", true)?
            .set_default("lyrics.default_provider", "lrclib")?
            .set_default("lyrics.force_provider", false)?
            .add_source(File::from(path.as_ref()).required(false))
            .add_source(Environment::with_prefix("LYDIAN").separator("_"))
            .build()?;

        Ok(config.try_deserialize::<Self>()?)
    }

    pub fn load() -> Result<Self> {
        if let Ok(config_path) = std::env::var("LYDIAN_CONFIG") {
            return Self::load_from_file(config_path);
        }

        if Path::new("config.toml").exists() {
            return Self::load_from_file("config.toml");
        }

        anyhow::bail!("No config available!");
    }
}

impl ProvidersConfig {
    pub fn build(&self, http_client: &reqwest::Client) -> Result<Vec<Arc<dyn MusicProvider>>> {
        let mut remotes: Vec<Arc<dyn MusicProvider>> = Vec::new();

        for (provider_name, subsonic_cfg) in &self.subsonic {
            let username = subsonic_cfg.username.clone().unwrap_or_default();
            let password = subsonic_cfg.password.clone().unwrap_or_default();

            info!("Registering Subsonic remote [{}] at {}", provider_name, subsonic_cfg.url);

            let subson = SubsonicProvider::new(
                http_client.clone(),
                &subsonic_cfg.url,
                &username,
                &password,
                provider_name,
            )?;

            remotes.push(Arc::new(subson));
        }

        for (provider_name, qobuz_cfg) in &self.qobuz {
            info!("Registering Qobuz remote [{}] at {}", provider_name, qobuz_cfg.url);

            let qobuz = QobuzDlClient::new(http_client.clone(), &qobuz_cfg.url, provider_name);
            remotes.push(Arc::new(qobuz));
        }

        if remotes.is_empty() {
            warn!("No remotes configured! Please add at least one remote provider to the config.");
        }

        Ok(remotes)
    }
}
