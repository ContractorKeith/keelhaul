use std::io::{self, Read};
// Drain continuously, retaining a bounded diagnostic tail.
pub(super) fn drain(input: &mut impl Read) -> Vec<u8> {
    let mut tail = Vec::new();
    let mut buffer = [0; 4096];
    loop {
        match input.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => {
                tail.extend_from_slice(&buffer[..count]);
                if tail.len() > 65536 {
                    tail.drain(..tail.len() - 65536);
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
    tail
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sftp_stderr_continues_after_interrupted_read() {
        // Chain a finite tail after an injected interruption.
        struct Reader {
            interrupted: bool,
            rest: io::Cursor<Vec<u8>>,
        }
        impl Read for Reader {
            fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
                if !self.interrupted {
                    self.interrupted = true;
                    return Err(io::ErrorKind::Interrupted.into());
                }
                self.rest.read(out)
            }
        }
        let mut reader = Reader {
            interrupted: false,
            rest: io::Cursor::new(b"ssh diagnostic".to_vec()),
        };
        assert_eq!(drain(&mut reader), b"ssh diagnostic");
    }
}
