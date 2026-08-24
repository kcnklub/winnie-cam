//! Capture backend for USB (UVC) webcams over V4L2 - used both for a real
//! USB webcam plugged into the Pi and, just as usefully, for developing on a
//! laptop with no CSI camera or `rpicam-vid` available at all.
//!
//! Unlike the Pi CSI camera, a UVC webcam exposes MJPEG directly through
//! V4L2: no external binary needed. In principle each dequeued buffer is
//! already one complete JPEG frame - but in practice, at least on the
//! webcam this was tested against, the driver's reported `bytesused` runs a
//! few bytes past the real end-of-image (stale bytes left over from a
//! previous capture into the same mmap buffer, since only a handful of
//! buffers are cycled through). So frames are run through the same
//! marker-aware [`JpegSplitter`] the rpicam backend uses, which finds the
//! real `FF D9` and discards whatever the driver tacked on after it.
//!
//! Config changes are detected via [`SharedVideoConfig::take_signal`]
//! between frames; when true the loop exits cleanly so [`supervise`]
//! restarts it with the new settings.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use anyhow::Context;
use tokio_util::sync::CancellationToken;
use v4l::buffer::Type;
use v4l::io::traits::CaptureStream;
use v4l::prelude::*;
use v4l::video::Capture;
use v4l::{Format, FourCC, Fraction};

use super::CaptureSource;
use crate::config::SharedVideoConfig;
use crate::hub::FrameHub;
use crate::jpeg::JpegSplitter;

/// Number of mmap buffers to allocate for the capture stream.
const BUFFER_COUNT: u32 = 4;

pub struct V4l2Capture {
    device: PathBuf,
    /// Runtime-changeable settings; checked between frames via
    /// [`SharedVideoConfig::take_signal`].
    video_config: Arc<SharedVideoConfig>,
}

impl V4l2Capture {
    pub fn from_config(video_config: Arc<SharedVideoConfig>) -> Self {
        Self {
            // The device path is the only capture-field not in
            // VideoSettings; it's fixed at startup.
            device: PathBuf::from("/dev/video0"),
            video_config,
        }
    }

    /// Sets the `device` path (called after construction by `capture::build`,
    /// which has the CLI [`Config`]).
    pub fn with_device(mut self, device: PathBuf) -> Self {
        self.device = device;
        self
    }

    async fn run_impl(&self, hub: FrameHub, shutdown: CancellationToken) -> anyhow::Result<()> {
        let device = self.device.clone();
        let video_config = Arc::clone(&self.video_config);

        // The v4l crate's I/O is blocking, so it gets its own thread rather
        // than stalling the tokio runtime.
        tokio::task::spawn_blocking(move || capture_loop(&device, &video_config, hub, shutdown))
            .await
            .context("v4l2 capture thread panicked")?
    }
}

impl CaptureSource for V4l2Capture {
    fn run<'a>(
        &'a self,
        hub: FrameHub,
        shutdown: CancellationToken,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(self.run_impl(hub, shutdown))
    }
}

/// Runs synchronously on a blocking thread: opens the device, negotiates
/// MJPEG at the current resolution/fps from `video_config`, and publishes
/// frames until `shutdown` is cancelled, the device errors out, or a config
/// change is signaled.
///
/// Cancellation is checked between frames rather than pre-empting an
/// in-flight dequeue, so shutdown latency is bounded by one frame interval
/// (tens of milliseconds at the configured fps) - acceptable for a graceful
/// shutdown, not for hard real-time cancellation.
fn capture_loop(
    device_path: &Path,
    video_config: &SharedVideoConfig,
    hub: FrameHub,
    shutdown: CancellationToken,
) -> anyhow::Result<()> {
    // --- read current settings ---
    // Snapshot once at the top so we don't have to re-read every iteration.
    let (width, height, fps) = {
        let s = video_config.settings.read().unwrap();
        (s.width, s.height, s.fps)
    };
    // Consume any stale signal that may have fired before we entered.
    video_config.take_signal();

    let dev = Device::with_path(device_path)
        .with_context(|| format!("opening {}", device_path.display()))?;

    let requested = Format::new(width, height, FourCC::new(b"MJPG"));
    let actual = dev
        .set_format(&requested)
        .context("negotiating MJPG capture format")?;
    anyhow::ensure!(
        actual.fourcc == FourCC::new(b"MJPG"),
        "{} does not support MJPG at {}x{} (driver offered {} instead)",
        device_path.display(),
        width,
        height,
        actual.fourcc
    );
    if actual.width != width || actual.height != height {
        tracing::warn!(
            requested = %format!("{width}x{height}"),
            actual = %format!("{}x{}", actual.width, actual.height),
            "device adjusted the requested resolution"
        );
    }

    let mut params = dev.params().context("querying stream parameters")?;
    params.interval = Fraction::new(1, fps.max(1));
    // Best-effort: not every device honors a requested frame interval, and
    // that's fine - we still capture whatever it actually produces.
    let _ = dev.set_params(&params);

    let mut stream = MmapStream::with_buffers(&dev, Type::VideoCapture, BUFFER_COUNT)
        .context("starting mmap capture stream")?;
    let mut splitter = JpegSplitter::new();

    while !shutdown.is_cancelled() {
        // Check for config changes between frames.
        if video_config.take_signal() {
            return Ok(());
        }
        let (buf, meta) = stream.next().context("dequeuing a frame")?;
        // `bytesused` is the driver's word for where this frame ends, but
        // isn't reliably exact (see module docs) - feed it through the
        // splitter rather than trusting it as-is.
        let len = (meta.bytesused as usize).min(buf.len());
        for frame in splitter.push(&buf[..len]) {
            hub.publish(frame);
        }
    }
    Ok(())
}
