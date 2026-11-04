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
    start: usize,
    end: usize,
    eof: bool,
    #[cfg(test)]
    compactions: usize,
}
impl<R: Read> Chunker<R> {
    pub fn new(reader: R, key: &[u8; 32]) -> Self {
        Self {
            reader,
            gear: gear(key),
            buffer: vec![0; MAX],
            start: 0,
            end: 0,
            eof: false,
            #[cfg(test)]
            compactions: 0,
        }
    }
    /// The next chunk borrows the fixed buffer until the next call.
    pub fn next_chunk(&mut self) -> io::Result<Option<&[u8]>> {
        loop {
            let available = self.end - self.start;
            if available == 0 && self.eof {
                return Ok(None);
            }
            let length = cut(&self.buffer[self.start..self.end], &self.gear);
            // A discovered cut is independent of unread input. Exhaust the
            // current tail before compacting, unless we need more bytes to cut.
            if self.eof || available == MAX || length < available {
                let start = self.start;
                self.start += length;
                return Ok(Some(&self.buffer[start..self.start]));
            }
            if self.start > 0 {
                self.buffer.copy_within(self.start..self.end, 0);
                self.end = available;
                self.start = 0;
                #[cfg(test)]
                {
                    self.compactions += 1;
                }
            }
            while self.end < MAX {
                match self.reader.read(&mut self.buffer[self.end..]) {
                    Ok(0) => {
                        self.eof = true;
                        break;
                    }
                    Ok(n) => self.end += n,
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(e) => return Err(e),
                }
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cuts_borrow_one_fixed_buffer_and_compact_only_when_more_input_is_needed() {
        let mut data = vec![0; 32 * 1024 * 1024];
        blake3::Hasher::new().finalize_xof().fill(&mut data);
        let mut reader = Chunker::new(data.as_slice(), &[1; 32]);
        let allocation = reader.buffer.as_ptr();
        let mut chunks = 0;
        let mut total = 0;
        while let Some(chunk) = reader.next_chunk().unwrap() {
            chunks += 1;
            total += chunk.len();
        }
        assert_eq!(total, data.len());
        assert_eq!(reader.buffer.as_ptr(), allocation);
        assert!(reader.compactions < chunks / 2);
    }
}
