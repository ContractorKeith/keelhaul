//! Validate a whole response before any operation consumes it.
use super::codec::{self, Attrs, Fields, Packet};
use std::io;
pub(super) fn validate(request: u8, packet: &Packet) -> io::Result<()> {
    let expected = match request {
        3 | 11 => 102,
        5 => 103,
        7 | 8 | 17 => 105,
        12 => 104,
        _ => 101,
    };
    if packet.kind != 101 && packet.kind != expected {
        return Err(codec::invalid("unexpected SFTP response type"));
    }
    let mut fields = Fields(&packet.body);
    match packet.kind {
        101 => {
            let code = fields.u32()?;
            if code == 0 && expected != 101 {
                return Err(codec::invalid("unexpected successful SFTP STATUS"));
            }
            fields.string()?;
            fields.string()?;
        }
        102 | 103 => {
            fields.string()?;
        }
        104 => {
            let count = fields.u32()?;
            if count == 0 {
                return Err(codec::invalid("empty SFTP NAME page"));
            }
            for _ in 0..count {
                fields.string()?;
                fields.string()?;
                Attrs::decode(&mut fields)?;
            }
        }
        105 => {
            Attrs::decode(&mut fields)?;
        }
        _ => return Err(codec::invalid("unexpected SFTP packet")),
    }
    fields.end()
}
