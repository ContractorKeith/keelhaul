use super::{SftpBackend, client::Client, operations::DirectoryEntry};
use crate::backend::{BackendError, ForeignEntry, Kind, foreign, valid_hex};
use std::{
    io,
    time::{Duration, UNIX_EPOCH},
};
fn children(client: &mut Client, path: &str) -> io::Result<Vec<DirectoryEntry>> {
    match client.directory(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        other => other,
    }
}
impl SftpBackend {
    pub(super) fn scan(&self, kind: Kind) -> Result<Vec<ForeignEntry>, BackendError> {
        let mut client = self.client()?;
        let mut directories = vec![kind.directory().to_owned()];
        if kind == Kind::Data {
            directories.clear();
            for entry in children(&mut client, &self.path("data"))? {
                if let Ok(shard) = std::str::from_utf8(&entry.name)
                    && valid_hex(shard, 2)
                    && entry
                        .attrs
                        .permissions
                        .is_some_and(|p| p & 0o170000 == 0o040000)
                {
                    directories.push(format!("data/{shard}"));
                }
            }
        }
        let mut result = Vec::new();
        for directory in directories {
            for entry in children(&mut client, &self.path(&directory))? {
                if entry.name == b"."
                    || entry.name == b".."
                    || entry.name.contains(&b'/')
                    || entry.name.contains(&0)
                {
                    continue;
                }
                if !entry
                    .attrs
                    .permissions
                    .is_some_and(|p| p & 0o170000 == 0o100000)
                {
                    continue;
                }
                let lossless = std::str::from_utf8(&entry.name).is_ok();
                let leaf = String::from_utf8_lossy(&entry.name);
                let raw = if directory.is_empty() {
                    leaf.into_owned()
                } else {
                    format!("{directory}/{leaf}")
                };
                if foreign::position(&raw) != Some(kind) {
                    continue;
                }
                result.push(ForeignEntry {
                    deletable: lossless && foreign::validate(&raw).is_ok(),
                    raw,
                    size: entry
                        .attrs
                        .size
                        .ok_or_else(|| super::codec::invalid("missing listing size"))?,
                    modified: entry
                        .attrs
                        .modified
                        .map(|s| UNIX_EPOCH + Duration::from_secs(u64::from(s))),
                });
            }
        }
        result.sort_by(|a, b| a.raw.cmp(&b.raw));
        Ok(result)
    }
}
