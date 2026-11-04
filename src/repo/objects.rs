//! Authenticated, content-addressed metadata objects.
use super::{Error, Repository, open::damage};
use crate::{
    backend::{Backend, BackendError, Kind, ObjectName},
    crypto::envelope::{self, Label},
    format::snapshot::Snapshot,
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
    pub fn snapshots(&self) -> Result<Vec<(String, Snapshot)>, Error> {
        let mut values = Vec::new();
        for entry in self.backend.list(Kind::Snapshots)? {
            let payload = self.read_object(&entry.name, Label::Snapshot)?;
            let snapshot = Snapshot::parse(&payload)
                .map_err(|e| damage(entry.name.as_str(), "bad-format", &e.to_string()))?;
            values.push((entry.name.as_str()[10..].to_owned(), snapshot));
        }
        values.sort_by(|a, b| a.1.time.cmp(&b.1.time).then(a.0.cmp(&b.0)));
        Ok(values)
    }
    /// `latest` is the newest timestamp; full IDs break ties deterministically.
    pub fn select_snapshot(&self, selector: &str) -> Result<(String, Snapshot), Error> {
        let snapshots = self.snapshots()?;
        if selector == "latest" {
            return snapshots
                .into_iter()
                .last()
                .ok_or_else(|| Error::Refused("no snapshots".into()));
        }
        if selector.len() < 8
            || selector.len() > 64
            || !selector
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(Error::Refused(
                "snapshot prefix must be 8–64 lowercase hex characters".into(),
            ));
        }
        let mut matches = snapshots
            .into_iter()
            .filter(|(id, _)| id.starts_with(selector));
        let value = matches
            .next()
            .ok_or_else(|| Error::Refused(format!("snapshot {selector} not found")))?;
        if matches.next().is_some() {
            return Err(Error::Refused(format!(
                "snapshot prefix {selector} is ambiguous"
            )));
        }
        Ok(value)
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
