use super::{BackendError, ForeignEntry, Kind, ObjectName, foreign, local_fs::at};
use std::{
    fs::{self, Metadata},
    io,
    path::Path,
};

pub(super) struct ScannedEntry {
    pub object: Option<ObjectName>,
    pub foreign: ForeignEntry,
}

// The storage layout is trusted: follow links, but only at the fixed layout depths.
pub(super) fn scan(root: &Path, kind: Kind) -> Result<Vec<ScannedEntry>, BackendError> {
    let directory = root.join(kind.directory());
    let mut entries = Vec::new();
    if kind == Kind::Data {
        for child in children(root, &directory, &mut entries)? {
            let path = child.path();
            let valid_shard = child
                .file_name()
                .to_str()
                .is_some_and(|s| super::valid_hex(s, 2));
            match fs::metadata(&path) {
                Ok(metadata) if valid_shard && metadata.is_dir() => {
                    scan_leaves(root, &path, kind, &mut entries)?
                }
                Ok(metadata) => record(root, &path, Some(metadata), None, &mut entries)?,
                Err(error) if broken_link(&path, &error) => {
                    record(root, &path, None, None, &mut entries)?
                }
                Err(error) => return Err(at(&path, error)),
            }
        }
    } else {
        scan_leaves(root, &directory, kind, &mut entries)?;
    }
    entries.sort_by(|a, b| a.foreign.raw.cmp(&b.foreign.raw));
    Ok(entries)
}

fn scan_leaves(
    root: &Path,
    directory: &Path,
    kind: Kind,
    entries: &mut Vec<ScannedEntry>,
) -> Result<(), BackendError> {
    for child in children(root, directory, entries)? {
        let path = child.path();
        match fs::metadata(&path) {
            Ok(metadata) if metadata.is_file() => {
                let relative = path.strip_prefix(root).expect("scan stays under root");
                let object = relative
                    .to_str()
                    .and_then(|raw| ObjectName::new(raw).ok())
                    .filter(|name| name.kind() == kind);
                record(root, &path, Some(metadata), object, entries)?;
            }
            Ok(metadata) => {
                // Layout directories are traversed by their own kind. Object-depth
                // directories are foreign entries; never descend into them.
                if kind == Kind::Config
                    && child.file_name().to_str().is_some_and(|n| {
                        matches!(n, "keys" | "data" | "index" | "snapshots" | "locks")
                    })
                    && metadata.is_dir()
                {
                    continue;
                }
                record(root, &path, Some(metadata), None, entries)?;
            }
            Err(error) if broken_link(&path, &error) => record(root, &path, None, None, entries)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(at(&path, error)),
        }
    }
    Ok(())
}

fn children(
    root: &Path,
    directory: &Path,
    entries: &mut Vec<ScannedEntry>,
) -> Result<Vec<fs::DirEntry>, BackendError> {
    match fs::read_dir(directory) {
        Ok(children) => children
            .map(|entry| entry.map_err(|e| at(directory, e)))
            .collect(),
        Err(error) if broken_link(directory, &error) => {
            record(root, directory, None, None, entries)?;
            Ok(Vec::new())
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(at(directory, error)),
    }
}

fn broken_link(path: &Path, error: &io::Error) -> bool {
    // ELOOP is 40 on Linux and 62 on macOS.
    #[cfg(target_os = "linux")]
    const ELOOP: i32 = 40;
    #[cfg(target_os = "macos")]
    const ELOOP: i32 = 62;
    (error.kind() == io::ErrorKind::NotFound || error.raw_os_error() == Some(ELOOP))
        && fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink())
}

fn record(
    root: &Path,
    path: &Path,
    metadata: Option<Metadata>,
    object: Option<ObjectName>,
    entries: &mut Vec<ScannedEntry>,
) -> Result<(), BackendError> {
    let relative = path.strip_prefix(root).expect("scan stays under root");
    let lossless = relative.to_str().is_some();
    let raw = relative.to_string_lossy().into_owned();
    let deletable = lossless && foreign::validate(&raw).is_ok();
    let metadata = match metadata {
        Some(metadata) => metadata,
        None => match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(at(path, error)),
        },
    };
    entries.push(ScannedEntry {
        object,
        foreign: ForeignEntry {
            raw,
            deletable: deletable && !metadata.is_dir(),
            size: metadata.len(),
            modified: metadata.modified().ok(),
        },
    });
    Ok(())
}
