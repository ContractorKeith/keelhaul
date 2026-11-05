use super::{Error, Repository, open::damage};
use crate::{
    backend::{Backend, Kind, ObjectName},
    crypto::envelope::Label,
    format::snapshot::Snapshot,
};
pub struct SnapshotList {
    pub entries: Vec<(String, Snapshot)>,
    pub damaged: bool,
    pub damage: Vec<crate::damage::Damage>,
}
impl<B: Backend> Repository<B> {
    fn read_snapshot(&self, name: &ObjectName) -> Result<(String, Snapshot), Error> {
        let payload = self.read_object(name, Label::Snapshot)?;
        let snapshot = Snapshot::parse(&payload)
            .map_err(|e| damage(name.as_str(), "bad-format", &e.to_string()))?;
        Ok((name.as_str()[10..].into(), snapshot))
    }
    pub fn scan_snapshots(&self) -> Result<SnapshotList, Error> {
        let mut candidates = self.backend.list(Kind::Snapshots)?;
        // Backend mtimes prioritize reads, but only authenticated snapshot time
        // determines the result. All candidates must be read to establish newest.
        candidates.sort_by(|a, b| b.modified.cmp(&a.modified).then(b.name.cmp(&a.name)));
        let mut result = SnapshotList {
            entries: Vec::new(),
            damaged: false,
            damage: Vec::new(),
        };
        for candidate in candidates {
            match self.read_snapshot(&candidate.name) {
                Ok(snapshot) => result.entries.push(snapshot),
                Err(e @ Error::Damage { .. }) => {
                    eprintln!("warning: {e}");
                    result.damage.push(crate::damage::Damage::from_error(e)?);
                    result.damaged = true;
                }
                Err(e) => return Err(e),
            }
        }
        result
            .entries
            .sort_by(|a, b| a.1.time.cmp(&b.1.time).then(a.0.cmp(&b.0)));
        Ok(result)
    }
    pub fn snapshots(&self) -> Result<Vec<(String, Snapshot)>, Error> {
        Ok(self.scan_snapshots()?.entries)
    }
    pub fn select_snapshot(&self, selector: &str) -> Result<(String, Snapshot), Error> {
        self.select_snapshot_for_host(selector, None)
    }
    pub fn select_snapshot_for_host(
        &self,
        selector: &str,
        host: Option<&str>,
    ) -> Result<(String, Snapshot), Error> {
        if selector == "latest" {
            let snapshots = self.scan_snapshots()?.entries;
            let local_host = crate::host::hostname();
            let host = host.unwrap_or(&local_host);
            let hosts = snapshots
                .iter()
                .map(|(_, s)| s.host.as_str())
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>()
                .join(", ");
            return snapshots
                .into_iter()
                .rev()
                .find(|(_, s)| s.host == host)
                .ok_or_else(|| {
                    Error::Refused(format!(
                        "no snapshots for host {host}; hosts present: {}",
                        if hosts.is_empty() { "none" } else { &hosts }
                    ))
                });
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
        // Resolve names first: unrelated damaged snapshots cannot block an ID read.
        let candidates = self.backend.list(Kind::Snapshots)?;
        let mut matches = candidates
            .iter()
            .filter(|entry| entry.name.as_str()[10..].starts_with(selector));
        let selected = matches
            .next()
            .ok_or_else(|| Error::Refused(format!("snapshot {selector} not found")))?;
        if matches.next().is_some() {
            return Err(Error::Refused(format!(
                "snapshot prefix {selector} is ambiguous"
            )));
        }
        self.read_snapshot(&selected.name)
    }
}
