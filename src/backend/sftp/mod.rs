//! SFTP v3 storage through the user's system ssh.
pub mod client;
mod codec;
mod operations;
mod response;
mod scan;
mod ssh;
mod status;
mod stderr;
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
        let url = parse_url(url)?;
        let client = Client::connect(&url)?;
        Self::with_client(url.path, client)
    }
    pub fn open(url: &str) -> Result<Self, BackendError> {
        let url = parse_url(url)?;
        let mut client = Client::connect(&url)?;
        let result = client.attrs(&url.path);
        let attrs =
            result.map_err(|e| BackendError::from(e).context(None, Some(Path::new(&url.path))))?;
        if !attrs
            .permissions
            .is_some_and(|mode| mode & 0o170000 == 0o040000)
        {
            return Err(
                BackendError::from(io::Error::other("repository path is not a directory"))
                    .context(None, Some(Path::new(&url.path))),
            );
        }
        Ok(Self {
            root: url.path.trim_end_matches('/').into(),
            client: Mutex::new(client),
        })
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
            match client.matches_file(&path, bytes) {
                Ok(Some(true)) => Ok(true),
                Ok(Some(false)) => Err(BackendError::AlreadyExists(name.clone())),
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
        self.write_object(name, bytes)
            .map_err(|e| e.context(Some(name), Some(Path::new(&self.path(name.as_str())))))
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
        self.client()?
            .read_file(&path, offset, len)
            .map_err(|e| object_error(name, Path::new(&path), e))
    }
    fn stat(&self, name: &ObjectName) -> Result<u64, BackendError> {
        let path = self.path(name.as_str());
        let mut client = self.client()?;
        client
            .object_size(&path)
            .map_err(|e| object_error(name, Path::new(&path), e))
    }
    fn delete(&self, name: &ObjectName) -> Result<(), BackendError> {
        let path = self.path(name.as_str());
        self.client()?
            .remove(&path)
            .map_err(|e| object_error(name, Path::new(&path), e))
    }
    fn delete_foreign(&self, raw: &str) -> Result<(), BackendError> {
        foreign::validate(raw)?;
        self.client()?.remove(&self.path(raw))?;
        Ok(())
    }
    fn list(&self, kind: Kind) -> Result<Vec<Entry>, BackendError> {
        Ok(self
            .scan(kind)?
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
        Ok(self
            .scan(kind)?
            .into_iter()
            .filter(|entry| entry.object.is_none())
            .map(|entry| entry.foreign)
            .collect())
    }
}

fn parse_url(url: &str) -> Result<SftpUrl, BackendError> {
    SftpUrl::parse(url).map_err(|_| BackendError::InvalidConfig("invalid SFTP repository URL"))
}
