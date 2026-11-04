//! SPEC §6: keyed gear and streaming normalized FastCDC.
use std::io::{self, Read};
pub const MIN: usize = 524288;
pub const NORMAL: usize = 1048576;
pub const MAX: usize = 8388608;

pub fn gear(key: &[u8; 32]) -> [u64; 256] {
    let mut bytes = [0; 2048];
    blake3::Hasher::new_keyed(key)
        .finalize_xof()
        .fill(&mut bytes);
    std::array::from_fn(|i| {
        u64::from_le_bytes(
            bytes[i * 8..i * 8 + 8]
                .try_into()
                .expect("eight-byte entry"),
        )
    })
}

pub fn cut(bytes: &[u8], gear: &[u64; 256]) -> usize {
    let end = bytes.len().min(MAX);
    if end <= MIN {
        return end;
    }
    let normal = end.min(NORMAL);
    let mut hash = 0u64;
    for i in MIN..end {
        hash = (hash << 1).wrapping_add(gear[bytes[i] as usize]);
        let mask = if i < normal {
            0xFFFF_F800_0000_0000
        } else {
            0xFFFF_E000_0000_0000
        };
        if hash & mask == 0 {
            return i + 1;
        }
    }
    end
}

pub struct Chunker<R> {
    reader: R,
    gear: [u64; 256],
    buffer: Vec<u8>,
    eof: bool,
}
impl<R: Read> Chunker<R> {
    pub fn new(reader: R, key: &[u8; 32]) -> Self {
        Self {
            reader,
            gear: gear(key),
            buffer: Vec::with_capacity(MAX),
            eof: false,
        }
    }
    pub fn next_chunk(&mut self) -> io::Result<Option<Vec<u8>>> {
        if !self.eof {
            let mut filled = self.buffer.len();
            self.buffer.resize(MAX, 0);
            while filled < MAX {
                match self.reader.read(&mut self.buffer[filled..]) {
                    Ok(0) => {
                        self.eof = true;
                        break;
                    }
                    Ok(n) => filled += n,
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(e) => {
                        self.buffer.truncate(filled);
                        return Err(e);
                    }
                }
            }
            self.buffer.truncate(filled);
        }
        if self.buffer.is_empty() {
            return Ok(None);
        }
        let end = cut(&self.buffer, &self.gear);
        let chunk = self.buffer[..end].to_vec();
        self.buffer.copy_within(end.., 0);
        self.buffer.truncate(self.buffer.len() - end);
        Ok(Some(chunk))
    }
}
