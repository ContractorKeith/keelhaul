//! Bounded SFTP v3 framing and primitive fields (network byte order).
use std::io::{self, Read, Write};
pub const MAX_PACKET: usize = 256 * 1024;
pub fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[derive(Debug, PartialEq, Eq)]
pub struct Packet {
    pub kind: u8,
    pub body: Vec<u8>,
}
impl Packet {
    pub fn read(input: &mut impl Read) -> io::Result<Self> {
        let mut size = [0; 4];
        input.read_exact(&mut size)?;
        let size = u32::from_be_bytes(size) as usize;
        if !(1..=MAX_PACKET).contains(&size) {
            return Err(invalid("invalid SFTP packet length"));
        }
        let mut bytes = vec![0; size];
        input.read_exact(&mut bytes)?;
        let kind = bytes[0];
        if !matches!(kind, 1..=6 | 7 | 8 | 11..=18 | 101..=105 | 200 | 201) {
            return Err(invalid("unknown SFTP packet type"));
        }
        Ok(Self {
            kind,
            body: bytes[1..].to_vec(),
        })
    }
    pub fn write(&self, output: &mut impl Write) -> io::Result<()> {
        if self.body.len() >= MAX_PACKET {
            return Err(invalid("oversized SFTP packet"));
        }
        output.write_all(&((self.body.len() + 1) as u32).to_be_bytes())?;
        output.write_all(&[self.kind])?;
        output.write_all(&self.body)?;
        output.flush()
    }
}
pub fn u32_field(out: &mut Vec<u8>, value: u32) {
    out.extend(value.to_be_bytes());
}
pub fn u64_field(out: &mut Vec<u8>, value: u64) {
    out.extend(value.to_be_bytes());
}
pub fn string(out: &mut Vec<u8>, value: &[u8]) {
    u32_field(out, value.len() as u32);
    out.extend(value);
}
pub struct Fields<'a>(pub &'a [u8]);
impl<'a> Fields<'a> {
    pub fn take(&mut self, count: usize) -> io::Result<&'a [u8]> {
        if count > self.0.len() {
            return Err(invalid("truncated SFTP fields"));
        }
        let (value, rest) = self.0.split_at(count);
        self.0 = rest;
        Ok(value)
    }
    pub fn u32(&mut self) -> io::Result<u32> {
        Ok(u32::from_be_bytes(
            self.take(4)?.try_into().map_err(|_| invalid("u32"))?,
        ))
    }
    pub fn u64(&mut self) -> io::Result<u64> {
        Ok(u64::from_be_bytes(
            self.take(8)?.try_into().map_err(|_| invalid("u64"))?,
        ))
    }
    pub fn string(&mut self) -> io::Result<&'a [u8]> {
        let len = self.u32()? as usize;
        self.take(len)
    }
    pub fn end(&self) -> io::Result<()> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err(invalid("trailing SFTP fields"))
        }
    }
}
#[derive(Debug, Default)]
pub struct Attrs {
    pub size: Option<u64>,
    pub permissions: Option<u32>,
    pub modified: Option<u32>,
}
impl Attrs {
    pub(super) fn regular_size(&self) -> io::Result<u64> {
        let mode = self
            .permissions
            .ok_or_else(|| invalid("missing file type"))?;
        if mode & 0o170000 != 0o100000 {
            return Err(io::Error::other("object is not a regular file"));
        }
        self.size.ok_or_else(|| invalid("missing file size"))
    }
    pub fn decode(fields: &mut Fields<'_>) -> io::Result<Self> {
        let flags = fields.u32()?;
        if flags & !0x8000000f != 0 {
            return Err(invalid("unknown SFTP attributes"));
        }
        let size = if flags & 1 != 0 {
            Some(fields.u64()?)
        } else {
            None
        };
        if flags & 2 != 0 {
            fields.take(8)?;
        }
        let permissions = if flags & 4 != 0 {
            Some(fields.u32()?)
        } else {
            None
        };
        let modified = if flags & 8 != 0 {
            fields.u32()?;
            Some(fields.u32()?)
        } else {
            None
        };
        if flags & 0x80000000 != 0 {
            for _ in 0..fields.u32()? {
                fields.string()?;
                fields.string()?;
            }
        }
        Ok(Self {
            size,
            permissions,
            modified,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sftp_packet_fixtures() {
        // Literal v3 wire fixtures, with request ID 7 and real type-specific bodies.
        for (kind, hex) in [
            (1, "000000050100000003"),
            (
                2,
                "00000026020000000300000018706f7369782d72656e616d65406f70656e7373682e636f6d0000000131",
            ),
            (3, "000000130300000007000000022f700000000100000000"),
            (4, "0000000a04000000070000000168"),
            (5, "0000001605000000070000000168000000000000800000008000"),
            (
                6,
                "0000001b060000000700000001680000000000000000000000056279746573",
            ),
            (7, "0000000b0700000007000000022f70"),
            (8, "0000000a08000000070000000168"),
            (11, "0000000b0b00000007000000022f70"),
            (12, "0000000a0c000000070000000168"),
            (13, "0000000b0d00000007000000022f70"),
            (14, "0000000f0e00000007000000022f7000000000"),
            (18, "000000111200000007000000022f70000000022f71"),
            (
                101,
                "0000001a650000000700000002000000096e6f7420666f756e6400000000",
            ),
            (102, "0000000a66000000070000000168"),
            (103, "0000000e6700000007000000056279746573"),
            (
                104,
                "000000366800000007000000010000000466696c65000000096c6f6e672066696c650000000d0000000000000006000081800000006400000065",
            ),
            (
                105,
                "0000001d69000000070000000d0000000000000006000081800000006400000065",
            ),
            (
                200,
                "0000002dc80000000700000018706f7369782d72656e616d65406f70656e7373682e636f6d000000022f70000000022f71",
            ),
        ] {
            let fixture = hex::decode(hex).unwrap();
            let packet = Packet::read(&mut &fixture[..]).unwrap();
            assert_eq!(packet.kind, kind);
            assert_eq!(packet.body, &fixture[5..]);
            let mut encoded = Vec::new();
            packet.write(&mut encoded).unwrap();
            assert_eq!(encoded, fixture);
        }
    }
    #[test]
    fn sftp_rejects_bad_packets_before_reading_payload() {
        for bytes in [
            &[0, 0, 0][..],
            &[0, 0, 0, 2, 101],
            &[0, 4, 0, 1],
            &[0, 0, 0, 0],
            &[0, 0, 0, 1, 99],
        ] {
            assert!(Packet::read(&mut &bytes[..]).is_err());
        }
        assert!(Fields(&[0, 0, 0, 10, 1]).string().is_err());
        assert!(Attrs::decode(&mut Fields(&[0, 0, 0, 16])).is_err());
    }
}

#[cfg(test)]
mod field_tests {
    use super::*;
    #[test]
    fn sftp_attributes_fixture_and_packet_boundary() {
        let fixture = hex::decode("8000000f0000000000000006000003e8000003e80000818000000064000000650000000100000001780000000179").unwrap();
        let mut fields = Fields(&fixture);
        let attrs = Attrs::decode(&mut fields).unwrap();
        fields.end().unwrap();
        assert_eq!(attrs.size, Some(6));
        assert_eq!(attrs.permissions, Some(0o100600));
        assert_eq!(attrs.modified, Some(101));
        let mut bytes = Vec::new();
        Packet {
            kind: 103,
            body: vec![0; MAX_PACKET - 1],
        }
        .write(&mut bytes)
        .unwrap();
        assert_eq!(
            Packet::read(&mut &bytes[..]).unwrap().body.len(),
            MAX_PACKET - 1
        );
        bytes.clear();
        assert!(
            Packet {
                kind: 103,
                body: vec![0; MAX_PACKET]
            }
            .write(&mut bytes)
            .is_err()
        );
        assert!(bytes.is_empty());
        assert_eq!(
            Packet::read(&mut &[0, 4, 0, 1][..]).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }
}
