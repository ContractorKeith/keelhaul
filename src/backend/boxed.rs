use super::{Backend, BackendError, Entry, ForeignEntry, Kind, ObjectName};
use std::path::Path;

// Keep Repository generic while runtime backend selection uses one trait object.
impl<B: Backend + ?Sized> Backend for Box<B> {
    fn write(&self, name: &ObjectName, bytes: &[u8]) -> Result<(), BackendError> {
        (**self).write(name, bytes)
    }
    fn read(&self, name: &ObjectName) -> Result<Vec<u8>, BackendError> {
        (**self).read(name)
    }
    fn read_range(
        &self,
        name: &ObjectName,
        offset: u64,
        len: u64,
    ) -> Result<Vec<u8>, BackendError> {
        (**self).read_range(name, offset, len)
    }
    fn list(&self, kind: Kind) -> Result<Vec<Entry>, BackendError> {
        (**self).list(kind)
    }
    fn list_foreign(&self, kind: Kind) -> Result<Vec<ForeignEntry>, BackendError> {
        (**self).list_foreign(kind)
    }
    fn delete_foreign(&self, raw: &str) -> Result<(), BackendError> {
        (**self).delete_foreign(raw)
    }
    fn stat(&self, name: &ObjectName) -> Result<u64, BackendError> {
        (**self).stat(name)
    }
    fn delete(&self, name: &ObjectName) -> Result<(), BackendError> {
        (**self).delete(name)
    }
    fn contains_destination(&self, path: &Path) -> Result<bool, BackendError> {
        (**self).contains_destination(path)
    }
}
