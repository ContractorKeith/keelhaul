use super::{Backend, BackendError, Entry, Kind, ObjectName, object_error};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

#[derive(Debug)]
pub struct LocalBackend {
    root: PathBuf,
}

impl LocalBackend {
    pub fn new(root: impl AsRef<Path>) -> Result<Self, BackendError> {
        ensure_directory(root.as_ref())?;
        Ok(Self {
            root: root.as_ref().to_path_buf(),
        })
    }

    fn path(&self, name: &ObjectName) -> PathBuf {
        self.root.join(name.as_str())
    }

    fn collect(
        &self,
        directory: &Path,
        kind: Kind,
        entries: &mut Vec<Entry>,
    ) -> Result<(), BackendError> {
        let children = match fs::read_dir(directory) {
            Ok(children) => children,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        for child in children {
            let child = child?;
            if !child.file_type()?.is_file() {
                continue;
            }
            let path = child.path();
            let relative = path
                .strip_prefix(&self.root)
                .expect("listing only visits directories beneath the root");
            if let Some(relative) = relative.to_str()
                && let Ok(name) = ObjectName::new(relative)
                && name.kind() == kind
            {
                match child.metadata() {
                    Ok(metadata) => entries.push(Entry {
                        name,
                        size: metadata.len(),
                    }),
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.into()),
                }
            }
        }
        Ok(())
    }
}

impl Backend for LocalBackend {
    fn write(&self, name: &ObjectName, bytes: &[u8]) -> Result<(), BackendError> {
        let path = self.path(name);
        let parent = path
            .parent()
            .expect("validated object has a parent directory");
        ensure_directory(parent)?;
        // OS directory locks serialize check + rename across independent instances
        // and processes. Closing the handle releases the lock even after a crash.
        let directory = File::open(parent)?;
        directory.lock()?;
        match fs::read(&path) {
            Ok(existing) if existing == bytes => {
                File::open(&path)?.sync_all()?;
                directory.sync_all()?;
                return Ok(());
            }
            Ok(_) => return Err(BackendError::AlreadyExists(name.clone())),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let mut nonce = [0; 8];
        getrandom::fill(&mut nonce).map_err(io::Error::other)?;
        let mut temp = path.as_os_str().to_os_string();
        temp.push(format!(".tmp-{}", hex::encode(nonce)));
        let temp = PathBuf::from(temp);
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        let cleanup = TempFile(temp);
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&cleanup.0, &path)?;
        directory.sync_all()?;
        Ok(())
    }

    fn read(&self, name: &ObjectName) -> Result<Vec<u8>, BackendError> {
        fs::read(self.path(name)).map_err(|error| object_error(name, error))
    }

    fn read_range(
        &self,
        name: &ObjectName,
        offset: u64,
        len: u64,
    ) -> Result<Vec<u8>, BackendError> {
        let mut file = File::open(self.path(name)).map_err(|error| object_error(name, error))?;
        let size = file.metadata()?.len();
        file.seek(SeekFrom::Start(offset.min(size)))?;
        let mut bytes = Vec::new();
        file.take(len).read_to_end(&mut bytes)?;
        Ok(bytes)
    }

    fn list(&self, kind: Kind) -> Result<Vec<Entry>, BackendError> {
        let directory = self.root.join(kind.directory());
        let mut entries = Vec::new();
        if kind == Kind::Data {
            let children = match fs::read_dir(&directory) {
                Ok(children) => children,
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(entries),
                Err(error) => return Err(error.into()),
            };
            for child in children {
                let child = child?;
                if child.file_type()?.is_dir()
                    && child
                        .file_name()
                        .to_str()
                        .is_some_and(|s| super::valid_hex(s, 2))
                {
                    self.collect(&child.path(), kind, &mut entries)?;
                }
            }
        } else {
            self.collect(&directory, kind, &mut entries)?;
        }
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(entries)
    }

    fn stat(&self, name: &ObjectName) -> Result<u64, BackendError> {
        fs::metadata(self.path(name))
            .map(|metadata| metadata.len())
            .map_err(|error| object_error(name, error))
    }

    fn delete(&self, name: &ObjectName) -> Result<(), BackendError> {
        let path = self.path(name);
        let parent = path
            .parent()
            .expect("validated object has a parent directory");
        let directory = File::open(parent).map_err(|error| object_error(name, error))?;
        directory.lock()?;
        fs::remove_file(path).map_err(|error| object_error(name, error))?;
        directory.sync_all()?;
        Ok(())
    }
}

// Persist each newly created directory entry, including data's shard directories.
fn ensure_directory(path: &Path) -> io::Result<()> {
    if path.is_dir() {
        // Another writer may have just created this directory. Persist its entry
        // before relying on it, even if that writer has not finished its fsync.
        File::open(path)?.sync_all()?;
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        return File::open(parent)?.sync_all();
    }
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        ensure_directory(parent)?;
    }
    match fs::create_dir(path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists && path.is_dir() => {}
        Err(error) => return Err(error),
    }
    File::open(path)?.sync_all()?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    File::open(parent)?.sync_all()
}

struct TempFile(PathBuf);
impl Drop for TempFile {
    fn drop(&mut self) {
        // Failed writes leave no object; after rename the temporary name is gone.
        let _ = fs::remove_file(&self.0);
    }
}
