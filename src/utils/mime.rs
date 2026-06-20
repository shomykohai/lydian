const MIME_TABLE: &[(&str, &str)] = &[
    ("mp3", "audio/mpeg"),
    ("flac", "audio/flac"),
    ("ogg", "audio/ogg"),
    ("m4a", "audio/mp4"),
    ("wav", "audio/wav"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("png", "image/png"),
    ("webp", "image/webp"),
    ("gif", "image/gif"),
];

#[allow(dead_code)]
pub fn ext_to_mime(ext: &str) -> &'static str {
    MIME_TABLE
        .iter()
        .find(|(e, _)| *e == ext)
        .map(|(_, mime)| *mime)
        .unwrap_or("application/octet-stream")
}

#[allow(dead_code)]
pub fn mime_to_ext(mime: &str) -> &'static str {
    MIME_TABLE.iter().find(|(_, m)| *m == mime).map(|(ext, _)| *ext).unwrap_or_else(|| "jpg")
}
