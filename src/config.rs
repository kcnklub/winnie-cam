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
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    Auto,
    Rpicam,
    V4l2,
}
