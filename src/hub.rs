//! Fans out captured JPEG frames from a single camera to any number of HTTP
//! viewers, and tracks the handful of stats `/healthz` reports.
//!
//! The camera is opened exactly once regardless of how many browsers are
//! watching: the capture backend publishes into the hub, and each viewer
//! gets its own broadcast subscription. A slow viewer falls behind and drops
//! frames (see [`FrameHub::subscribe`]) rather than ever blocking capture.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bytes::Bytes;
use tokio::sync::broadcast;

/// Bounds how far a viewer can fall behind before frames start being
/// dropped for them. Small on purpose: for live video, a queued-up frame is
/// a stale frame, so there's no benefit to buffering deeply.
const CHANNEL_CAPACITY: usize = 8;

pub struct HubStats {
    pub uptime: Duration,
    pub frames_captured: u64,
    /// Every live broadcast receiver, not just browser viewers - once
    /// detection is enabled, `detect::pump_loop` holds one of these too.
    /// `src/web.rs`'s `/healthz` handler tracks browser viewers separately
    /// for that reason, so this field currently only has test coverage;
    /// kept as a general-purpose stat rather than removed.
    #[allow(dead_code)]
    pub subscribers: usize,
    /// `None` if no frame has ever been captured.
    pub since_last_frame: Option<Duration>,
}

struct Inner {
    tx: broadcast::Sender<Bytes>,
    latest: Mutex<Option<Bytes>>,
    frame_count: AtomicU64,
    last_frame_at: Mutex<Option<Instant>>,
    started_at: Instant,
}

/// Cheaply cloneable handle to the shared frame hub.
#[derive(Clone)]
pub struct FrameHub {
    inner: Arc<Inner>,
}

impl FrameHub {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(CHANNEL_CAPACITY);
        Self {
            inner: Arc::new(Inner {
                tx,
                latest: Mutex::new(None),
                frame_count: AtomicU64::new(0),
                last_frame_at: Mutex::new(None),
                started_at: Instant::now(),
            }),
        }
    }

    /// Publish a newly captured frame. Cheap even with no subscribers.
    pub fn publish(&self, frame: Bytes) {
        *self.inner.latest.lock().unwrap() = Some(frame.clone());
        *self.inner.last_frame_at.lock().unwrap() = Some(Instant::now());
        self.inner.frame_count.fetch_add(1, Ordering::Relaxed);
        // No receivers is not an error - it just means nobody's watching.
        let _ = self.inner.tx.send(frame);
    }

    /// Subscribe to the live frame stream. If the receiver isn't drained
    /// fast enough, older frames are dropped in favor of newer ones
    /// (`RecvError::Lagged`) - callers should log and keep reading rather
    /// than treat it as fatal.
    pub fn subscribe(&self) -> broadcast::Receiver<Bytes> {
        self.inner.tx.subscribe()
    }

    /// The most recently published frame, if any, so a new viewer can paint
    /// something immediately instead of waiting for the next capture.
    pub fn latest(&self) -> Option<Bytes> {
        self.inner.latest.lock().unwrap().clone()
    }

    pub fn subscriber_count(&self) -> usize {
        self.inner.tx.receiver_count()
    }

    pub fn stats(&self) -> HubStats {
        let since_last_frame = self
            .inner
            .last_frame_at
            .lock()
            .unwrap()
            .map(|t| t.elapsed());
        HubStats {
            uptime: self.inner.started_at.elapsed(),
            frames_captured: self.inner.frame_count.load(Ordering::Relaxed),
            subscribers: self.subscriber_count(),
            since_last_frame,
        }
    }
}

impl Default for FrameHub {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latest_is_none_until_first_publish() {
        let hub = FrameHub::new();
        assert!(hub.latest().is_none());

        hub.publish(Bytes::from_static(b"frame-a"));
        assert_eq!(hub.latest().unwrap(), Bytes::from_static(b"frame-a"));
    }

    #[tokio::test]
    async fn all_subscribers_receive_a_published_frame() {
        let hub = FrameHub::new();
        let mut sub1 = hub.subscribe();
        let mut sub2 = hub.subscribe();
        assert_eq!(hub.subscriber_count(), 2);

        hub.publish(Bytes::from_static(b"frame-a"));

        assert_eq!(sub1.recv().await.unwrap(), Bytes::from_static(b"frame-a"));
        assert_eq!(sub2.recv().await.unwrap(), Bytes::from_static(b"frame-a"));
    }

    #[test]
    fn stats_reflect_frame_count_and_subscribers() {
        let hub = FrameHub::new();
        let _sub = hub.subscribe();
        hub.publish(Bytes::from_static(b"frame-a"));
        hub.publish(Bytes::from_static(b"frame-b"));

        let stats = hub.stats();
        assert_eq!(stats.frames_captured, 2);
        assert_eq!(stats.subscribers, 1);
        assert!(stats.since_last_frame.is_some());
    }
}
