//! SFTP v3 storage through the user's system ssh.
pub mod client;
mod codec;
mod operations;
mod response;
mod scan;
#[cfg(test)]
mod tests;
mod transfer;
pub mod url;
use super::{Backend, BackendError, Entry, ForeignEntry, Kind, ObjectName, foreign, object_error};
use client::Client;
use std::{
    io,
    path::Path,
    sync::{Mutex, MutexGuard},
};
use url::SftpUrl;

pub struct SftpBackend {
    root: String,
    client: Mutex<Client>,
}
impl SftpBackend {
    pub fn new(url: &str) -> Result<Self, BackendError> {
        let url = SftpUrl::parse(url)?;
        let client = Client::connect(&url)?;
        Self::with_client(url.path, client)
    }
    pub fn with_client(root: String, mut client: Client) -> Result<Self, BackendError> {
        client.mkdir_all(&root)?;
        Ok(Self {
            root: root.trim_end_matches('/').into(),
            client: Mutex::new(client),
        })
    }
    fn client(&self) -> io::Result<MutexGuard<'_, Client>> {
        self.client
            .lock()
            .map_err(|_| io::Error::other("SFTP client mutex poisoned"))
    }
    fn finish<T>(&self, result: Result<T, BackendError>) -> Result<T, BackendError> {
        if result.is_err()
            && let Ok(mut client) = self.client.lock()
        {
            // Semantic backend failures (such as conflicting bytes) also end the child.
            let _ = client.abort(io::Error::other("SFTP backend operation failed"));
        }
        result
    }
    fn path(&self, name: &str) -> String {
        format!("{}/{name}", self.root)
    }
    /// Direct storage hook for conformance fixtures, including nested foreign names.
    pub fn plant_raw(&self, raw: &str, bytes: &[u8]) -> Result<(), BackendError> {
        if raw
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == "..")
            || raw.contains(['\0', '\\'])
        {
            return Err(BackendError::InvalidName(raw.into()));
        }
        let path = self.path(raw);
        let mut client = self.client()?;
        client.mkdir_all(path.rsplit_once('/').expect("root prefix adds slash").0)?;
        client.write_file(&path, bytes)?;
        Ok(())
    }
    pub fn child_pid(&self) -> io::Result<u32> {
        Ok(self.client()?.pid())
    }
    fn write_object(&self, name: &ObjectName, bytes: &[u8]) -> Result<(), BackendError> {
        let path = self.path(name.as_str());
        let mut client = self.client()?;
        let compare = |client: &mut Client| -> Result<bool, BackendError> {
            match client.read_optional(&path) {
                Ok(Some(existing)) if existing == bytes => Ok(true),
                Ok(Some(_)) => Err(BackendError::AlreadyExists(name.clone())),
                Ok(None) => Ok(false),
                Err(e) => Err(e.into()),
            }
        };
        if compare(&mut client)? {
            return Ok(());
        }
        client.mkdir_all(path.rsplit_once('/').expect("object path contains slash").0)?;
        let mut nonce = [0; 8];
        getrandom::fill(&mut nonce).map_err(io::Error::other)?;
        let temp = format!("{path}.tmp-{}", hex::encode(nonce));
        let result = (|| -> Result<(), BackendError> {
            client.write_file(&temp, bytes)?;
            // v3 RENAME refuses an existing destination, unlike POSIX rename.
            // This is essential even across independent clients, not just this mutex.
            if let Err(error) = client.rename(&temp, &path, false)
                && !compare(&mut client)?
            {
                return Err(error.into());
            }
            Ok(())
        })();
        let cleanup = client.remove(&temp);
        result?;
        cleanup?;
        Ok(())
    }
}
impl Backend for SftpBackend {
    fn write(&self, name: &ObjectName, bytes: &[u8]) -> Result<(), BackendError> {
        let result = self
            .write_object(name, bytes)
            .map_err(|e| e.context(Some(name), Some(Path::new(&self.path(name.as_str())))));
        self.finish(result)
    }
    fn read(&self, name: &ObjectName) -> Result<Vec<u8>, BackendError> {
        self.read_range(name, 0, u64::MAX)
    }
    fn read_range(
        &self,
        name: &ObjectName,
        offset: u64,
        len: u64,
    ) -> Result<Vec<u8>, BackendError> {
        let path = self.path(name.as_str());
        let result = self
            .client()?
            .read_file(&path, offset, len)
            .map_err(|e| object_error(name, Path::new(&path), e));
        self.finish(result)
    }
    fn stat(&self, name: &ObjectName) -> Result<u64, BackendError> {
        let path = self.path(name.as_str());
        let result = self
            .client()?
            .attrs(&path)
            .and_then(|a| a.size.ok_or_else(|| codec::invalid("missing size")))
            .map_err(|e| object_error(name, Path::new(&path), e));
        self.finish(result)
    }
    fn delete(&self, name: &ObjectName) -> Result<(), BackendError> {
        let path = self.path(name.as_str());
        let result = self
            .client()?
            .remove(&path)
            .map_err(|e| object_error(name, Path::new(&path), e));
        self.finish(result)
    }
    fn delete_foreign(&self, raw: &str) -> Result<(), BackendError> {
        foreign::validate(raw)?;
        self.client()?.remove(&self.path(raw))?;
        Ok(())
    }
    fn list(&self, kind: Kind) -> Result<Vec<Entry>, BackendError> {
        let result = self.scan(kind);
        Ok(self
            .finish(result)?
            .into_iter()
            .filter_map(|e| {
                let name = ObjectName::new(&e.raw).ok()?;
                (name.kind() == kind).then_some(Entry {
                    name,
                    size: e.size,
                    modified: e.modified,
                })
            })
            .collect())
    }
    fn list_foreign(&self, kind: Kind) -> Result<Vec<ForeignEntry>, BackendError> {
        let result = self.scan(kind);
        Ok(self
            .finish(result)?
            .into_iter()
            .filter(|e| ObjectName::new(&e.raw).is_err())
            .collect())
    }
}
