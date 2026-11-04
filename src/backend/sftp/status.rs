use super::codec::{self, Fields, Packet};
use std::io;
#[derive(Debug, thiserror::Error)]
#[error("SFTP status {code}: {message}")]
pub(super) struct RemoteStatus {
    pub code: u32,
    message: String,
}
pub(super) fn unresolved_link(error: &io::Error) -> bool {
    // v3 has no ELOOP status: OpenSSH reports loops as generic FAILURE (4).
    error
        .get_ref()
        .and_then(|e| e.downcast_ref::<RemoteStatus>())
        .is_some_and(|s| matches!(s.code, 2 | 4))
}
pub(super) fn status(packet: &Packet) -> io::Result<()> {
    if packet.kind != 101 {
        return Err(codec::invalid("expected SFTP STATUS"));
    }
    let mut fields = Fields(&packet.body);
    let code = fields.u32()?;
    let message = String::from_utf8_lossy(fields.string()?).into_owned();
    fields.string()?;
    fields.end()?;
    if code == 0 {
        return Ok(());
    }
    let kind = match code {
        1 => io::ErrorKind::UnexpectedEof,
        2 => io::ErrorKind::NotFound,
        3 => io::ErrorKind::PermissionDenied,
        8 => io::ErrorKind::Unsupported,
        _ => io::ErrorKind::Other,
    };
    Err(io::Error::new(kind, RemoteStatus { code, message }))
}
pub(super) fn payload(packet: &Packet, kind: u8) -> io::Result<&[u8]> {
    if packet.kind == 101 {
        status(packet)?;
        return Err(codec::invalid("unexpected successful SFTP STATUS"));
    }
    if packet.kind != kind {
        return Err(codec::invalid("unexpected SFTP response type"));
    }
    Ok(&packet.body)
}
