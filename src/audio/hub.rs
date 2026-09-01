//! Fans out encoded audio from one microphone to any number of listeners,
//! and tells the supervisor whether the microphone needs to be open at all.
//!
//! Deliberately shaped differently from [`crate::hub::FrameHub`] in two
//! ways, both forced by audio not being frame-independent:
//!
//! - A new listener must be replayed the stream's init segment before any
//!   chunk makes sense, so that is held in a `watch` channel alongside the
//!   `broadcast` of chunks (see [`crate::audio::webm`]).
//! - The listener count is itself a `watch` channel rather than a plain
//!   counter, because the supervisor waits on it: with nobody listening
//!   there is no reason to hold the microphone open on a Pi that is already
//!   spending its CPU budget on inference.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use bytes::Bytes;
use tokio::sync::{broadcast, watch};

/// Roughly two seconds of 100ms clusters. Small on purpose, for the same
/// reason [`crate::hub`]'s is: a queued-up chunk is stale audio, and a
/// listener that has fallen this far behind is better off skipping ahead.
const CHANNEL_CAPACITY: usize = 20;

/// What the microphone is doing, for `/healthz`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioState {
    /// Enabled, but nobody is listening, so no subprocess is running.
    Idle,
    /// The microphone is open and chunks are being published.
    Streaming,
}

impl AudioState {
    pub fn as_str(self) -> &'static str {
        match self {
            AudioState::Idle => "idle",
            AudioState::Streaming => "streaming",
        }
    }
}

/// The header a decoder needs before any chunk of this stream is decodable.
///
/// `generation` increments every time the microphone subprocess restarts.
/// Chunks from a new subprocess cannot be decoded against an older init
/// segment, so a listener holding a stale generation has to be disconnected
/// rather than fed bytes that will only produce noise.
pub struct InitSegment {
    pub generation: u64,
    /// Empty for self-framing formats like ADTS, which need no header.
    pub bytes: Bytes,
}

struct Inner {
    chunks: broadcast::Sender<Bytes>,
    init: watch::Sender<Option<Arc<InitSegment>>>,
    listeners: watch::Sender<usize>,
    running: AtomicBool,
    /// Kept outside the `init` channel so it survives [`AudioHub::end_stream`]
    /// clearing that channel, and keeps increasing across restarts.
    generation: AtomicU64,
}

/// Cheaply cloneable handle to the shared audio stream.
#[derive(Clone)]
pub struct AudioHub {
    inner: Arc<Inner>,
}

impl AudioHub {
    pub fn new() -> Self {
        let (chunks, _) = broadcast::channel(CHANNEL_CAPACITY);
        Self {
            inner: Arc::new(Inner {
                chunks,
                init: watch::Sender::new(None),
                listeners: watch::Sender::new(0),
                running: AtomicBool::new(false),
                generation: AtomicU64::new(0),
            }),
        }
    }

    /// Publish the init segment for a freshly started stream. Must be called
    /// before the first [`AudioHub::publish`] of that stream, so a listener
    /// never receives chunks it has no header for.
    pub fn start_stream(&self, bytes: Bytes) {
        let generation = self.inner.generation.fetch_add(1, Ordering::Relaxed);

        self.inner.running.store(true, Ordering::Release);
        // `send_replace`, not `send`: the latter refuses to store anything
        // when no receiver happens to be subscribed, which would lose the
        // header of a stream started for a listener that hasn't reached the
        // handler body yet.
        let _ = self
            .inner
            .init
            .send_replace(Some(Arc::new(InitSegment { generation, bytes })));
    }

    /// Mark the stream as over. Open listener connections see the init
    /// segment go away and end; new ones wait for the next stream to start
    /// rather than being handed a header the microphone is no longer
    /// producing chunks for.
    pub fn end_stream(&self) {
        self.inner.running.store(false, Ordering::Release);
        let _ = self.inner.init.send_replace(None);
    }

    /// Publish one encoded chunk. Cheap even with no listeners.
    pub fn publish(&self, chunk: Bytes) {
        // No receivers is not an error - it just means the last listener
        // disconnected and the supervisor hasn't wound us down yet.
        let _ = self.inner.chunks.send(chunk);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Bytes> {
        self.inner.chunks.subscribe()
    }

    /// Watches the current stream's init segment. `None` means no stream is
    /// running right now.
    pub fn init(&self) -> watch::Receiver<Option<Arc<InitSegment>>> {
        self.inner.init.subscribe()
    }

    /// Registers a listener for as long as the returned guard is alive.
    ///
    /// RAII rather than a pair of calls for the reason spelled out on
    /// `web::DisconnectLog`: a response whose body is never polled (axum
    /// routes `HEAD` to the same handler) would otherwise leak a listener
    /// forever, and here that would also pin the microphone open.
    pub fn listen(&self) -> ListenerGuard {
        self.inner.listeners.send_modify(|count| *count += 1);
        tracing::info!(listeners = self.listener_count(), "listener connected");
        ListenerGuard { hub: self.clone() }
    }

    pub fn listener_count(&self) -> usize {
        *self.inner.listeners.borrow()
    }

    /// Watches the listener count, so the supervisor can sleep until the
    /// first listener arrives or the last one leaves.
    pub fn listeners(&self) -> watch::Receiver<usize> {
        self.inner.listeners.subscribe()
    }

    pub fn state(&self) -> AudioState {
        match self.inner.running.load(Ordering::Acquire) {
            true => AudioState::Streaming,
            false => AudioState::Idle,
        }
    }
}

impl Default for AudioHub {
    fn default() -> Self {
        Self::new()
    }
}

/// Decrements the listener count when dropped. See [`AudioHub::listen`].
pub struct ListenerGuard {
    hub: AudioHub,
}

impl Drop for ListenerGuard {
    fn drop(&mut self) {
        self.hub
            .inner
            .listeners
            .send_modify(|count| *count = count.saturating_sub(1));
        tracing::info!(
            listeners = self.hub.listener_count(),
            "listener disconnected"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_idle_with_no_listeners() {
        let hub = AudioHub::new();

        assert_eq!(hub.state(), AudioState::Idle);
        assert_eq!(hub.listener_count(), 0);
        assert!(hub.init().borrow().is_none());
    }

    #[test]
    fn listener_guard_releases_on_drop() {
        let hub = AudioHub::new();

        let guard = hub.listen();
        assert_eq!(hub.listener_count(), 1);

        drop(guard);
        assert_eq!(hub.listener_count(), 0);
    }

    #[tokio::test]
    async fn all_listeners_receive_a_published_chunk() {
        let hub = AudioHub::new();
        let mut a = hub.subscribe();
        let mut b = hub.subscribe();

        hub.publish(Bytes::from_static(b"chunk"));

        assert_eq!(a.recv().await.unwrap(), Bytes::from_static(b"chunk"));
        assert_eq!(b.recv().await.unwrap(), Bytes::from_static(b"chunk"));
    }

    #[test]
    fn restarting_a_stream_bumps_the_generation() {
        let hub = AudioHub::new();

        hub.start_stream(Bytes::from_static(b"init-a"));
        assert_eq!(hub.state(), AudioState::Streaming);
        let first = hub.init().borrow().clone().unwrap().generation;

        hub.end_stream();
        assert_eq!(hub.state(), AudioState::Idle);

        hub.start_stream(Bytes::from_static(b"init-b"));
        let second = hub.init().borrow().clone().unwrap();

        assert!(second.generation > first);
        assert_eq!(second.bytes, Bytes::from_static(b"init-b"));
    }
}
