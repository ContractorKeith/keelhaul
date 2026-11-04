use super::{ParseError, require, valid_id, validate_time};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Summary {
    pub files_new: u64,
    pub files_changed: u64,
    pub files_unchanged: u64,
    pub dirs: u64,
    pub files_skipped: u64,
    pub data_blobs_added: u64,
    pub bytes_added: u64,
    pub bytes_processed: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub version: u32,
    pub time: String,
    pub host: String,
    pub username: String,
    pub paths: Vec<String>,
    pub tags: Vec<String>,
    pub excludes: Vec<String>,
    pub tree: String,
    #[serde(deserialize_with = "required_parent")]
    pub parent: Option<String>,
    pub program: String,
    pub summary: Summary,
}
impl Snapshot {
    pub fn parse(bytes: &[u8]) -> Result<Self, ParseError> {
        let value: Self = serde_json::from_slice(bytes)?;
        require(value.version == 1, "unknown snapshot version")?;
        validate_time(&value.time)?;
        require(
            valid_id(&value.tree) && value.parent.as_ref().is_none_or(|p| valid_id(p)),
            "invalid snapshot reference",
        )?;
        require(!value.paths.is_empty(), "snapshot has no paths")?;
        for path in &value.paths {
            require(
                clean_absolute(path),
                "snapshot path must be absolute and clean",
            )?;
        }
        require(
            value.paths.windows(2).all(|p| p[0] < p[1]),
            "snapshot paths must be sorted and unique",
        )?;
        Ok(value)
    }
}
pub fn clean_absolute(path: &str) -> bool {
    path == "/"
        || (path.starts_with('/')
            && path[1..]
                .split('/')
                .all(|c| !c.is_empty() && c != "." && c != ".." && !c.contains('\0')))
}
pub fn root_name(path: &str) -> String {
    path.replace('%', "%25").replace('/', "%2F")
}
pub fn decode_root(name: &str) -> Result<String, ParseError> {
    let mut raw = Vec::new();
    let b = name.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let escaped = b
                .get(i..i + 3)
                .ok_or_else(|| ParseError("invalid root escape".into()))?;
            match escaped {
                b"%2F" => raw.push(b'/'),
                b"%25" => raw.push(b'%'),
                _ => return Err(ParseError("invalid root escape".into())),
            }
            i += 3;
        } else {
            raw.push(b[i]);
            i += 1;
        }
    }
    let path = String::from_utf8(raw).map_err(|e| ParseError(e.to_string()))?;
    require(
        clean_absolute(&path) && root_name(&path) == name,
        "invalid encoded root path",
    )?;
    Ok(path)
}

fn required_parent<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    Option::deserialize(deserializer)
}
