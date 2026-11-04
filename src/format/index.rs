use super::{
    ParseError,
    pack::{Blob, validate_blobs},
    require, valid_id,
};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pack {
    pub id: String,
    pub blobs: Vec<Blob>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Index {
    pub version: u32,
    pub supersedes: Vec<String>,
    pub packs: Vec<Pack>,
}
impl Index {
    pub fn parse(bytes: &[u8]) -> Result<Self, ParseError> {
        let value: Self = serde_json::from_slice(bytes)?;
        require(value.version == 1, "unknown index version")?;
        for id in &value.supersedes {
            require(valid_id(id), "invalid superseded index ID")?;
        }
        for pack in &value.packs {
            require(valid_id(&pack.id), "invalid pack ID")?;
            validate_blobs(&pack.blobs, None)?;
        }
        Ok(value)
    }
}
