//! Decides what the `<audio>` element should do to stay pinned to the live
//! edge of the MSE buffer. A browser media element happily plays further
//! and further behind a live source after any stall; this turns buffered
//! ranges into a single decision each tick.
//!
//! Pure logic, wasm-independent, unit tested natively.

/// Steady-state distance behind the buffered end, in seconds. Audio this
/// close to live reads as synchronized with the (near-latency-free) MJPEG
/// video while still tolerating jitter.
pub const TARGET_LAG_SECS: f64 = 0.3;

/// Past this much lag the gentle path is abandoned and playback seeks to
/// the live edge. Audio-only seeks are inaudible.
pub const MAX_LAG_SECS: f64 = 1.5;

/// Buffered audio further behind playback than this is trimmed from the
/// SourceBuffer so a live session never hits its quota.
pub const KEEP_BEHIND_SECS: f64 = 5.0;

/// What the keep-up tick should do this cycle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Decision {
    /// Lag is inside the threshold; keep playing where we are.
    Hold,
    /// Lag exceeded the threshold; jump to this playback position.
    Seek(f64),
}

/// One keep-up decision.
///
/// `buffered_end` is the end of the last buffered time range — the newest
/// data the decoder has. `None` (or an end already behind playback, which
/// shouldn't happen) means there is nothing playable and the only correct
/// move is to wait.
pub fn decide(current_time: f64, buffered_end: Option<f64>) -> Decision {
    let Some(end) = buffered_end else {
        return Decision::Hold;
    };

    let lag = end - current_time;
    if lag <= MAX_LAG_SECS {
        return Decision::Hold;
    }

    // With MAX_LAG_SECS > TARGET_LAG_SECS this is always ahead of
    // `current_time`, so seeks are forward-only by construction.
    Decision::Seek(end - TARGET_LAG_SECS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn holds_within_threshold() {
        // Just under the max lag.
        assert_eq!(decide(10.0, Some(11.4)), Decision::Hold);
        // Right at the max lag.
        assert_eq!(decide(10.0, Some(11.5)), Decision::Hold);
    }

    #[test]
    fn holds_with_no_buffer() {
        assert_eq!(decide(10.0, None), Decision::Hold);
    }

    #[test]
    fn holds_when_buffer_is_behind_playback() {
        // Defensive: garbage ranges must never produce a backwards seek.
        assert_eq!(decide(10.0, Some(9.0)), Decision::Hold);
    }

    #[test]
    fn seeks_to_target_when_far_behind() {
        assert_eq!(
            decide(10.0, Some(16.0)),
            Decision::Seek(16.0 - TARGET_LAG_SECS)
        );
    }

    #[test]
    #[allow(clippy::assertions_on_constants)]
    fn threshold_ordering_keeps_seeks_forward_only() {
        // Tautological with today's constants on purpose: it only fires if
        // someone retunes them into an order that makes seeks go backwards.
        assert!(MAX_LAG_SECS > TARGET_LAG_SECS);
    }
}
