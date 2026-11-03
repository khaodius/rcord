use flate2::{Decompress, FlushDecompress, Status};

const ZLIB_SUFFIX: [u8; 4] = [0x00, 0x00, 0xff, 0xff];

/// Decoder for `compress=zlib-stream`: one zlib context for the whole connection, with each
/// message ending in a sync flush. Frames are buffered until the flush marker arrives.
pub struct ZlibStream {
    inflater: Decompress,
    pending: Vec<u8>,
    output: Vec<u8>,
}

impl Default for ZlibStream {
    fn default() -> Self {
        Self::new()
    }
}

impl ZlibStream {
    pub fn new() -> Self {
        Self {
            inflater: Decompress::new(true),
            pending: Vec::with_capacity(64 * 1024),
            output: Vec::with_capacity(256 * 1024),
        }
    }

    /// Feeds one binary frame. Returns the decompressed message once a frame ends it.
    pub fn push(&mut self, frame: &[u8]) -> Result<Option<&[u8]>, flate2::DecompressError> {
        self.pending.extend_from_slice(frame);
        if !self.pending.ends_with(&ZLIB_SUFFIX) {
            return Ok(None);
        }
        self.output.clear();
        let mut offset = 0;
        loop {
            if self.output.capacity() == self.output.len() {
                self.output.reserve(self.output.capacity().max(64 * 1024));
            }
            let before_in = self.inflater.total_in();
            let status = self.inflater.decompress_vec(
                &self.pending[offset..],
                &mut self.output,
                FlushDecompress::Sync,
            )?;
            offset += (self.inflater.total_in() - before_in) as usize;
            let output_full = self.output.len() == self.output.capacity();
            if offset >= self.pending.len() && !output_full {
                break;
            }
            if status == Status::StreamEnd {
                break;
            }
        }
        self.pending.clear();
        // READY can be tens of megabytes; do not keep that much buffered afterwards.
        if self.pending.capacity() > 1 << 20 {
            self.pending.shrink_to(64 * 1024);
        }
        Ok(Some(&self.output))
    }

    /// Releases the buffers a large message grew, once it has been parsed.
    pub fn trim(&mut self) {
        if self.output.capacity() > 4 << 20 {
            self.output = Vec::with_capacity(256 * 1024);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{Compress, Compression, FlushCompress};

    fn compress_stream(messages: &[&[u8]]) -> Vec<Vec<u8>> {
        let mut c = Compress::new(Compression::default(), true);
        messages
            .iter()
            .map(|m| {
                let mut out = Vec::with_capacity(m.len() * 2 + 64);
                c.compress_vec(m, &mut out, FlushCompress::Sync).unwrap();
                out
            })
            .collect()
    }

    #[test]
    fn shared_context_and_split_frames() {
        let big = vec![b'x'; 1_000_000];
        let frames = compress_stream(&[b"{\"op\":10}", &big, b"{\"op\":11}"]);
        let mut z = ZlibStream::new();
        assert_eq!(z.push(&frames[0]).unwrap().unwrap(), b"{\"op\":10}");
        let (a, b) = frames[1].split_at(frames[1].len() / 2);
        assert!(z.push(a).unwrap().is_none());
        assert_eq!(z.push(b).unwrap().unwrap().len(), big.len());
        z.trim();
        assert_eq!(z.push(&frames[2]).unwrap().unwrap(), b"{\"op\":11}");
    }
}
