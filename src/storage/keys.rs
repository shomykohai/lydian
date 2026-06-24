use std::path::Path;

use crate::models::SubsonicStreamFormat;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum StorageKey {
    Media { id: String, format: SubsonicStreamFormat },
    CoverArt { id: String, size: u32 },
    TranscodedMedia { id: String, format: SubsonicStreamFormat, bitrate: u32, offset: u32 },
}

impl StorageKey {
    pub fn to_file_name(&self) -> String {
        let sanitized = |id: &str| {
            Path::new(id)
                .file_name()
                .and_then(|os_str| os_str.to_str())
                .unwrap_or(id)
                .replace("..", "")
        };

        match self {
            Self::Media { id, format } => format!("media/{}_{}", sanitized(id), format),
            Self::CoverArt { id, size } => format!("image/{}_{}", sanitized(id), size),
            Self::TranscodedMedia { id, format, bitrate, offset } => {
                format!(
                    "cache/transcodes/{}_{}_{}_{}.{}",
                    sanitized(id),
                    format,
                    bitrate,
                    offset,
                    format
                )
            }
        }
    }

    pub const fn default_mime(&self) -> &'static str {
        match self {
            Self::Media { .. } => "audio/mpeg",
            Self::CoverArt { .. } => "image/jpeg",
            Self::TranscodedMedia { format, .. } => format.mime_type(),
        }
    }
}
