use std::process::Stdio;
use std::task::{Context, Poll};

use anyhow::anyhow;
use axum::body::Body;
use futures::future::{BoxFuture, err, ready};
use tokio::process::Command;
use tokio_util::io::ReaderStream;

use super::TranscodeRequest;
use crate::models::SubsonicStreamFormat;

#[derive(Clone, Default)]
pub struct FfmpegService;

impl tower::Service<TranscodeRequest> for FfmpegService {
    type Error = anyhow::Error;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;
    type Response = Body;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: TranscodeRequest) -> Self::Future {
        let mut cmd = Command::new("ffmpeg");

        cmd.arg("-fflags").arg("nobuffer");

        /* Hey ffmpeg, we don't need logs anyway */
        cmd.arg("-loglevel").arg("error");

        /* Seek to offset */
        if req.offset > 0 {
            cmd.arg("-ss").arg(req.offset.to_string());
        }

        cmd.arg("-i").arg(&req.file_path);

        /* Disable video (cover art counts!!) */
        cmd.arg("-vn");

        /* Disable metadata copying */
        cmd.arg("-map_metadata").arg("-1");

        if req.bitrate > 0 {
            cmd.arg("-b:a").arg(format!("{}k", req.bitrate));
        }

        match req.format {
            SubsonicStreamFormat::Mp3 => {
                cmd.arg("-c:a").arg("libmp3lame").arg("-preset").arg("veryfast");
            }
            SubsonicStreamFormat::Aac => {
                cmd.arg("-c:a").arg("aac").arg("-aac_coder").arg("fast");
            }
            _ => {}
        }

        cmd.arg("-f")
            .arg(req.format.to_string())
            .arg("-")
            .stdout(Stdio::piped())
            .stderr(Stdio::null());

        match cmd.spawn() {
            Ok(mut child) => {
                let Some(stdout) = child.stdout.take() else {
                    return Box::pin(err(anyhow!("Failed to capture stdout from FFmpeg process")));
                };
                let stream = ReaderStream::new(stdout);
                Box::pin(ready(Ok(Body::from_stream(stream))))
            }
            Err(e) => {
                log::error!("FFmpeg execution failed: {}", e);
                Box::pin(ready(Err(anyhow!("FFmpeg execution failed: {}", e))))
            }
        }
    }
}
