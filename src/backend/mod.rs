//! Blocking storage of validated repository object names and opaque bytes.
mod boxed;
#[cfg(any(test, feature = "fault-injection"))]
pub mod fault;
mod foreign;
pub mod local;
mod local_fs;
mod local_scan;
pub mod memory;
pub mod s3;
pub mod sftp;

use std::{
    fmt, io,
    path::{Path, PathBuf},
    time::SystemTime,
};

#[derive(Debug, thiserror::Error)]
pub enum BackendError {
    #[error("bucket not found: {0}")]
    BucketNotFound(String),
    #[error("invalid configuration: {0}")]
    InvalidConfig(&'static str),
    #[error("object not found: {0}")]
    NotFound(ObjectName),
    #[error("object already exists with different content: {0}")]
    AlreadyExists(ObjectName),
    #[error("I/O error for {name:?} at {path:?}: {source}")]
    Io {
        name: Option<ObjectName>,
        path: Option<PathBuf>,
        #[source]
        source: io::Error,
    },
    #[error("invalid object name: {0}")]
    InvalidName(String),
}

impl From<io::Error> for BackendError {
    fn from(source: io::Error) -> Self {
        Self::Io {
            name: None,
            path: None,
            source,
        }
    }
}

impl BackendError {
    pub(crate) fn context(self, name: Option<&ObjectName>, path: Option<&Path>) -> Self {
        match self {
            Self::Io {
                name: old_name,
                path: old_path,
                source,
            } => Self::Io {
                name: old_name.or_else(|| name.cloned()),
                path: old_path.or_else(|| path.map(Path::to_path_buf)),
                source,
            },
            other => other,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ObjectName(String);

impl ObjectName {
    pub fn new(name: &str) -> Result<Self, BackendError> {
        let parts: Vec<_> = name.split('/').collect();
        let valid = match parts.as_slice() {
            ["config"] => true,
            ["keys" | "index" | "snapshots" | "locks", id] => valid_hex(id, 64),
            ["data", prefix, id] => {
                valid_hex(prefix, 2) && valid_hex(id, 64) && id.starts_with(prefix)
            }
            _ => false,
        };
        if valid {
            Ok(Self(name.to_owned()))
        } else {
            Err(BackendError::InvalidName(name.to_owned()))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn kind(&self) -> Kind {
        // Construction has validated the complete namespace.
        match self.0.split('/').next() {
            Some("config") => Kind::Config,
            Some("keys") => Kind::Keys,
            Some("data") => Kind::Data,
            Some("index") => Kind::Index,
            Some("snapshots") => Kind::Snapshots,
            Some("locks") => Kind::Locks,
            _ => unreachable!("ObjectName is validated at construction"),
        }
    }
}

impl fmt::Display for ObjectName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

fn valid_hex(s: &str, len: usize) -> bool {
    s.len() == len
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Config,
    Keys,
    Data,
    Index,
    Snapshots,
    Locks,
}

impl Kind {
    pub const ALL: [Self; 6] = [
        Self::Config,
        Self::Keys,
        Self::Data,
        Self::Index,
        Self::Snapshots,
        Self::Locks,
    ];

    pub(crate) fn directory(self) -> &'static str {
        match self {
            Self::Config => "",
            Self::Keys => "keys",
            Self::Data => "data",
            Self::Index => "index",
            Self::Snapshots => "snapshots",
            Self::Locks => "locks",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: ObjectName,
    pub size: u64,
    pub modified: Option<SystemTime>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForeignEntry {
    /// Display name; lossy or otherwise unsafe names cannot be deleted.
    pub raw: String,
    pub deletable: bool,
    pub size: u64,
    pub modified: Option<SystemTime>,
}

/// Writes are create-once: an identical retry succeeds; different bytes fail.
/// Ranges are clipped at EOF, including offsets beyond EOF. Lists are sorted by name.
pub trait Backend: Send + Sync {
    /// Whether a local secret destination lies inside this repository.
    /// Remote storage cannot contain a local filesystem destination.
    fn contains_destination(&self, _path: &Path) -> Result<bool, BackendError> {
        Ok(false)
    }
    fn write(&self, name: &ObjectName, bytes: &[u8]) -> Result<(), BackendError>;
    fn read(&self, name: &ObjectName) -> Result<Vec<u8>, BackendError>;
    fn read_range(&self, name: &ObjectName, offset: u64, len: u64)
    -> Result<Vec<u8>, BackendError>;
    fn list(&self, kind: Kind) -> Result<Vec<Entry>, BackendError>;
    fn list_foreign(&self, kind: Kind) -> Result<Vec<ForeignEntry>, BackendError>;
    /// Deletes safe, non-object names only at the fixed repository object depths.
    /// Both deletion operations are idempotent when the target is missing.
    fn delete_foreign(&self, raw: &str) -> Result<(), BackendError>;
    fn stat(&self, name: &ObjectName) -> Result<u64, BackendError>;
    fn delete(&self, name: &ObjectName) -> Result<(), BackendError>;
}

pub(crate) fn object_error(name: &ObjectName, path: &Path, error: io::Error) -> BackendError {
    if error.kind() == io::ErrorKind::NotFound {
        BackendError::NotFound(name.clone())
    } else {
        BackendError::from(error).context(Some(name), Some(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_spec_names() {
        for kind in ["keys", "index", "snapshots", "locks"] {
            assert!(ObjectName::new(&format!("{kind}/{}", "ab".repeat(32))).is_ok());
        }
        assert!(ObjectName::new("config").is_ok());
        assert!(ObjectName::new(&format!("data/ab/{}", "ab".repeat(32))).is_ok());
        for name in [
            "",
            "/config",
            "config/",
            "../config",
            "keys/..",
            "keys/abc",
            "config.tmp-0123456789abcdef",
            "data/ab/../config",
            &format!("data/aa/{}", "ab".repeat(32)),
            &format!("keys/{}", "AB".repeat(32)),
            &format!("keys/{}/extra", "ab".repeat(32)),
            &format!("keys/{}.tmp-0123456789abcdef", "ab".repeat(32)),
        ] {
            assert!(
                matches!(ObjectName::new(name), Err(BackendError::InvalidName(_))),
                "{name}"
            );
        }
    }
}
