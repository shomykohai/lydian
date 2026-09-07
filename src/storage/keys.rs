use std::borrow::Cow;
use std::path::Path;

use crate::models::SubsonicStreamFormat;
use crate::utils::mime::MediaType;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum StorageKey {
    Media {
        id: String,
        format: SubsonicStreamFormat,
    },
    CoverArt {
        id: String,
        // None = original size
        size: Option<u32>,
    },
    TranscodedMedia {
        id: String,
        format: SubsonicStreamFormat,
        bitrate: u32,
        offset: u32,
    },
}

impl StorageKey {
    const AUDIO_FALLBACK: &'static str = "mp3";
    const IMAGE_FALLBACK: &'static str = "jpg";

    pub fn to_file_name(&self) -> String {
        let sanitized = |id: &str| {
            Path::new(id)
                .file_name()
                .and_then(|os_str| os_str.to_str())
                .unwrap_or(id)
                .replace("..", "")
        };

        match self {
            Self::Media { id, format } => {
                let container = match format {
                    SubsonicStreamFormat::Raw | SubsonicStreamFormat::Flac => Cow::Borrowed("src"),
                    transcoded => Cow::Owned(transcoded.to_string()),
                };

                format!("media/{}_{}", sanitized(id), container)
            }
            Self::CoverArt { id, size } => size.as_ref().map_or_else(
                || format!("image/{}_orig", sanitized(id)),
                |size| format!("image/{}_{}", sanitized(id), size),
            ),
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

    pub fn default_mime(&self) -> &'static str {
        match self {
            Self::Media { .. } => MediaType::of(Self::AUDIO_FALLBACK),
            Self::CoverArt { .. } => MediaType::of(Self::IMAGE_FALLBACK),
            Self::TranscodedMedia { format, .. } => format.mime_type(),
        }
    }

    pub fn extension_for(file_key: &str, content_type: &str) -> &'static str {
        MediaType::extension(content_type).unwrap_or_else(|| {
            if file_key.starts_with("image") { Self::IMAGE_FALLBACK } else { Self::AUDIO_FALLBACK }
        })
    }
}
