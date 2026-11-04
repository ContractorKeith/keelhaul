use super::local_fs::{TempFile, at, ensure_directory, lock_result};
use super::local_scan;
use super::{Backend, BackendError, Entry, ForeignEntry, Kind, ObjectName, foreign, object_error};
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
        ensure_directory(root.as_ref(), root.as_ref())?;
        Ok(Self {
            root: root.as_ref().to_path_buf(),
        })
    }

    /// Open an existing repository without creating any directories.
    pub fn open(root: impl AsRef<Path>) -> Result<Self, BackendError> {
        let backend = Self {
            root: root.as_ref().to_path_buf(),
        };
        let config = ObjectName::new("config")?;
        let metadata =
            fs::metadata(&backend.root).map_err(|e| object_error(&config, &backend.root, e))?;
        if metadata.is_dir() {
            for marker in ["config", "keys", "data", "index", "snapshots"] {
                let path = backend.root.join(marker);
                match fs::metadata(&path) {
                    Ok(_) => return Ok(backend),
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(at(&path, error)),
                }
            }
        }
        Err(BackendError::NotFound(config))
    }

    /// Resolve symlinked parents before deciding whether a new secret would
    /// be stored inside this local repository.
    pub fn contains_destination(&self, path: &Path) -> Result<bool, BackendError> {
        let root = fs::canonicalize(&self.root).map_err(|e| at(&self.root, e))?;
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let parent = fs::canonicalize(parent).map_err(|e| at(parent, e))?;
        Ok(parent.starts_with(root))
    }

    fn path(&self, name: &ObjectName) -> PathBuf {
        self.root.join(name.as_str())
    }

    fn write_object(
        &self,
        name: &ObjectName,
        path: &Path,
        bytes: &[u8],
    ) -> Result<(), BackendError> {
        let parent = path
            .parent()
            .expect("validated object has a parent directory");
        ensure_directory(parent, &self.root)?;
        let directory = File::open(parent).map_err(|e| at(parent, e))?;
        lock_result(directory.lock()).map_err(|e| at(parent, e))?;
        match fs::metadata(path) {
            Ok(metadata) => {
                if metadata.len() != bytes.len() as u64 {
                    return Err(BackendError::AlreadyExists(name.clone()));
                }
                let mut existing = File::open(path)?;
                let mut stored = Vec::new();
                existing.read_to_end(&mut stored)?;
                if stored != bytes {
                    return Err(BackendError::AlreadyExists(name.clone()));
                }
                existing.sync_all()?;
                directory.sync_all()?;
                return Ok(());
            }
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
            .open(&temp)
            .map_err(|e| at(&temp, e))?;
        let temp = TempFile(temp);
        file.write_all(bytes)
            .and_then(|()| file.sync_all())
            .map_err(|e| at(&temp.0, e))?;
        fs::rename(&temp.0, path)?;
        directory.sync_all()?;
        Ok(())
    }

    fn delete_path(&self, path: &Path) -> Result<(), BackendError> {
        let parent = path
            .parent()
            .expect("validated path has a parent directory");
        let directory = match File::open(parent) {
            Ok(directory) => directory,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(at(parent, error)),
        };
        lock_result(directory.lock()).map_err(|e| at(parent, e))?;
        match fs::remove_file(path) {
            Ok(()) => directory.sync_all().map_err(|e| at(parent, e)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(at(path, error)),
        }
    }
}

impl Backend for LocalBackend {
    fn contains_destination(&self, path: &Path) -> Result<bool, BackendError> {
        LocalBackend::contains_destination(self, path)
    }

    fn write(&self, name: &ObjectName, bytes: &[u8]) -> Result<(), BackendError> {
        let path = self.path(name);
        self.write_object(name, &path, bytes)
            .map_err(|e| e.context(Some(name), Some(&path)))
    }

    fn read(&self, name: &ObjectName) -> Result<Vec<u8>, BackendError> {
        let path = self.path(name);
        fs::read(&path).map_err(|e| object_error(name, &path, e))
    }

    fn read_range(
        &self,
        name: &ObjectName,
        offset: u64,
        len: u64,
    ) -> Result<Vec<u8>, BackendError> {
        let path = self.path(name);
        let read = || -> io::Result<Vec<u8>> {
            let mut file = File::open(&path)?;
            let size = file.metadata()?.len();
            file.seek(SeekFrom::Start(offset.min(size)))?;
            let mut bytes = Vec::new();
            file.take(len).read_to_end(&mut bytes)?;
            Ok(bytes)
        };
        read().map_err(|e| object_error(name, &path, e))
    }

    fn list(&self, kind: Kind) -> Result<Vec<Entry>, BackendError> {
        Ok(local_scan::scan(&self.root, kind)?
            .into_iter()
            .filter_map(|entry| {
                let name = entry.object?;
                (name.kind() == kind).then_some(Entry {
                    name,
                    size: entry.foreign.size,
                    modified: entry.foreign.modified,
                })
            })
            .collect())
    }

    fn list_foreign(&self, kind: Kind) -> Result<Vec<ForeignEntry>, BackendError> {
        Ok(local_scan::scan(&self.root, kind)?
            .into_iter()
            .filter(|entry| entry.object.is_none())
            .map(|entry| entry.foreign)
            .collect())
    }

    fn delete_foreign(&self, raw: &str) -> Result<(), BackendError> {
        foreign::validate(raw)?;
        self.delete_path(&self.root.join(raw))
    }

    fn stat(&self, name: &ObjectName) -> Result<u64, BackendError> {
        let path = self.path(name);
        fs::metadata(&path)
            .map(|metadata| metadata.len())
            .map_err(|e| object_error(name, &path, e))
    }

    fn delete(&self, name: &ObjectName) -> Result<(), BackendError> {
        let path = self.path(name);
        self.delete_path(&path)
            .map_err(|e| e.context(Some(name), Some(&path)))
    }
}
