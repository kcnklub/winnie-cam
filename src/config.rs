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

    /// Enable person detection. Off by default: inference is by far the
    /// most expensive thing this program can do, and video capture must
    /// never be affected by it. See `--model`.
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
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    Auto,
    Rpicam,
    V4l2,
}
