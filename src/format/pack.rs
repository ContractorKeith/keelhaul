use super::{ParseError, require, valid_id};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BlobKind {
    Data,
    Tree,
}
impl BlobKind {
    pub fn label(self) -> crate::crypto::envelope::Label {
        match self {
            Self::Data => crate::crypto::envelope::Label::Data,
            Self::Tree => crate::crypto::envelope::Label::Tree,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Blob {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: BlobKind,
    pub offset: u64,
    pub length: u64,
    pub raw_length: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Header {
    pub version: u32,
    pub blobs: Vec<Blob>,
}
pub const MAX_HEADER: u64 = 16 * 1024 * 1024;
impl Header {
    pub fn parse(payload: &[u8], data_len: u64) -> Result<Self, ParseError> {
        let header: Self = serde_json::from_slice(payload)?;
        require(header.version == 1, "unknown pack version")?;
        validate_blobs(&header.blobs, Some(data_len))?;
        Ok(header)
    }
}
pub fn validate_blobs(blobs: &[Blob], data_len: Option<u64>) -> Result<(), ParseError> {
    require(!blobs.is_empty(), "pack has no blobs")?;
    let mut end = 0;
    let mut seen = std::collections::HashSet::new();
    for blob in blobs {
        require(valid_id(&blob.id), "invalid blob ID")?;
        require(seen.insert((&blob.id, blob.kind)), "duplicate blob")?;
        require(blob.offset == end, "blob coverage has gap or overlap")?;
        require(
            blob.length >= 41 && blob.length <= blob.raw_length.saturating_add(41),
            "invalid blob envelope length",
        )?;
        require(
            blob.raw_length <= crate::crypto::envelope::MAX_PAYLOAD,
            "blob raw length exceeds limit",
        )?;
        require(
            blob.kind != BlobKind::Data
                || (blob.raw_length > 0 && blob.raw_length <= crate::chunker::MAX as u64),
            "invalid data chunk length",
        )?;
        end = end
            .checked_add(blob.length)
            .ok_or_else(|| ParseError("blob offset overflow".into()))?;
    }
    if let Some(len) = data_len {
        require(end == len, "blob coverage does not equal pack data")?;
    }
    Ok(())
}
pub fn header_length(trailer: &[u8], pack_len: u64) -> Result<u64, ParseError> {
    let raw: [u8; 4] = trailer
        .try_into()
        .map_err(|_| ParseError("truncated pack trailer".into()))?;
    let len = u32::from_le_bytes(raw) as u64;
    require(
        (41..=MAX_HEADER).contains(&len) && pack_len.checked_sub(4).is_some_and(|n| len <= n),
        "invalid pack header length",
    )?;
    Ok(len)
}
