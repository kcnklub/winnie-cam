//! Microphone capture backend, built on an `ffmpeg` subprocess.
//!
//! Same shape as [`crate::capture::rpicam`], and for the same reason: the
//! encoding work belongs to a tool that already does it well, so this module
//! only has to spawn it, read its stdout, and keep its lifetime tied to
//! ours. ffmpeg reads the ALSA device, encodes, and muxes; everything here
//! is plumbing.
//!
//! The ALSA device is opened exactly once no matter how many people are
//! listening - a capture device generally cannot be opened twice, so the
//! single-process-plus-fan-out arrangement is a requirement rather than an
//! optimization. See [`crate::audio::hub`].

use std::future::Future;
use std::pin::Pin;
use std::process::Stdio;

use anyhow::{Context, anyhow};
use bytes::Bytes;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

use super::AudioSource;
use super::hub::AudioHub;
use super::webm::{AudioChunk, WebmChunker};
use crate::config::{AudioFormat, Config};

/// The encoder. Unlike `rpicam-vid` this is not preinstalled on Pi OS, so
/// its absence is reported at startup rather than as a capture failure.
const FFMPEG_BINARY: &str = "ffmpeg";

/// Size of each stdout read. Audio is two orders of magnitude smaller than
/// video, so this only has to be comfortably larger than one cluster.
const READ_CHUNK_SIZE: usize = 16 * 1024;

/// Target length of a WebM cluster, in milliseconds. Doubles as the floor on
/// how late a joining listener's first audio can be, since a listener can
/// only be dropped into the stream on a cluster boundary.
const CLUSTER_TIME_LIMIT_MS: u32 = 100;

pub struct AlsaCapture {
    device: String,
    rate: u32,
    bitrate: u32,
    format: AudioFormat,
}

impl AlsaCapture {
    pub fn from_config(cfg: &Config) -> anyhow::Result<Self> {
        anyhow::ensure!(cfg.audio_rate > 0, "--audio-rate must be > 0");
        anyhow::ensure!(cfg.audio_bitrate > 0, "--audio-bitrate must be > 0");

        crate::capture::find_in_path(FFMPEG_BINARY).ok_or_else(|| {
            anyhow!(
                "--audio needs {FFMPEG_BINARY} on PATH (Debian/Pi OS: sudo apt install {FFMPEG_BINARY})"
            )
        })?;

        Ok(Self {
            device: cfg.audio_device.clone(),
            rate: cfg.audio_rate,
            bitrate: cfg.audio_bitrate,
            format: cfg.audio_format,
        })
    }

    /// Populates `cmd` with the arguments for the configured format.
    ///
    /// Channel count and sample rate are output options, not input ones, so
    /// ffmpeg opens the device in whatever format it natively supports and
    /// resamples - USB microphones are frequently stereo-only, or refuse
    /// 48kHz, and would otherwise fail to open at all.
    fn build_command(&self, cmd: &mut Command) {
        cmd.arg("-hide_banner")
            .arg("-loglevel")
            .arg("warning")
            .arg("-f")
            .arg("alsa")
            .arg("-i")
            .arg(&self.device)
            .arg("-ac")
            .arg("1")
            .arg("-ar")
            .arg(self.rate.to_string())
            .arg("-b:a")
            .arg(self.bitrate.to_string());

        match self.format {
            AudioFormat::WebmOpus => {
                cmd.arg("-c:a")
                    .arg("libopus")
                    .arg("-application")
                    .arg("voip")
                    .arg("-f")
                    .arg("webm")
                    .arg("-cluster_time_limit")
                    .arg(CLUSTER_TIME_LIMIT_MS.to_string());
            }
            AudioFormat::AdtsAac => {
                cmd.arg("-c:a").arg("aac").arg("-f").arg("adts");
            }
        }

        // Without this ffmpeg buffers before writing, which shows up as the
        // stream simply not starting for several seconds.
        cmd.arg("-flush_packets").arg("1").arg("pipe:1");
    }

    async fn run_impl(&self, hub: AudioHub, shutdown: CancellationToken) -> anyhow::Result<()> {
        let mut cmd = Command::new(FFMPEG_BINARY);
        self.build_command(&mut cmd);
        cmd.stdin(Stdio::null());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        cmd.kill_on_drop(true);

        let mut child = cmd
            .spawn()
            .with_context(|| format!("failed to spawn {FFMPEG_BINARY}"))?;
        let mut stdout = child.stdout.take().expect("stdout was piped");
        let stderr = child.stderr.take().expect("stderr was piped");

        // A wrong --audio-device, a mic already in use, or a missing libopus
        // build only ever shows up on stderr, so log it rather than discard
        // it - exactly as the rpicam backend does for libcamera.
        let stderr_task = tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::warn!(target: "ffmpeg", "{line}");
            }
        });

        let mut publisher = Publisher::new(self.format);
        let mut buf = [0u8; READ_CHUNK_SIZE];

        let result = loop {
            tokio::select! {
                _ = shutdown.cancelled() => break Ok(()),
                read = stdout.read(&mut buf) => {
                    match read {
                        Ok(0) => break Err(anyhow!("{FFMPEG_BINARY} exited (stdout closed)")),
                        Ok(n) => publisher.push(&hub, &buf[..n]),
                        Err(e) => {
                            break Err(anyhow::Error::from(e)
                                .context(format!("reading {FFMPEG_BINARY} stdout")));
                        }
                    }
                }
            }
        };

        // Whatever path got us here, make sure the child doesn't outlive us.
        let _ = child.start_kill();
        let _ = child.wait().await;
        stderr_task.abort();

        result
    }
}

impl AudioSource for AlsaCapture {
    fn run<'a>(
        &'a self,
        hub: AudioHub,
        shutdown: CancellationToken,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(self.run_impl(hub, shutdown))
    }
}

/// Turns raw ffmpeg stdout into hub publishes, which is the one place the
/// two formats genuinely differ: WebM has to be split so a late listener can
/// be replayed a header (see [`crate::audio::webm`]), while ADTS is
/// self-framing and passes straight through.
enum Publisher {
    Webm(WebmChunker),
    /// `true` once the stream has been announced to the hub.
    Adts(bool),
}

impl Publisher {
    fn new(format: AudioFormat) -> Self {
        match format {
            AudioFormat::WebmOpus => Publisher::Webm(WebmChunker::new()),
            AudioFormat::AdtsAac => Publisher::Adts(false),
        }
    }

    fn push(&mut self, hub: &AudioHub, data: &[u8]) {
        match self {
            Publisher::Webm(chunker) => {
                for chunk in chunker.push(data) {
                    match chunk {
                        AudioChunk::Init(bytes) => hub.start_stream(bytes),
                        AudioChunk::Cluster(bytes) => hub.publish(bytes),
                    }
                }
            }
            Publisher::Adts(started) => {
                // Announced on the first real bytes rather than at spawn, so
                // a listener is never told a stream exists before ffmpeg has
                // proven it can actually open the device.
                if !*started {
                    hub.start_stream(Bytes::new());
                    *started = true;
                }
                hub.publish(Bytes::copy_from_slice(data));
            }
        }
    }
}
