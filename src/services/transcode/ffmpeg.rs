use std::path::Path;
use std::process::Stdio;
use std::task::{Context, Poll};

use anyhow::{Result, anyhow};
use axum::body::Body;
use futures::TryStreamExt;
use futures::future::BoxFuture;
use tokio::io::AsyncWriteExt;
use tokio::process::{Child, ChildStdin, Command};
use tokio_util::io::ReaderStream;

use super::TranscodeRequest;
use crate::models::SubsonicStreamFormat;

/// Where ffmpeg reads from.
#[derive(Clone, Copy)]
enum Input<'a> {
    /// A file, which can be seeked.
    File(&'a Path),
    /// stdin, which can't be seeked.
    Pipe,
}

impl Input<'_> {
    const fn seekable(self) -> bool {
        matches!(self, Self::File(_))
    }
}

pub struct Ffmpeg;

impl Ffmpeg {
    pub async fn file(req: &TranscodeRequest) -> Result<Body> {
        let cmd = Self::command(Input::File(&req.file_path), req.format, req.bitrate, req.offset);

        Self::output(&mut Self::spawn(cmd).await?)
    }

    pub async fn pipe(
        input: Body,
        format: SubsonicStreamFormat,
        bitrate: u32,
        offset: u32,
    ) -> Result<Body> {
        let cmd = Self::command(Input::Pipe, format, bitrate, offset);
        let mut child = Self::spawn(cmd).await?;

        let Some(stdin) = child.stdin.take() else {
            return Err(anyhow!("Failed to open a pipe to the FFmpeg process"));
        };

        Self::feed(input, stdin);

        Self::output(&mut child)
    }

    fn command(
        input: Input<'_>,
        format: SubsonicStreamFormat,
        bitrate: u32,
        offset: u32,
    ) -> Command {
        let mut cmd = Command::new("ffmpeg");

        /* Hey ffmpeg, we don't need logs anyway */
        cmd.arg("-fflags").arg("nobuffer").arg("-loglevel").arg("error");

        if offset > 0 && input.seekable() {
            cmd.arg("-ss").arg(offset.to_string());
        }

        match input {
            Input::File(path) => cmd.arg("-i").arg(path),
            Input::Pipe => cmd.arg("-i").arg("pipe:0").stdin(Stdio::piped()),
        };

        if offset > 0 && !input.seekable() {
            cmd.arg("-ss").arg(offset.to_string());
        }

        /* Disable video (cover art counts!!) and metadata copying */
        cmd.arg("-vn").arg("-map_metadata").arg("-1");

        match format {
            SubsonicStreamFormat::Mp3 => {
                cmd.arg("-c:a").arg("libmp3lame").arg("-preset").arg("veryfast")
            }
            SubsonicStreamFormat::Aac => cmd.arg("-c:a").arg("aac").arg("-aac_coder").arg("fast"),
            SubsonicStreamFormat::Ogg => &mut cmd,
            /* No need to reencode if the input is already what we want */
            SubsonicStreamFormat::Raw | SubsonicStreamFormat::Flac => cmd.arg("-c:a").arg("copy"),
        };

        if bitrate > 0 && format.is_transcode() {
            cmd.arg("-b:a").arg(format!("{bitrate}k"));
        }

        cmd.arg("-f").arg(format.container()).arg("-").stdout(Stdio::piped()).stderr(Stdio::null());

        cmd
    }

    async fn spawn(mut cmd: Command) -> Result<Child> {
        tokio::task::spawn_blocking(move || cmd.spawn())
            .await?
            .map_err(|e| anyhow!("FFmpeg execution failed: {e}"))
    }

    fn output(child: &mut Child) -> Result<Body> {
        let Some(stdout) = child.stdout.take() else {
            return Err(anyhow!("Failed to capture stdout from FFmpeg process"));
        };

        Ok(Body::from_stream(ReaderStream::new(stdout)))
    }

    /// Hi ffmpeg, here's some data to transcode. Thanks!
    fn feed(input: Body, mut stdin: ChildStdin) {
        tokio::spawn(async move {
            let mut source = input.into_data_stream();

            while let Ok(Some(chunk)) = source.try_next().await {
                if stdin.write_all(&chunk).await.is_err() {
                    break;
                }
            }

            let _ = stdin.shutdown().await;
        });
    }
}

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
        Box::pin(async move { Ffmpeg::file(&req).await })
    }
}
