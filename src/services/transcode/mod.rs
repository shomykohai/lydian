pub mod ffmpeg;
pub mod transcoding;

use std::path::PathBuf;

use crate::models::SubsonicStreamFormat;

#[derive(Debug, Clone)]
pub struct TranscodeRequest {
    pub id: String,
    pub file_path: PathBuf,
    pub format: SubsonicStreamFormat,
    pub bitrate: u32,
    pub offset: u32,
}
