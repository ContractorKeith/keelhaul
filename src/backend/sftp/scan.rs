use super::{
    SftpBackend, client::Client, codec::Attrs, operations::DirectoryEntry, status::unresolved_link,
};
use crate::backend::{BackendError, ForeignEntry, Kind, ObjectName, foreign, valid_hex};
use std::{
    io,
    time::{Duration, UNIX_EPOCH},
};
pub(super) struct ScannedEntry {
    pub object: Option<ObjectName>,
    pub foreign: ForeignEntry,
}
fn file_type(attrs: &Attrs) -> Option<u32> {
    attrs.permissions.map(|p| p & 0o170000)
}
fn record(
    raw: String,
    attrs: Attrs,
    live: bool,
    entries: &mut Vec<ScannedEntry>,
) -> io::Result<()> {
    let object = if live {
        ObjectName::new(&raw).ok()
    } else {
        None
    };
    entries.push(ScannedEntry {
        object,
        foreign: ForeignEntry {
            deletable: foreign::validate(&raw).is_ok(),
            raw,
            size: attrs
                .size
                .ok_or_else(|| super::codec::invalid("missing listing size"))?,
            modified: attrs
                .modified
                .map(|s| UNIX_EPOCH + Duration::from_secs(u64::from(s))),
        },
    });
    Ok(())
}
fn children(
    client: &mut Client,
    path: &str,
    raw: &str,
    entries: &mut Vec<ScannedEntry>,
) -> io::Result<Vec<DirectoryEntry>> {
    match client.directory(path) {
        Ok(entries) => Ok(entries),
        Err(error) if unresolved_link(&error) => match client.lstat(path) {
            Ok(attrs) if file_type(&attrs) == Some(0o120000) => match client.attrs(path) {
                Err(target) if unresolved_link(&target) => {
                    record(raw.into(), attrs, false, entries)?;
                    Ok(Vec::new())
                }
                Err(target) => Err(target),
                Ok(_) => Err(error), // A live link to a non-directory is still an error.
            },
            Ok(_) if error.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
            Ok(_) => Err(error),
            Err(probe)
                if probe.kind() == io::ErrorKind::NotFound
                    && unresolved_link(&probe)
                    && error.kind() == io::ErrorKind::NotFound =>
            {
                Ok(Vec::new())
            }
            Err(probe) => Err(probe),
        },
        Err(error) => Err(error),
    }
}
// READDIR describes the link itself. Follow only links at fixed layout depths.
fn resolve(client: &mut Client, path: &[u8], attrs: Attrs) -> io::Result<(Attrs, bool)> {
    if file_type(&attrs) != Some(0o120000) {
        return Ok((attrs, true));
    }
    match client.attrs_raw(path) {
        Ok(target) => Ok((target, true)),
        Err(error) if unresolved_link(&error) => Ok((attrs, false)),
        Err(error) => Err(error),
    }
}
impl SftpBackend {
    pub(super) fn scan(&self, kind: Kind) -> Result<Vec<ScannedEntry>, BackendError> {
        let mut client = self.client()?;
        let result = self.scan_with(&mut client, kind);
        client.finish(result).map_err(BackendError::from)
    }
    fn scan_with(&self, client: &mut Client, kind: Kind) -> io::Result<Vec<ScannedEntry>> {
        let mut result = Vec::new();
        let mut directories = vec![kind.directory().to_owned()];
        if kind == Kind::Data {
            directories.clear();
            for entry in children(client, &self.path("data"), "data", &mut result)? {
                let Ok(shard) = std::str::from_utf8(&entry.name) else {
                    continue;
                };
                if !valid_hex(shard, 2) {
                    continue;
                }
                let raw = format!("data/{shard}");
                let (attrs, live) = resolve(client, self.path(&raw).as_bytes(), entry.attrs)?;
                if !live {
                    record(raw, attrs, false, &mut result)?;
                } else if file_type(&attrs) == Some(0o040000) {
                    directories.push(raw);
                }
            }
        }
        for directory in directories {
            for entry in children(client, &self.path(&directory), &directory, &mut result)? {
                if entry.name == b"."
                    || entry.name == b".."
                    || entry.name.contains(&b'/')
                    || entry.name.contains(&0)
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
                let mut path = self.path(&directory).into_bytes();
                if !path.ends_with(b"/") {
                    path.push(b'/');
                }
                path.extend_from_slice(&entry.name);
                let (attrs, live) = resolve(client, &path, entry.attrs)?;
                if live && file_type(&attrs) != Some(0o100000) {
                    continue;
                }
                record(raw, attrs, live && lossless, &mut result)?;
            }
        }
        result.sort_by(|a, b| a.foreign.raw.cmp(&b.foreign.raw));
        Ok(result)
    }
}
