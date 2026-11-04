use super::{Backend, BackendError, Entry, Kind, ObjectName};
use std::{
    collections::BTreeMap,
    io,
    sync::{Mutex, MutexGuard},
};

#[derive(Debug, Default)]
pub struct MemoryBackend {
    objects: Mutex<BTreeMap<ObjectName, Vec<u8>>>,
}

impl MemoryBackend {
    pub fn new() -> Self {
        Self::default()
    }

    fn objects(&self) -> Result<MutexGuard<'_, BTreeMap<ObjectName, Vec<u8>>>, BackendError> {
        self.objects
            .lock()
            .map_err(|_| io::Error::other("memory backend mutex poisoned").into())
    }
}

impl Backend for MemoryBackend {
    fn write(&self, name: &ObjectName, bytes: &[u8]) -> Result<(), BackendError> {
        let mut objects = self.objects()?;
        if let Some(existing) = objects.get(name) {
            if existing != bytes {
                return Err(BackendError::AlreadyExists(name.clone()));
            }
        } else {
            objects.insert(name.clone(), bytes.to_vec());
        }
        Ok(())
    }

    fn read(&self, name: &ObjectName) -> Result<Vec<u8>, BackendError> {
        self.objects()?
            .get(name)
            .cloned()
            .ok_or_else(|| BackendError::NotFound(name.clone()))
    }

    fn read_range(
        &self,
        name: &ObjectName,
        offset: u64,
        len: u64,
    ) -> Result<Vec<u8>, BackendError> {
        let objects = self.objects()?;
        let bytes = objects
            .get(name)
            .ok_or_else(|| BackendError::NotFound(name.clone()))?;
        let start = offset.min(bytes.len() as u64) as usize;
        let end = offset.saturating_add(len).min(bytes.len() as u64) as usize;
        Ok(bytes[start..end].to_vec())
    }

    fn list(&self, kind: Kind) -> Result<Vec<Entry>, BackendError> {
        Ok(self
            .objects()?
            .iter()
            .filter(|(name, _)| name.kind() == kind)
            .map(|(name, bytes)| Entry {
                name: name.clone(),
                size: bytes.len() as u64,
            })
            .collect())
    }

    fn stat(&self, name: &ObjectName) -> Result<u64, BackendError> {
        self.objects()?
            .get(name)
            .map(|bytes| bytes.len() as u64)
            .ok_or_else(|| BackendError::NotFound(name.clone()))
    }

    fn delete(&self, name: &ObjectName) -> Result<(), BackendError> {
        self.objects()?
            .remove(name)
            .map(|_| ())
            .ok_or_else(|| BackendError::NotFound(name.clone()))
    }
}
