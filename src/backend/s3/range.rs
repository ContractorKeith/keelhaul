use super::{error, http::Response};
use crate::backend::BackendError;

// Validate the server's clipped range; servers ignoring Range return a full body.
pub(super) fn bytes(r: Response, offset: u64, requested_end: u64) -> Result<Vec<u8>, BackendError> {
    let invalid = || error("S3 returned invalid ranged response");
    match r.status {
        200 => {
            let start = offset.min(r.bytes.len() as u64) as usize;
            let end = requested_end.saturating_add(1).min(r.bytes.len() as u64) as usize;
            Ok(r.bytes[start..end].to_vec())
        }
        206 => {
            let header = r.content_range.as_deref().ok_or_else(invalid)?;
            let (span, total) = header
                .strip_prefix("bytes ")
                .and_then(|s| s.split_once('/'))
                .ok_or_else(invalid)?;
            let (start, end) = span.split_once('-').ok_or_else(invalid)?;
            let start = start.parse::<u64>().map_err(|_| invalid())?;
            let end = end.parse::<u64>().map_err(|_| invalid())?;
            let total = total.parse::<u64>().map_err(|_| invalid())?;
            if start != offset
                || start > end
                || end >= total
                || end != requested_end.min(total.saturating_sub(1))
                || r.bytes.len() as u64 != end - start + 1
            {
                return Err(invalid());
            }
            Ok(r.bytes)
        }
        _ => Err(invalid()),
    }
}
