use super::{ParseError, require, valid_id, validate_time};
use base64::{Engine, prelude::BASE64_STANDARD};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tree {
    pub version: u32,
    pub entries: Vec<Entry>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name_b64: Option<String>,
    pub mode: u32,
    pub mtime: String,
    pub ctime: String,
    pub uid: u32,
    pub gid: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    pub inode: u64,
    pub device: u64,
    pub links: u64,
    #[serde(flatten)]
    pub node: Node,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Node {
    File {
        size: u64,
        content: Vec<String>,
    },
    Dir {
        subtree: String,
    },
    Symlink {
        #[serde(skip_serializing_if = "Option::is_none")]
        target: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        target_b64: Option<String>,
    },
}
pub fn encode_bytes(raw: &[u8]) -> (Option<String>, Option<String>) {
    match std::str::from_utf8(raw) {
        Ok(s) => (Some(s.into()), None),
        Err(_) => (None, Some(BASE64_STANDARD.encode(raw))),
    }
}
fn decode_bytes(text: &Option<String>, b64: &Option<String>) -> Result<Vec<u8>, ParseError> {
    match (text, b64) {
        (Some(text), None) => Ok(text.as_bytes().to_vec()),
        (None, Some(raw)) => {
            let bytes = BASE64_STANDARD
                .decode(raw)
                .map_err(|e| ParseError(e.to_string()))?;
            require(
                BASE64_STANDARD.encode(&bytes) == *raw,
                "noncanonical base64",
            )?;
            require(
                std::str::from_utf8(&bytes).is_err(),
                "base64 is only for non-UTF-8 bytes",
            )?;
            Ok(bytes)
        }
        _ => Err(ParseError(
            "exactly one text or base64 field is required".into(),
        )),
    }
}
impl Entry {
    pub fn name_bytes(&self) -> Result<Vec<u8>, ParseError> {
        decode_bytes(&self.name, &self.name_b64)
    }
    pub fn target_bytes(&self) -> Result<Vec<u8>, ParseError> {
        match &self.node {
            Node::Symlink { target, target_b64 } => decode_bytes(target, target_b64),
            _ => Err(ParseError("not a symlink".into())),
        }
    }
    pub fn validate(&self) -> Result<(), ParseError> {
        let name = self.name_bytes()?;
        require(
            !name.is_empty()
                && name != b"."
                && name != b".."
                && !name.contains(&b'/')
                && !name.contains(&0),
            "invalid tree name",
        )?;
        require(
            self.mode & !0o7777 == 0,
            "mode contains non-permission bits",
        )?;
        validate_time(&self.mtime)?;
        validate_time(&self.ctime)?;
        match &self.node {
            Node::File { size, content } => {
                require(
                    (*size == 0) == content.is_empty(),
                    "file size and content disagree",
                )?;
                for id in content {
                    require(valid_id(id), "invalid chunk ID")?;
                }
            }
            Node::Dir { subtree } => require(valid_id(subtree), "invalid subtree ID")?,
            Node::Symlink { .. } => {
                let target = self.target_bytes()?;
                require(
                    !target.is_empty() && !target.contains(&0),
                    "invalid symlink target",
                )?;
            }
        }
        Ok(())
    }
}
impl Tree {
    pub fn parse(bytes: &[u8]) -> Result<Self, ParseError> {
        let tree: Self = serde_json::from_slice(bytes)?;
        require(tree.version == 1, "unknown tree version")?;
        let mut last: Option<Vec<u8>> = None;
        for entry in &tree.entries {
            entry.validate()?;
            let name = entry.name_bytes()?;
            require(
                last.as_ref().is_none_or(|v| *v < name),
                "tree names must be sorted and unique",
            )?;
            last = Some(name);
        }
        Ok(tree)
    }
    /// Restore salvages individually valid entries without trusting malformed names.
    pub fn salvage(bytes: &[u8]) -> Result<(Self, Vec<String>), ParseError> {
        #[derive(Deserialize)]
        struct Raw {
            version: u32,
            entries: Vec<serde_json::Value>,
        }
        let raw: Raw = serde_json::from_slice(bytes)?;
        require(raw.version == 1, "unknown tree version")?;
        let mut tree = Self {
            version: 1,
            entries: Vec::new(),
        };
        let mut problems = Vec::new();
        let mut last: Option<Vec<u8>> = None;
        for value in raw.entries {
            let parsed = (|| {
                let entry: Entry = serde_json::from_value(value)?;
                entry.validate()?;
                let name = entry.name_bytes()?;
                require(
                    last.as_ref().is_none_or(|v| *v < name),
                    "tree names must be sorted and unique",
                )?;
                last = Some(name);
                Ok::<_, ParseError>(entry)
            })();
            match parsed {
                Ok(e) => tree.entries.push(e),
                Err(e) => problems.push(e.to_string()),
            }
        }
        Ok((tree, problems))
    }
}
