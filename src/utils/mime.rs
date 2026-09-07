const TABLE: &[(&str, &str)] = &[
    ("mp3", "audio/mpeg"),
    ("flac", "audio/flac"),
    ("flac", "audio/x-flac"),
    ("ogg", "audio/ogg"),
    ("ogg", "application/ogg"),
    ("m4a", "audio/mp4"),
    ("m4a", "audio/x-m4a"),
    ("m4a", "audio/m4a"),
    ("wav", "audio/wav"),
    ("wav", "audio/x-wav"),
    ("jpg", "image/jpeg"),
    ("jpg", "image/jpg"),
    ("png", "image/png"),
    ("webp", "image/webp"),
    ("gif", "image/gif"),
];

pub struct MediaType;

impl MediaType {
    pub const UNKNOWN: &'static str = "application/octet-stream";

    pub fn extensions() -> impl Iterator<Item = &'static str> {
        TABLE
            .iter()
            .map(|(ext, _)| *ext)
            .enumerate()
            .filter_map(|(at, ext)| TABLE[..at].iter().all(|(seen, _)| *seen != ext).then_some(ext))
    }

    /// The canonical media type for an extension.
    pub fn of(extension: &str) -> &'static str {
        TABLE
            .iter()
            .find(|(ext, _)| ext.eq_ignore_ascii_case(extension))
            .map_or(Self::UNKNOWN, |(_, mime)| *mime)
    }

    pub fn extension(content_type: &str) -> Option<&'static str> {
        let content_type = content_type.split(';').next().unwrap_or_default().trim();

        TABLE.iter().find(|(_, mime)| mime.eq_ignore_ascii_case(content_type)).map(|(ext, _)| *ext)
    }
}
