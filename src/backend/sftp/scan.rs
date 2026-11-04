use super::{
    SftpBackend, client::Client, codec::Attrs, operations::DirectoryEntry, status::unresolved_link,
};
use crate::backend::{BackendError, ForeignEntry, Kind, ObjectName, foreign, valid_hex};
use std::{
    ffi::OsStr,
    io,
    os::unix::ffi::OsStrExt,
    path::Path,
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
            deletable: foreign::validate(&raw).is_ok() && file_type(&attrs) != Some(0o040000),
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
fn children_inner(
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
fn finish_at<T>(
    client: &mut Client,
    path: &[u8],
    result: io::Result<T>,
) -> Result<T, BackendError> {
    client.finish(result).map_err(|error| {
        BackendError::from(error).context(None, Some(Path::new(OsStr::from_bytes(path))))
    })
}
fn children(
    client: &mut Client,
    path: &str,
    raw: &str,
    entries: &mut Vec<ScannedEntry>,
) -> Result<Vec<DirectoryEntry>, BackendError> {
    let result = children_inner(client, path, raw, entries);
    finish_at(client, path.as_bytes(), result)
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
        self.scan_with(&mut client, kind)
    }
    fn scan_with(
        &self,
        client: &mut Client,
        kind: Kind,
    ) -> Result<Vec<ScannedEntry>, BackendError> {
        let mut result = Vec::new();
        let mut directories = vec![kind.directory().to_owned()];
        if kind == Kind::Data {
            directories.clear();
            for entry in children(client, &self.path("data"), "data", &mut result)? {
                if entry.name == b"."
                    || entry.name == b".."
                    || entry.name.contains(&b'/')
                    || entry.name.contains(&0)
                {
                    continue;
                }
                let shard = String::from_utf8_lossy(&entry.name);
                let raw = format!("data/{shard}");
                let mut path = self.path("data").into_bytes();
                path.push(b'/');
                path.extend(&entry.name);
                let resolved = resolve(client, &path, entry.attrs);
                let (attrs, live) = finish_at(client, &path, resolved)?;
                if live && valid_hex(&shard, 2) && file_type(&attrs) == Some(0o040000) {
                    directories.push(raw);
                } else {
                    let recorded = record(raw, attrs, false, &mut result);
                    finish_at(client, &path, recorded)?;
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
                let resolved = resolve(client, &path, entry.attrs);
                let (attrs, live) = finish_at(client, &path, resolved)?;
                if kind == Kind::Config
                    && file_type(&attrs) == Some(0o040000)
                    && matches!(
                        raw.as_str(),
                        "keys" | "data" | "index" | "snapshots" | "locks"
                    )
                {
                    continue;
                }
                let regular = file_type(&attrs) == Some(0o100000);
                let recorded = record(raw, attrs, live && lossless && regular, &mut result);
                finish_at(client, &path, recorded)?;
            }
        }
        result.sort_by(|a, b| a.foreign.raw.cmp(&b.foreign.raw));
        Ok(result)
    }
}
