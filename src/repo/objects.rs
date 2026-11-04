//! Authenticated, content-addressed metadata objects.
use super::{Error, Repository, open::damage};
use crate::{
    backend::{Backend, BackendError, ObjectName},
    crypto::envelope::{self, Label},
};

impl<B: Backend> Repository<B> {
    pub fn read_object(&self, name: &ObjectName, label: Label) -> Result<Vec<u8>, Error> {
        let stored = self.backend.read(name).map_err(|e| missing(name, e))?;
        let id = name
            .as_str()
            .rsplit('/')
            .next()
            .expect("object names have a component");
        if blake3::hash(&stored).to_hex().as_str() != id {
            return Err(damage(
                name.as_str(),
                "hash-mismatch",
                "stored object ID differs",
            ));
        }
        envelope::open(&self.master.subkeys().encryption, label, &stored)
            .map_err(|e| damage(name.as_str(), "undecryptable", &e.to_string()))
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
