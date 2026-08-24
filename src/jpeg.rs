//! Splits a raw byte stream of back-to-back JPEG images (as emitted by
//! `rpicam-vid --codec mjpeg -o -`) into individual complete frames.
//!
//! The stream has no framing of its own: frames are simply concatenated, and
//! arrive in arbitrarily-sized chunks that can split a frame anywhere,
//! including mid-marker or mid-entropy-data. [`JpegSplitter`] buffers input
//! and walks JPEG marker segments (rather than scanning for a literal `FF D9`
//! byte pair) so that an EOI-looking byte pair embedded inside e.g. an EXIF
//! thumbnail in an `APP1` segment can't be mistaken for the real end of frame.

use bytes::{Bytes, BytesMut};

/// Hard cap on the internal buffer. If a malformed stream never produces a
/// complete frame, the buffer is dropped and scanning resynchronizes on
/// whatever SOI arrives next, rather than growing without bound.
const MAX_BUFFER: usize = 8 * 1024 * 1024;

pub struct JpegSplitter {
    buf: BytesMut,
}

impl JpegSplitter {
    pub fn new() -> Self {
        Self {
            buf: BytesMut::new(),
        }
    }

    /// Feed in newly-read bytes and get back every complete JPEG frame that
    /// could be extracted, in order. Bytes that don't yet form a complete
    /// frame are retained internally for the next call.
    pub fn push(&mut self, data: &[u8]) -> Vec<Bytes> {
        self.buf.extend_from_slice(data);

        let mut frames = Vec::new();
        while let Some(frame) = self.try_extract_frame() {
            frames.push(frame);
        }

        if self.buf.len() > MAX_BUFFER {
            tracing::warn!(
                buffered = self.buf.len(),
                "jpeg splitter buffer exceeded cap without a complete frame, resyncing"
            );
            self.buf.clear();
        }

        frames
    }

    /// Finds the next Start Of Image marker, discarding anything before it,
    /// then scans forward to see whether a complete frame follows. Returns
    /// `None` when the buffer doesn't (yet) contain a complete frame.
    fn try_extract_frame(&mut self) -> Option<Bytes> {
        loop {
            let start = find_soi(&self.buf)?;
            if start > 0 {
                let _ = self.buf.split_to(start);
            }

            match scan_frame(&self.buf) {
                ScanResult::Frame(len) => return Some(self.buf.split_to(len).freeze()),
                ScanResult::NeedMoreData => return None,
                ScanResult::Resync => {
                    // What looked like a frame starting at this SOI wasn't
                    // one. Drop just this SOI and search for the next
                    // candidate later in the buffer.
                    let _ = self.buf.split_to(2);
                }
            }
        }
    }
}

impl Default for JpegSplitter {
    fn default() -> Self {
        Self::new()
    }
}

enum ScanResult {
    /// A complete frame occupies `buf[0..len]`.
    Frame(usize),
    /// Not enough data yet to tell; keep buffering.
    NeedMoreData,
    /// `buf` does not start with a well-formed frame; caller should drop the
    /// leading SOI and resync on the next one.
    Resync,
}

/// Returns the index of the next `FF D8` (SOI) marker in `buf`, if any.
fn find_soi(buf: &[u8]) -> Option<usize> {
    if buf.len() < 2 {
        return None;
    }
    (0..=buf.len() - 2).find(|&i| buf[i] == 0xFF && buf[i + 1] == 0xD8)
}

/// Walks JPEG marker segments starting at `buf[0]`, which must be the `FF`
/// of an SOI marker, to find where the frame ends.
fn scan_frame(buf: &[u8]) -> ScanResult {
    let mut pos = 2; // past FF D8

    loop {
        if pos + 1 >= buf.len() {
            return ScanResult::NeedMoreData;
        }
        if buf[pos] != 0xFF {
            return ScanResult::Resync;
        }

        let marker = buf[pos + 1];
        if marker == 0xFF {
            // Fill byte before the real marker code; keep looking.
            pos += 1;
            continue;
        }

        match marker {
            // A second SOI means whatever preceded it wasn't a real frame.
            0xD8 => return ScanResult::Resync,
            // Standalone EOI (a degenerate/minimal frame with no scan data).
            0xD9 => return ScanResult::Frame(pos + 2),
            // TEM and restart markers carry no length field or payload.
            0x01 | 0xD0..=0xD7 => pos += 2,
            // Start Of Scan: header has a length, then entropy-coded data
            // follows until the real EOI.
            0xDA => match scan_entropy_data(buf, pos) {
                Some(result) => return result,
                None => return ScanResult::NeedMoreData,
            },
            // 0x00 is only meaningful as byte-stuffing inside entropy-coded
            // data; seeing it here means the stream isn't well-formed.
            0x00 => return ScanResult::Resync,
            // Everything else (APPn, DQT, DHT, SOFn, COM, DRI, ...) is a
            // segment with a 2-byte big-endian length, length-field
            // inclusive, that we can skip over wholesale.
            _ => {
                if pos + 3 >= buf.len() {
                    return ScanResult::NeedMoreData;
                }
                let seg_len = u16::from_be_bytes([buf[pos + 2], buf[pos + 3]]) as usize;
                if seg_len < 2 {
                    return ScanResult::Resync;
                }
                pos += 2 + seg_len;
            }
        }
    }
}

/// Scans entropy-coded scan data (following an SOS header at `sos_pos`) for
/// the real EOI, skipping byte-stuffed `FF 00` and restart markers, both of
/// which can't be mistaken for EOI once identified. Returns `None` if more
/// data is needed.
fn scan_entropy_data(buf: &[u8], sos_pos: usize) -> Option<ScanResult> {
    if sos_pos + 3 >= buf.len() {
        return None;
    }
    let seg_len = u16::from_be_bytes([buf[sos_pos + 2], buf[sos_pos + 3]]) as usize;
    if seg_len < 2 {
        return Some(ScanResult::Resync);
    }

    let mut i = sos_pos + 2 + seg_len;
    loop {
        if i >= buf.len() {
            return None;
        }
        if buf[i] != 0xFF {
            i += 1;
            continue;
        }
        if i + 1 >= buf.len() {
            return None;
        }
        match buf[i + 1] {
            0x00 => i += 2,        // byte-stuffed literal 0xFF
            0xD0..=0xD7 => i += 2, // restart marker, part of the scan
            0xFF => i += 1,        // fill byte, real marker follows
            0xD9 => return Some(ScanResult::Frame(i + 2)),
            _ => return Some(ScanResult::Resync), // e.g. a truncated frame followed by a new SOI
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal degenerate "frame": SOI immediately followed by EOI, with no
    /// scan data. Enough to exercise the splitter without needing real
    /// image data.
    fn minimal_frame() -> Vec<u8> {
        vec![0xFF, 0xD8, 0xFF, 0xD9]
    }

    /// A frame with an SOS segment containing a byte-stuffed 0xFF in its
    /// entropy-coded data, to make sure stuffing isn't mistaken for EOI.
    fn frame_with_sos() -> Vec<u8> {
        vec![
            0xFF, 0xD8, // SOI
            0xFF, 0xDA, 0x00, 0x04, 0x3F, 0x00, // SOS header, length 4
            0x11, 0xFF, 0x00, 0x22, // entropy data with a stuffed 0xFF
            0xFF, 0xD9, // EOI
        ]
    }

    /// A frame with an APP1 segment whose payload contains a byte pair that
    /// looks exactly like an EOI marker, plus a real EOI afterward. A naive
    /// splitter that just scans for `FF D9` would truncate at the fake one.
    fn frame_with_embedded_fake_eoi() -> Vec<u8> {
        vec![
            0xFF, 0xD8, // SOI
            0xFF, 0xE1, 0x00, 0x08, // APP1, length 8 (2 + 6 data bytes)
            0xAA, 0xFF, 0xD9, 0xBB, 0xCC, 0xDD, // fake EOI inside the payload
            0xFF, 0xD9, // real EOI
        ]
    }

    #[test]
    fn splits_two_concatenated_frames_in_one_chunk() {
        let mut splitter = JpegSplitter::new();
        let mut input = minimal_frame();
        input.extend(minimal_frame());

        let frames = splitter.push(&input);
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].as_ref(), minimal_frame().as_slice());
        assert_eq!(frames[1].as_ref(), minimal_frame().as_slice());
    }

    #[test]
    fn assembles_a_frame_delivered_one_byte_at_a_time() {
        let mut splitter = JpegSplitter::new();
        let frame = minimal_frame();

        let mut collected = Vec::new();
        for byte in &frame {
            collected.extend(splitter.push(&[*byte]));
        }

        assert_eq!(collected.len(), 1);
        assert_eq!(collected[0].as_ref(), frame.as_slice());
    }

    #[test]
    fn reassembles_a_frame_split_mid_sos_across_a_chunk_boundary() {
        let mut splitter = JpegSplitter::new();
        let frame = frame_with_sos();

        // Split right after the stuffed 0xFF, before its trailing 0x00, so
        // the boundary lands inside what could be mistaken for a marker.
        let split_at = frame
            .windows(2)
            .position(|w| w == [0xFF, 0x00])
            .expect("stuffed FF present")
            + 1;
        let (first, second) = frame.split_at(split_at);

        let frames_from_first = splitter.push(first);
        assert!(
            frames_from_first.is_empty(),
            "no complete frame before the boundary"
        );

        let frames_from_second = splitter.push(second);
        assert_eq!(frames_from_second.len(), 1);
        assert_eq!(frames_from_second[0].as_ref(), frame.as_slice());
    }

    #[test]
    fn skips_leading_garbage_before_the_first_soi() {
        let mut splitter = JpegSplitter::new();
        let mut input = vec![0x00, 0x01, 0xFF, 0x02, 0xAB];
        input.extend(minimal_frame());

        let frames = splitter.push(&input);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].as_ref(), minimal_frame().as_slice());
    }

    #[test]
    fn does_not_truncate_on_eoi_bytes_embedded_in_an_app_segment() {
        let mut splitter = JpegSplitter::new();
        let frame = frame_with_embedded_fake_eoi();

        let frames = splitter.push(&frame);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].as_ref(), frame.as_slice());
    }

    #[test]
    fn resyncs_after_an_overlong_malformed_run() {
        let mut splitter = JpegSplitter::new();

        // An SOI followed by a segment claiming a length far larger than
        // will ever arrive should be dropped once the cap is hit, and a
        // subsequent well-formed frame should still be extracted.
        let mut garbage = vec![0xFF, 0xD8, 0xFF, 0xE0, 0xFF, 0xFF];
        garbage.extend(std::iter::repeat_n(0u8, MAX_BUFFER));
        let frames = splitter.push(&garbage);
        assert!(frames.is_empty());

        let frames = splitter.push(&minimal_frame());
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].as_ref(), minimal_frame().as_slice());
    }
}
