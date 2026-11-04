use super::{Backend, BackendError, Entry, ForeignEntry, Kind, ObjectName, foreign};
use std::{
    collections::BTreeMap,
    io,
    sync::{Mutex, MutexGuard},
    time::SystemTime,
};

#[derive(Debug)]
struct Stored {
    bytes: Vec<u8>,
    modified: SystemTime,
}

#[derive(Debug, Default)]
pub struct MemoryBackend {
    objects: Mutex<BTreeMap<String, Stored>>,
}

impl MemoryBackend {
    pub fn new() -> Self {
        Self::default()
    }

    fn objects(&self) -> Result<MutexGuard<'_, BTreeMap<String, Stored>>, BackendError> {
        self.objects
            .lock()
            .map_err(|_| io::Error::other("memory backend mutex poisoned").into())
    }

    /// Test fixtures can plant opaque storage entries without object validation.
    pub fn plant_raw(&self, raw: &str, bytes: &[u8]) -> Result<(), BackendError> {
        self.objects()?.insert(
            raw.to_owned(),
            Stored {
                bytes: bytes.to_vec(),
                modified: SystemTime::now(),
            },
        );
        Ok(())
    }
}

impl Backend for MemoryBackend {
    fn write(&self, name: &ObjectName, bytes: &[u8]) -> Result<(), BackendError> {
        let mut objects = self.objects()?;
        if let Some(existing) = objects.get(name.as_str()) {
            if existing.bytes != bytes {
                return Err(BackendError::AlreadyExists(name.clone()));
            }
        } else {
            objects.insert(
                name.as_str().to_owned(),
                Stored {
                    bytes: bytes.to_vec(),
                    modified: SystemTime::now(),
                },
            );
        }
        Ok(())
    }

    fn read(&self, name: &ObjectName) -> Result<Vec<u8>, BackendError> {
        self.objects()?
            .get(name.as_str())
            .map(|stored| stored.bytes.clone())
            .ok_or_else(|| BackendError::NotFound(name.clone()))
    }

    fn read_range(
        &self,
        name: &ObjectName,
        offset: u64,
        len: u64,
    ) -> Result<Vec<u8>, BackendError> {
        let objects = self.objects()?;
        let bytes = &objects
            .get(name.as_str())
            .ok_or_else(|| BackendError::NotFound(name.clone()))?
            .bytes;
        let start = offset.min(bytes.len() as u64) as usize;
        let end = offset.saturating_add(len).min(bytes.len() as u64) as usize;
        Ok(bytes[start..end].to_vec())
    }

    fn list(&self, kind: Kind) -> Result<Vec<Entry>, BackendError> {
        Ok(self
            .objects()?
            .iter()
            .filter_map(|(raw, stored)| {
                let name = ObjectName::new(raw).ok()?;
                (name.kind() == kind).then_some(Entry {
                    name,
                    size: stored.bytes.len() as u64,
                    modified: Some(stored.modified),
                })
            })
            .collect())
    }

    fn list_foreign(&self, kind: Kind) -> Result<Vec<ForeignEntry>, BackendError> {
        Ok(self
            .objects()?
            .iter()
            .filter(|(raw, _)| foreign::kind(raw) == Some(kind))
            .map(|(raw, stored)| ForeignEntry {
                raw: raw.clone(),
                deletable: foreign::validate(raw).is_ok(),
                size: stored.bytes.len() as u64,
                modified: Some(stored.modified),
            })
            .collect())
    }

    fn delete_foreign(&self, raw: &str) -> Result<(), BackendError> {
        foreign::validate(raw)?;
        self.objects()?.remove(raw);
        Ok(())
    }

    fn stat(&self, name: &ObjectName) -> Result<u64, BackendError> {
        self.objects()?
            .get(name.as_str())
            .map(|stored| stored.bytes.len() as u64)
            .ok_or_else(|| BackendError::NotFound(name.clone()))
    }

    fn delete(&self, name: &ObjectName) -> Result<(), BackendError> {
        self.objects()?.remove(name.as_str());
        Ok(())
    }
}
