use super::{BackendError, ForeignEntry, Kind};
use std::{
    fs::{self, File},
    io,
    path::{Path, PathBuf},
};

pub(super) fn at(path: &Path, error: io::Error) -> BackendError {
    BackendError::from(error).context(None, Some(path))
}

// Existing directories need no sync. Root creation tolerates an unreadable parent.
pub(super) fn ensure_directory(path: &Path, root: &Path) -> Result<(), BackendError> {
    if path.is_dir() {
        return Ok(());
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    if path == root {
        fs::create_dir_all(path).map_err(|e| at(path, e))?;
    } else {
        ensure_directory(parent, root)?;
        match fs::create_dir(path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists && path.is_dir() => {
                return Ok(());
            }
            Err(error) => return Err(at(path, error)),
        }
    }
    File::open(path)
        .and_then(|dir| dir.sync_all())
        .map_err(|e| at(path, e))?;
    match File::open(parent).and_then(|dir| dir.sync_all()) {
        Err(error) if path == root && error.kind() == io::ErrorKind::PermissionDenied => Ok(()),
        result => result.map_err(|e| at(parent, e)),
    }
}

pub(super) fn scan(root: &Path, kind: Kind) -> Result<Vec<ForeignEntry>, BackendError> {
    let mut entries = Vec::new();
    let mut pending = vec![root.join(kind.directory())];
    while let Some(directory) = pending.pop() {
        let children = match fs::read_dir(&directory) {
            Ok(children) => children,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(at(&directory, error)),
        };
        for child in children {
            let child = child.map_err(|e| at(&directory, e))?;
            let path = child.path();
            let metadata = match fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(at(&path, error)),
            };
            if metadata.is_dir() && kind != Kind::Config {
                pending.push(path);
            } else if metadata.is_file() {
                let relative = path
                    .strip_prefix(root)
                    .expect("scan visits only paths under root");
                if let Some(raw) = relative.to_str() {
                    entries.push(ForeignEntry {
                        raw: raw.to_owned(),
                        size: metadata.len(),
                        modified: metadata.modified().ok(),
                    });
                }
            }
        }
    }
    entries.sort_by(|a, b| a.raw.cmp(&b.raw));
    Ok(entries)
}

// Reject directory symlinks so raw deletion cannot escape into another directory.
pub(super) fn check_parents(root: &Path, raw: &str) -> Result<(), BackendError> {
    let mut path = root.to_path_buf();
    let parts: Vec<_> = raw.split('/').collect();
    for part in &parts[..parts.len() - 1] {
        path.push(part);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(BackendError::InvalidName(raw.to_owned()));
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(at(&path, error)),
        }
    }
    Ok(())
}

pub(super) fn lock_result(result: io::Result<()>) -> io::Result<()> {
    // NFS/CIFS may lack locks; rename stays atomic for identical content-addressed writes.
    #[cfg(target_os = "linux")]
    const UNAVAILABLE: [i32; 2] = [37, 95]; // ENOLCK, EOPNOTSUPP
    #[cfg(target_os = "macos")]
    const UNAVAILABLE: [i32; 2] = [77, 45]; // ENOLCK, EOPNOTSUPP
    match result {
        Err(error)
            if error.kind() == io::ErrorKind::Unsupported
                || error
                    .raw_os_error()
                    .is_some_and(|code| UNAVAILABLE.contains(&code)) =>
        {
            Ok(())
        }
        other => other,
    }
}

pub(super) struct TempFile(pub PathBuf);
impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unavailable_locks_are_optional_but_other_errors_propagate() {
        #[cfg(target_os = "linux")]
        let codes = [37, 95];
        #[cfg(target_os = "macos")]
        let codes = [77, 45];
        for code in codes {
            assert!(lock_result(Err(io::Error::from_raw_os_error(code))).is_ok());
        }
        assert!(lock_result(Err(io::ErrorKind::Unsupported.into())).is_ok());
        assert!(lock_result(Ok(())).is_ok());
        assert_eq!(
            lock_result(Err(io::ErrorKind::PermissionDenied.into()))
                .unwrap_err()
                .kind(),
            io::ErrorKind::PermissionDenied
        );
        assert_eq!(
            lock_result(Err(io::ErrorKind::Other.into()))
                .unwrap_err()
                .kind(),
            io::ErrorKind::Other
        );
    }
}
