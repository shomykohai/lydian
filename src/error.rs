use thiserror::Error as TError;

#[derive(TError, Debug)]
pub enum Error {
    #[error("Subsonic error: {0}")]
    Subsonic(#[from] crate::models::SubsonicErrorCode),

    #[error("Config error: {0}")]
    ConfigError(#[from] config::ConfigError),

    #[error("Service error: {0}")]
    ServiceError(#[from] crate::services::ServiceError),

    #[error("IO storage error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Internal error: {0}")]
    Internal(String),
}

