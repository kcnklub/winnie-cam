//! Splits the WebM byte stream `ffmpeg` writes to stdout into an init
//! segment plus whole clusters.
//!
//! A WebM stream is only decodable from its EBML header, so unlike MJPEG -
//! where any frame stands alone - a listener who connects while the stream
//! is already running cannot simply be handed the next bytes off the wire.
//! It has to be replayed the header first and then dropped in at a cluster
//! boundary, which is what this splitter exists to find:
//!
//! ```text
//! [EBML Header][Segment header ..][Cluster][Cluster][Cluster]...
//!  \_______________________________/       \_____/
//!            AudioChunk::Init               AudioChunk::Cluster
//! ```
//!
//! Boundaries are found by walking EBML element headers (an ID followed by
//! a variable-length size) rather than by scanning for the literal cluster
//! ID bytes, which would be free to occur inside a block's compressed audio
//! data and split a cluster in half.
//!
//! Only `AudioFormat::WebmOpus` needs any of this. ADTS is self-framing, so
//! it is passed straight through - see [`crate::audio::alsa`].

use bytes::{Bytes, BytesMut};

/// Hard cap on the internal buffer. If a malformed stream never yields a
/// complete cluster, the buffer is dropped rather than growing without
/// bound - the same protection [`crate::jpeg::JpegSplitter`] has.
const MAX_BUFFER: usize = 4 * 1024 * 1024;

/// EBML element IDs, stored as their on-the-wire bytes (marker bit included)
/// packed into a u32, which is how [`read_id`] reports them.
const ID_SEGMENT: u32 = 0x1853_8067;
const ID_CLUSTER: u32 = 0x1F43_B675;

/// One decodable piece of the stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioChunk {
    /// Everything up to the first cluster: the EBML header, the segment
    /// header, and the track definitions. Emitted exactly once per stream,
    /// and replayed verbatim to every listener that connects later.
    Init(Bytes),
    /// One complete cluster. Meaningless on its own - a decoder must have
    /// been given the [`AudioChunk::Init`] first.
    Cluster(Bytes),
}

/// Where in the stream the splitter is. The two states parse different
/// things, so they are worth naming rather than inferring from a flag.
enum State {
    /// Still walking the header elements, looking for the first cluster.
    Init,
    /// The init segment has been emitted; the buffer starts on a cluster.
    Clusters,
}

pub struct WebmChunker {
    buf: BytesMut,
    state: State,
}

impl WebmChunker {
    pub fn new() -> Self {
        Self {
            buf: BytesMut::new(),
            state: State::Init,
        }
    }

    /// Feed in newly-read bytes and get back every complete chunk that could
    /// be extracted, in order. Bytes that don't yet complete a chunk are
    /// retained for the next call.
    pub fn push(&mut self, data: &[u8]) -> Vec<AudioChunk> {
        self.buf.extend_from_slice(data);

        let mut chunks = Vec::new();
        loop {
            let chunk = match self.state {
                State::Init => self.take_init(),
                State::Clusters => self.take_cluster(),
            };
            match chunk {
                Some(chunk) => chunks.push(chunk),
                None => break,
            }
        }

        if self.buf.len() > MAX_BUFFER {
            tracing::warn!(
                buffered = self.buf.len(),
                "webm chunker buffer exceeded cap without a complete chunk, resyncing"
            );
            self.buf.clear();
        }

        chunks
    }

    /// Walks header elements until the first cluster is found, and emits
    /// everything before it as the init segment.
    fn take_init(&mut self) -> Option<AudioChunk> {
        let mut pos = 0;

        loop {
            let header = match read_header(&self.buf[pos..]) {
                Parse::Ok(header) => header,
                // A malformed header this early means ffmpeg isn't writing
                // what we think it is; there is nothing useful to resync to.
                Parse::NeedMoreData | Parse::Malformed => return None,
            };

            if header.id == ID_CLUSTER {
                let init = self.buf.split_to(pos).freeze();
                self.state = State::Clusters;
                return Some(AudioChunk::Init(init));
            }

            // Descend into the segment rather than skipping it: clusters
            // live inside it, and on a pipe its size is "unknown" anyway.
            if header.id == ID_SEGMENT {
                pos += header.len;
                continue;
            }

            // Any other header element (SeekHead, Info, Tracks, Tags) is
            // part of the init segment and is stepped over whole.
            let size = header.size?;
            pos += header.len + size as usize;
            if pos > self.buf.len() {
                return None;
            }
        }
    }

    /// Extracts one whole cluster from the front of the buffer.
    fn take_cluster(&mut self) -> Option<AudioChunk> {
        let header = match read_header(&self.buf) {
            Parse::Ok(header) => header,
            Parse::NeedMoreData => return None,
            Parse::Malformed => {
                tracing::warn!("webm chunker lost element alignment, resyncing on next cluster");
                return self.resync();
            }
        };

        // ffmpeg buffers each cluster before writing it, so sizes are known
        // even on a pipe. Unknown-size clusters are legal EBML though, and
        // then the only end marker is the next cluster starting.
        let end = match header.size {
            Some(size) => header.len + size as usize,
            None => find_cluster_id(&self.buf, header.len)?,
        };
        if self.buf.len() < end {
            return None;
        }

        let body = self.buf.split_to(end).freeze();

        // Trailing non-cluster elements (Cues, Tags) are dropped: they carry
        // no audio and a live listener has no use for them.
        if header.id != ID_CLUSTER {
            tracing::debug!(id = header.id, "skipping non-cluster element");
            return self.take_cluster();
        }

        Some(AudioChunk::Cluster(body))
    }

    /// Drops everything up to the next cluster ID, so a corrupt run of bytes
    /// costs one cluster instead of the whole connection.
    fn resync(&mut self) -> Option<AudioChunk> {
        let next = find_cluster_id(&self.buf, 1)?;
        let _ = self.buf.split_to(next);
        self.take_cluster()
    }
}

impl Default for WebmChunker {
    fn default() -> Self {
        Self::new()
    }
}

/// Outcome of parsing an EBML element header. Distinguishing "not yet" from
/// "never" matters: the first is normal on a chunked stream, the second
/// means the input isn't the WebM we expect.
enum Parse<T> {
    Ok(T),
    NeedMoreData,
    Malformed,
}

/// An element's ID plus how long its header is and how much content follows.
struct Header {
    id: u32,
    /// Bytes occupied by the ID and size together.
    len: usize,
    /// Content length, or `None` for EBML's "unknown size" encoding.
    size: Option<u64>,
}

fn read_header(buf: &[u8]) -> Parse<Header> {
    let (id, id_len) = match read_id(buf) {
        Parse::Ok(v) => v,
        Parse::NeedMoreData => return Parse::NeedMoreData,
        Parse::Malformed => return Parse::Malformed,
    };
    let (size, size_len) = match read_size(&buf[id_len..]) {
        Parse::Ok(v) => v,
        Parse::NeedMoreData => return Parse::NeedMoreData,
        Parse::Malformed => return Parse::Malformed,
    };

    Parse::Ok(Header {
        id,
        len: id_len + size_len,
        size,
    })
}

/// Reads an EBML element ID: 1-4 bytes, its length encoded as the number of
/// leading zero bits in the first byte. The marker bit is kept, since IDs
/// are conventionally written including it (`0x1F43B675`, not `0x0F43B675`).
fn read_id(buf: &[u8]) -> Parse<(u32, usize)> {
    let Some(&first) = buf.first() else {
        return Parse::NeedMoreData;
    };
    let len = first.leading_zeros() as usize + 1;
    if len > 4 {
        return Parse::Malformed;
    }
    if buf.len() < len {
        return Parse::NeedMoreData;
    }

    let id = buf[..len]
        .iter()
        .fold(0u32, |acc, &b| (acc << 8) | b as u32);
    Parse::Ok((id, len))
}

/// Reads an EBML size: 1-8 bytes, length encoded like an ID's but with the
/// marker bit cleared from the value. All-ones means "unknown size".
fn read_size(buf: &[u8]) -> Parse<(Option<u64>, usize)> {
    let Some(&first) = buf.first() else {
        return Parse::NeedMoreData;
    };
    if first == 0 {
        return Parse::Malformed;
    }
    let len = first.leading_zeros() as usize + 1;
    if buf.len() < len {
        return Parse::NeedMoreData;
    }

    let mut value = (first as u64) & (0xFF >> len);
    for &b in &buf[1..len] {
        value = (value << 8) | b as u64;
    }

    // The unknown-size encoding is every value bit set - 7 bits for a
    // 1-byte size, 14 for 2 bytes, and so on.
    let all_ones = (1u64 << (7 * len)) - 1;
    let size = (value != all_ones).then_some(value);

    Parse::Ok((size, len))
}

/// Finds the next cluster ID at or after `from`. Only used for the
/// unknown-size cases, where there is no length to trust.
fn find_cluster_id(buf: &[u8], from: usize) -> Option<usize> {
    const ID_BYTES: [u8; 4] = ID_CLUSTER.to_be_bytes();
    if buf.len() < ID_BYTES.len() {
        return None;
    }
    (from..=buf.len() - ID_BYTES.len()).find(|&i| buf[i..i + ID_BYTES.len()] == ID_BYTES)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds an element header: ID bytes, then a 1-byte size.
    fn element(id: u32, body: &[u8]) -> Vec<u8> {
        let id_len = 4 - (id.leading_zeros() / 8) as usize;
        let mut out = Vec::new();
        out.extend_from_slice(&id.to_be_bytes()[4 - id_len..]);
        assert!(body.len() < 0x80, "test helper only emits 1-byte sizes");
        out.push(0x80 | body.len() as u8);
        out.extend_from_slice(body);
        out
    }

    /// A minimal but structurally real stream: EBML header, an unknown-size
    /// segment, one Tracks element, then `clusters` clusters.
    fn stream(clusters: &[&[u8]]) -> (Vec<u8>, Vec<u8>) {
        const ID_EBML: u32 = 0x1A45_DFA3;
        const ID_TRACKS: u32 = 0x1654_AE6B;

        let mut init = element(ID_EBML, b"hdr");
        init.extend_from_slice(&ID_SEGMENT.to_be_bytes());
        init.push(0xFF); // unknown size
        init.extend_from_slice(&element(ID_TRACKS, b"tracks"));

        let mut all = init.clone();
        for body in clusters {
            all.extend_from_slice(&element(ID_CLUSTER, body));
        }
        (init, all)
    }

    #[test]
    fn splits_init_from_clusters() {
        let (init, all) = stream(&[b"one", b"two"]);

        let chunks = WebmChunker::new().push(&all);

        assert_eq!(
            chunks,
            vec![
                AudioChunk::Init(Bytes::from(init)),
                AudioChunk::Cluster(Bytes::from(element(ID_CLUSTER, b"one"))),
                AudioChunk::Cluster(Bytes::from(element(ID_CLUSTER, b"two"))),
            ]
        );
    }

    #[test]
    fn emits_nothing_until_the_first_cluster_starts() {
        let (init, _) = stream(&[]);
        let mut chunker = WebmChunker::new();

        assert!(chunker.push(&init).is_empty());
    }

    #[test]
    fn reassembles_chunks_split_across_reads() {
        let (init, all) = stream(&[b"one", b"two"]);
        let mut chunker = WebmChunker::new();

        // One byte at a time is the worst case a 64KB pipe read can produce.
        let mut chunks = Vec::new();
        for byte in &all {
            chunks.extend(chunker.push(&[*byte]));
        }

        assert_eq!(
            chunks,
            vec![
                AudioChunk::Init(Bytes::from(init)),
                AudioChunk::Cluster(Bytes::from(element(ID_CLUSTER, b"one"))),
                AudioChunk::Cluster(Bytes::from(element(ID_CLUSTER, b"two"))),
            ]
        );
    }

    #[test]
    fn cluster_id_inside_block_data_is_not_a_boundary() {
        // The literal cluster ID as payload - what a naive byte scan would
        // split on, corrupting both halves.
        let payload = ID_CLUSTER.to_be_bytes();
        let (_, all) = stream(&[&payload]);

        let chunks = WebmChunker::new().push(&all);

        assert_eq!(chunks.len(), 2, "expected exactly one init and one cluster");
        assert_eq!(
            chunks[1],
            AudioChunk::Cluster(Bytes::from(element(ID_CLUSTER, &payload)))
        );
    }

    #[test]
    fn resyncs_after_a_corrupt_element() {
        let (_, all) = stream(&[b"one"]);
        let good = element(ID_CLUSTER, b"two");

        let mut chunker = WebmChunker::new();
        chunker.push(&all);

        // A single stray zero byte can't start a valid EBML size.
        let chunks = chunker.push(&[0x00]);
        assert!(chunks.is_empty());

        assert_eq!(
            chunker.push(&good),
            vec![AudioChunk::Cluster(Bytes::from(good.clone()))]
        );
    }

    #[test]
    fn reads_unknown_size_as_none() {
        match read_size(&[0xFF]) {
            Parse::Ok((size, len)) => {
                assert_eq!(size, None);
                assert_eq!(len, 1);
            }
            _ => panic!("expected a parsed size"),
        }
    }
}
