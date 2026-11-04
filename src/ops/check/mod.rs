//! Metadata-only structural audit. Damage never stops unrelated objects being examined.
mod inventory;
mod lock;
mod trees;
use crate::{
    backend::{Backend, ObjectName},
    crypto::keys::Subkeys,
    damage::Report,
    format::{
        index::Index,
        pack::{Blob, BlobKind, Header},
        snapshot::Snapshot,
    },
    repo::{Error, Repository, audit},
    secret::Secret,
};
use std::collections::{BTreeMap, HashMap, HashSet};

pub(super) type BlobKey = (BlobKind, String);
pub(super) struct State<'a, B> {
    pub backend: &'a B,
    pub keys: Subkeys,
    pub report: Report,
    pub packs: BTreeMap<String, Option<Header>>,
    pub indexes: BTreeMap<String, Index>,
    pub snapshots: BTreeMap<String, Snapshot>,
    pub live: HashMap<BlobKey, Vec<(String, Blob)>>,
    pub used: HashSet<BlobKey>,
    pub bad: HashSet<BlobKey>,
}
impl<'a, B: Backend> State<'a, B> {
    pub fn new(backend: &'a B, keys: Subkeys) -> Self {
        Self {
            backend,
            keys,
            report: Report::default(),
            packs: BTreeMap::new(),
            indexes: BTreeMap::new(),
            snapshots: BTreeMap::new(),
            live: HashMap::new(),
            used: HashSet::new(),
            bad: HashSet::new(),
        }
    }
    pub fn metadata(&mut self) -> Result<(), Error> {
        // M5: shared lock
        inventory::scan(self)?;
        inventory::index(self)?;
        Ok(())
    }
    pub fn finish(mut self, verify: bool) -> Result<Report, Error> {
        trees::walk(&mut self, verify)?;
        let live_packs = self
            .live
            .values()
            .flatten()
            .map(|(id, _)| id)
            .collect::<HashSet<_>>();
        for (name, index) in &self.indexes {
            if !index.packs.iter().any(|p| {
                p.blobs
                    .iter()
                    .any(|b| self.used.contains(&(b.kind, b.id.clone())))
                    && live_packs.contains(&p.id)
            }) {
                self.report.note(name, "index", "unused-index");
            }
        }
        self.report.sort();
        Ok(self.report)
    }
    pub fn tree(&self, id: &str) -> Result<Vec<u8>, Error> {
        let Some(locations) = self.live.get(&(BlobKind::Tree, id.into())) else {
            return Err(Error::Damage {
                object: id.into(),
                problem: "dangling-reference",
                detail: "tree is absent from live index".into(),
            });
        };
        let mut last = None;
        for (pack, blob) in locations {
            if !self.packs.contains_key(pack) {
                continue;
            }
            let name = crate::repo::blobs::pack_name(pack)?;
            let result = audit::range(self.backend, &name, blob.offset, blob.length)
                .and_then(|stored| audit::blob(&self.keys, id, blob, &stored));
            match result {
                Ok(bytes) => return Ok(bytes),
                Err(e @ Error::Damage { .. }) => last = Some(e),
                Err(e) => return Err(e),
            }
        }
        Err(last.unwrap_or_else(|| Error::Damage {
            object: id.into(),
            problem: "dangling-reference",
            detail: "tree has no available pack".into(),
        }))
    }
}
pub fn check<B: Backend>(repo: &Repository<B>) -> Result<Report, Error> {
    let mut state = State::new(&repo.backend, repo.subkeys());
    state.metadata()?;
    state.finish(false)
}
/// Authenticate without requiring a valid config: audit it with the other objects.
pub fn check_backend<B: Backend>(backend: &B, secret: &Secret) -> Result<Report, Error> {
    let mut state = State::new(backend, Repository::unlock(backend, secret)?.subkeys());
    state.metadata()?;
    state.finish(false)
}
pub(super) fn name_id(name: &ObjectName) -> String {
    name.as_str()
        .rsplit('/')
        .next()
        .expect("validated object name")
        .into()
}
