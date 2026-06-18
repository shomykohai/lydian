use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum StorageKey {
    Media { id: String },
    CoverArt { id: String, size: u32 },
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
            Self::Media { id } => format!("media/{}", sanitized(id)),
            Self::CoverArt { id, size } => format!("image/{}_{}", sanitized(id), size),
        }
    }

    pub const fn default_mime(&self) -> &'static str {
        match self {
            Self::Media { .. } => "audio/mpeg",
            Self::CoverArt { .. } => "image/jpeg",
        }
    }
}
