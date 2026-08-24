/// Connection health tracked by the MJPEG stream hook and surfaced in the
/// status indicator bar.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ConnectionState {
    Connecting,
    Live,
    Reconnecting,
    Stale,
    Offline,
}

impl ConnectionState {
    /// Maps to CSS classes `is-live` / `is-wait` / `is-bad` for the status
    /// pill.
    pub fn css_class(&self) -> &'static str {
        match self {
            ConnectionState::Connecting => "is-wait",
            ConnectionState::Live => "is-live",
            ConnectionState::Reconnecting => "is-wait",
            ConnectionState::Stale => "is-wait",
            ConnectionState::Offline => "is-bad",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            ConnectionState::Connecting => "Connecting\u{2026}",
            ConnectionState::Live => "Live",
            ConnectionState::Reconnecting => "Reconnecting\u{2026}",
            ConnectionState::Stale => "Feed stalled \u{2014} retrying\u{2026}",
            ConnectionState::Offline => "Can\u{2019}t reach the camera",
        }
    }
}

/// Placeholder content shown over the stage when video isn't streaming.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placeholder {
    pub title: &'static str,
    pub sub: &'static str,
}

impl Placeholder {
    pub const WAKING: Placeholder = Placeholder {
        title: "Waking the camera up\u{2026}",
        sub: "This only takes a moment.",
    };

    pub const LOST: Placeholder = Placeholder {
        title: "Lost the picture",
        sub: "Reconnecting on its own \u{2014} hang tight.",
    };

    pub const NO_CAMERA: Placeholder = Placeholder {
        title: "No camera picture yet",
        sub: "The camera may still be starting, or unplugged.",
    };

    pub const UNREACHABLE: Placeholder = Placeholder {
        title: "Can\u{2019}t reach the monitor",
        sub: "Check that the Pi is on and you\u{2019}re on the same Wi-Fi.",
    };
}
