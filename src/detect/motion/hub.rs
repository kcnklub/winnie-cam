//! Fans motion events out to `/events` SSE viewers, `/events.json` pollers,
//! and `/healthz`.
//!
//! Deliberately not built on [`crate::detect::hub::DetectionHub`]'s `watch`
//! channel: detections are a "latest value" thing where a coalesced-away
//! intermediate result doesn't matter, but a motion event is a discrete
//! occurrence - losing one because two happened close together (which a
//! `watch` channel would do, since a new value overwrites the old one
//! before a slow receiver reads it) would mean a real
//! `motion_started`/`motion_stopped` pair silently vanishing. A `broadcast`
//! channel doesn't coalesce, and events are rare enough that nobody's
//! expected to actually lag it.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::sync::broadcast;

use crate::json::frac;
use super::detector::MotionKind;

/// How many past events `/events.json` and a freshly-connected `/events`
/// client can see. Generous enough to cover "what happened while I wasn't
/// looking at the tab" without growing unbounded.
const RING_CAPACITY: usize = 50;

/// Broadcast channel capacity. Events are rare (debounced, on the order of
/// one per real motion episode) so this only needs to be big enough to
/// survive a brief stall, unlike `FrameHub`'s tight video budget.
const CHANNEL_CAPACITY: usize = 16;

/// Surfaced on `/healthz` as `"motion"` (alongside `"off"` when detection -
/// and therefore motion - isn't running at all).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotionState {
    Idle,
    Active,
}

impl MotionState {
    pub fn as_str(self) -> &'static str {
        match self {
            MotionState::Idle => "idle",
            MotionState::Active => "active",
        }
    }
}

#[derive(Debug, Clone)]
pub struct MotionEvent {
    pub seq: u64,
    pub kind: MotionKind,
    /// Wall-clock time, milliseconds since the Unix epoch - the browser
    /// needs a real time to show, not a monotonic `Instant`.
    pub at_unix_ms: u64,
    /// The masked-diff score that triggered this transition, 0..1.
    pub score: f32,
}

/// Everything protected by one lock: the ring buffer and the sequence
/// counter that numbers it, kept together so a `publish` only takes one
/// lock instead of two that could observe each other out of order.
struct Ring {
    seq: u64,
    events: VecDeque<Arc<MotionEvent>>,
}

struct Inner {
    tx: broadcast::Sender<Arc<MotionEvent>>,
    ring: Mutex<Ring>,
    state: AtomicU8,
}

/// Cheaply cloneable handle to the shared motion event stream.
#[derive(Clone)]
pub struct MotionHub {
    inner: Arc<Inner>,
}

impl MotionHub {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(CHANNEL_CAPACITY);
        Self {
            inner: Arc::new(Inner {
                tx,
                ring: Mutex::new(Ring { seq: 0, events: VecDeque::with_capacity(RING_CAPACITY) }),
                state: AtomicU8::new(0),
            }),
        }
    }

    pub fn state(&self) -> MotionState {
        match self.inner.state.load(Ordering::Relaxed) {
            1 => MotionState::Active,
            _ => MotionState::Idle,
        }
    }

    /// Records one motion transition: pushes it onto the ring (evicting the
    /// oldest entry past [`RING_CAPACITY`]), updates the `/healthz` state,
    /// and broadcasts it to any live `/events` subscribers. Only
    /// `motion::worker_loop` ever calls this - like
    /// `DetectionHub::publish`, there's no concurrent-publisher case to
    /// race on, so `seq` is just a plain counter behind the same lock as
    /// the ring rather than a separate atomic.
    pub fn publish(&self, kind: MotionKind, score: f32) -> Arc<MotionEvent> {
        let active = matches!(kind, MotionKind::Started);
        self.inner.state.store(active as u8, Ordering::Relaxed);

        let at_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        let event = {
            let mut ring = self.inner.ring.lock().expect("motion ring lock");
            ring.seq += 1;
            let event = Arc::new(MotionEvent { seq: ring.seq, kind, at_unix_ms, score });
            ring.events.push_back(event.clone());
            if ring.events.len() > RING_CAPACITY {
                ring.events.pop_front();
            }
            event
        };

        // Ignore "no receivers" the same way `FrameHub::publish` does -
        // nobody watching `/events` right now doesn't mean the event
        // didn't happen, just that nobody's here to be pushed it live (the
        // ring still has it for the next connection).
        let _ = self.inner.tx.send(event.clone());
        event
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Arc<MotionEvent>> {
        self.inner.tx.subscribe()
    }

    pub fn subscriber_count(&self) -> usize {
        self.inner.tx.receiver_count()
    }

    /// Total events ever published, for `/healthz`'s `motion_events` field.
    pub fn event_count(&self) -> u64 {
        self.inner.ring.lock().expect("motion ring lock").seq
    }

    /// The ring buffer's current contents, oldest first.
    pub fn recent(&self) -> Vec<Arc<MotionEvent>> {
        self.inner.ring.lock().expect("motion ring lock").events.iter().cloned().collect()
    }

    /// `/events.json`'s body: the ring buffer, oldest first.
    pub fn recent_json(&self) -> String {
        let events = self.recent();
        let mut out = String::with_capacity(16 + events.len() * 72);
        out.push_str("{\"events\":[");
        for (i, e) in events.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str(&event_json(e));
        }
        out.push_str("]}");
        out
    }
}

impl Default for MotionHub {
    fn default() -> Self {
        Self::new()
    }
}

/// Hand-rolled JSON for one event, matching the style already established
/// by `detect::hub::detections_json` and `web::healthz` - see the former's
/// doc comment for why this project doesn't pull in `serde` for payloads
/// this small. Runs `score` through [`frac`] so a non-finite value (should
/// never happen, but inputs derived from float math aren't trusted blindly)
/// can't reach the wire and break `JSON.parse` for every viewer.
pub fn event_json(e: &MotionEvent) -> String {
    let (kind, duration_field) = match e.kind {
        MotionKind::Started => ("started", String::new()),
        MotionKind::Stopped { duration_ms } => ("stopped", format!(",\"duration_ms\":{duration_ms}")),
    };
    format!(
        "{{\"seq\":{},\"kind\":\"{kind}\",\"at\":{},\"score\":{:.4}{duration_field}}}",
        e.seq,
        e.at_unix_ms,
        frac(e.score),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seq_increments_across_publishes() {
        let hub = MotionHub::new();
        hub.publish(MotionKind::Started, 0.5);
        let second = hub.publish(MotionKind::Stopped { duration_ms: 1200 }, 0.1);
        assert_eq!(second.seq, 2);
        assert_eq!(hub.event_count(), 2);
    }

    #[test]
    fn state_reflects_the_most_recent_publish() {
        let hub = MotionHub::new();
        assert_eq!(hub.state(), MotionState::Idle);
        hub.publish(MotionKind::Started, 0.5);
        assert_eq!(hub.state(), MotionState::Active);
        hub.publish(MotionKind::Stopped { duration_ms: 500 }, 0.0);
        assert_eq!(hub.state(), MotionState::Idle);
    }

    #[test]
    fn the_ring_caps_at_its_capacity_and_evicts_the_oldest() {
        let hub = MotionHub::new();
        for _ in 0..(RING_CAPACITY + 10) {
            hub.publish(MotionKind::Started, 0.5);
            hub.publish(MotionKind::Stopped { duration_ms: 1 }, 0.5);
        }
        let recent = hub.recent();
        assert_eq!(recent.len(), RING_CAPACITY);
        // The oldest surviving entry should not be seq 1 - it must have
        // been evicted.
        assert!(recent.first().unwrap().seq > 1);
        // Ordered oldest-first.
        assert!(recent.windows(2).all(|w| w[0].seq < w[1].seq));
    }

    #[test]
    fn event_json_has_the_expected_shape_for_started_and_stopped() {
        let started = MotionEvent { seq: 3, kind: MotionKind::Started, at_unix_ms: 1000, score: 0.42 };
        assert_eq!(event_json(&started), "{\"seq\":3,\"kind\":\"started\",\"at\":1000,\"score\":0.4200}");

        let stopped = MotionEvent {
            seq: 4,
            kind: MotionKind::Stopped { duration_ms: 2500 },
            at_unix_ms: 2000,
            score: 0.05,
        };
        assert_eq!(
            event_json(&stopped),
            "{\"seq\":4,\"kind\":\"stopped\",\"at\":2000,\"score\":0.0500,\"duration_ms\":2500}"
        );
    }

    #[test]
    fn a_non_finite_score_never_reaches_the_wire() {
        let e = MotionEvent { seq: 1, kind: MotionKind::Started, at_unix_ms: 0, score: f32::NAN };
        let json = event_json(&e);
        assert!(!json.contains("NaN"), "{json}");
        assert!(!json.contains("inf"), "{json}");
    }

    #[test]
    fn recent_json_wraps_events_in_an_array() {
        let hub = MotionHub::new();
        assert_eq!(hub.recent_json(), "{\"events\":[]}");
        hub.publish(MotionKind::Started, 0.9);
        assert!(hub.recent_json().starts_with("{\"events\":[{\"seq\":1,"));
    }
}
