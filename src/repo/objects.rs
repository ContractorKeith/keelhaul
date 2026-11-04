//! Authenticated, content-addressed metadata objects.
use super::{Error, Repository, open::damage};
use crate::{
    backend::{Backend, BackendError, ObjectName},
    crypto::envelope::{self, Label},
};

impl<B: Backend> Repository<B> {
    pub fn read_object(&self, name: &ObjectName, label: Label) -> Result<Vec<u8>, Error> {
        super::audit::object(&self.backend, &self.master.subkeys(), name, label)
    }

    pub fn write_object(
        &self,
        directory: &str,
        label: Label,
        payload: &[u8],
    ) -> Result<String, Error> {
        let stored = envelope::seal(&self.master.subkeys().encryption, label, payload)?;
        let id = blake3::hash(&stored).to_hex().to_string();
        self.backend
            .write(&ObjectName::new(&format!("{directory}/{id}"))?, &stored)?;
        Ok(id)
    }
    pub fn chunker_key(&self) -> [u8; 32] {
        self.master.subkeys().gear
    }
}
pub(super) fn missing(name: &ObjectName, error: BackendError) -> Error {
    match error {
        BackendError::NotFound(_) => damage(name.as_str(), "missing", "object is missing"),
        other => other.into(),
    }
}
