use std::net::SocketAddr;
use std::path::PathBuf;

use clap::{Parser, ValueEnum};

/// A simple LAN baby-monitor camera server: forwards a camera's video to any
/// browser on the network as an MJPEG stream.
#[derive(Parser, Debug, Clone)]
#[command(name = "winnie-cam", version, about)]
pub struct Config {
    /// Which capture backend to use. `auto` uses the Pi CSI camera (via
    /// rpicam-vid/libcamera-vid) if found on PATH, otherwise falls back to a
    /// USB webcam over V4L2.
    #[arg(long, value_enum, default_value_t = SourceKind::Auto)]
    pub source: SourceKind,

    /// V4L2 device to use for the `v4l2` backend. Ignored for `rpicam`.
    #[arg(long, default_value = "/dev/video0")]
    pub device: PathBuf,

    #[arg(long, default_value_t = 1280)]
    pub width: u32,

    #[arg(long, default_value_t = 720)]
    pub height: u32,

    #[arg(long, default_value_t = 15)]
    pub fps: u32,

    /// JPEG quality, 1-100. Only affects the `rpicam` backend - a USB
    /// webcam's MJPEG is already encoded on-device by the time it arrives.
    #[arg(long, default_value_t = 80)]
    pub quality: u8,

    /// Flip the image horizontally. Useful once the camera is actually
    /// mounted wherever it ends up (e.g. on a crib rail).
    #[arg(long)]
    pub hflip: bool,

    /// Flip the image vertically.
    #[arg(long)]
    pub vflip: bool,

    /// Address to bind the HTTP server to. Defaults to all interfaces so
    /// the stream is reachable from other devices on the LAN.
    #[arg(long, default_value = "0.0.0.0:8080")]
    pub bind: SocketAddr,

    /// Enable person detection *and* motion events. Off by default:
    /// inference is by far the most expensive thing this program can do,
    /// and video capture must never be affected by it. There is no separate
    /// switch for motion - it's derived from the person box, so it comes
    /// along with `--detect` automatically. See `--model` and the
    /// `--motion-*` flags below to tune it.
    #[arg(long, requires = "model")]
    pub detect: bool,

    /// Path to a YOLOv8/YOLO11 ONNX model, exported with a fixed square
    /// input and without built-in NMS (`yolo export ... dynamic=False` -
    /// see the README's model export section). Required by `--detect`.
    #[arg(long)]
    pub model: Option<PathBuf>,

    /// Model input side length in pixels. Must match the size the model
    /// was exported with, or loading fails at startup.
    #[arg(long, default_value_t = 320)]
    pub detect_size: u32,

    /// Detection passes per second while at least one viewer has the
    /// overlay open.
    #[arg(long, default_value_t = 1.0)]
    pub detect_fps: f32,

    /// Detection passes per second when nobody is watching the overlay.
    /// Detection keeps running at a low rate rather than stopping, so
    /// there's a continuous signal to build alerting on later - but slowly,
    /// since sustained inference is the Pi's dominant heat source.
    #[arg(long, default_value_t = 0.2)]
    pub detect_idle_fps: f32,

    /// Minimum confidence to report a detection, 0.0-1.0.
    #[arg(long, default_value_t = 0.35)]
    pub detect_threshold: f32,

    /// IoU threshold for non-maximum suppression, 0.0-1.0.
    #[arg(long, default_value_t = 0.45)]
    pub detect_iou: f32,

    /// Motion samples per second. Runs independently of `--detect-fps`/
    /// `--detect-idle-fps` - motion diffs raw frames, it doesn't run the
    /// model - but only produces events while a fresh-enough person box
    /// exists (see `--motion-box-ttl-ms`).
    #[arg(long, default_value_t = 2.0)]
    pub motion_fps: f32,

    /// Side length of the grid motion is diffed on (grid_side x grid_side
    /// grayscale cells). Larger catches finer movement but costs more CPU.
    #[arg(long, default_value_t = 64)]
    pub motion_grid: u32,

    /// Minimum grayscale change (0-255) for a grid cell to count as
    /// "changed" between samples.
    #[arg(long, default_value_t = 12)]
    pub motion_pixel_delta: u8,

    /// Fraction of cells inside the person box that must have changed for a
    /// sample to count as motion, 0.0-1.0.
    #[arg(long, default_value_t = 0.08)]
    pub motion_threshold: f32,

    /// How long motion must persist, in milliseconds, before a
    /// `motion_started` event fires.
    #[arg(long, default_value_t = 700)]
    pub motion_sustain_ms: u64,

    /// How long stillness must persist, in milliseconds, before a
    /// `motion_stopped` event fires.
    #[arg(long, default_value_t = 4000)]
    pub motion_quiet_ms: u64,

    /// Minimum gap, in milliseconds, between a `motion_stopped` event and
    /// the next `motion_started` - keeps a score hovering near the
    /// threshold from machine-gunning events.
    #[arg(long, default_value_t = 3000)]
    pub motion_cooldown_ms: u64,

    /// Ignore person boxes older than this many milliseconds when deciding
    /// whether there's a subject to diff motion against. Defaults to twice
    /// `--detect-idle-fps`'s ~5s period, so a box survives one missed
    /// detection pass.
    #[arg(long, default_value_t = 10000)]
    pub motion_box_ttl_ms: u64,

    /// Dilate each person box by this fraction of its own width/height
    /// before masking, so a limb moving just outside the last known box
    /// still counts.
    #[arg(long, default_value_t = 0.10)]
    pub motion_box_margin: f32,
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    Auto,
    Rpicam,
    V4l2,
}
