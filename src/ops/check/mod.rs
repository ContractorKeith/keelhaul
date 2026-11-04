//! Metadata-only structural audit. Damage never stops unrelated objects being examined.
mod inventory;
mod lock;
mod trees;
use crate::format::tree::Tree;
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
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    rc::Rc,
};
pub(super) type TreeLocation = (String, String, u64, u64, u64);

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
    pub(super) tree_cache: HashMap<TreeLocation, Option<Rc<Tree>>>,
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
            tree_cache: HashMap::new(),
        }
    }
    pub fn metadata(&mut self) -> Result<(), Error> {
        inventory::scan(self)?;
        inventory::index(self)?;
        Ok(())
    }
    fn audit_tree_blobs(&mut self) -> Result<(), Error> {
        let locations = self
            .packs
            .iter()
            .flat_map(|(pack, h)| {
                h.iter().flat_map(move |h| {
                    h.blobs
                        .iter()
                        .filter(|b| b.kind == BlobKind::Tree)
                        .map(move |b| (pack.clone(), b.clone()))
                })
            })
            .collect::<Vec<_>>();
        for (pack, blob) in locations {
            self.read_tree(&pack, &blob)?;
        }
        Ok(())
    }
    fn read_tree(&mut self, pack: &str, blob: &Blob) -> Result<Option<Rc<Tree>>, Error> {
        let key = tree_key(pack, blob);
        if let Some(tree) = self.tree_cache.get(&key) {
            return Ok(tree.clone());
        }
        let name = crate::repo::blobs::pack_name(pack)?;
        let result = audit::range(self.backend, &name, blob.offset, blob.length)
            .and_then(|stored| audit::blob(&self.keys, &blob.id, name.as_str(), blob, &stored))
            .and_then(|bytes| {
                Tree::parse(&bytes)
                    .map_err(|e| inventory::bad(&blob.id, &format!("{e}; pack {name}")))
            });
        let tree = self.report.collect(result)?.map(Rc::new);
        self.tree_cache.insert(key, tree.clone());
        Ok(tree)
    }
    pub(super) fn reachable(&mut self) -> Result<(), Error> {
        trees::walk(self, false)
    }
    pub fn finish(mut self, verify: bool) -> Result<Report, Error> {
        self.audit_tree_blobs()?;
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
    pub fn tree(&mut self, id: &str) -> Result<Option<Rc<Tree>>, Error> {
        let locations = self
            .live
            .get(&(BlobKind::Tree, id.into()))
            .cloned()
            .unwrap_or_default();
        let mut available = false;
        for (pack, blob) in locations {
            let blob = match self.packs.get(&pack) {
                None => continue,
                Some(Some(header)) => match header
                    .blobs
                    .iter()
                    .find(|b| b.kind == BlobKind::Tree && b.id == id)
                {
                    Some(actual) => actual.clone(),
                    None => continue,
                },
                Some(None) => blob,
            };
            available = true;
            if let Some(tree) = self.read_tree(&pack, &blob)? {
                return Ok(Some(tree));
            }
        }
        if !available {
            self.report.add(
                id,
                "blob",
                "dangling-reference",
                "tree has no available live index location",
            );
        }
        Ok(None)
    }
}
pub fn check<B: Backend>(repo: &Repository<B>) -> Result<Report, Error> {
    crate::repo::lock::with_audit(&repo.backend, repo.subkeys(), |damage| {
        run_check(State::new(&repo.backend, repo.subkeys()), damage)
    })
}
/// Authenticate without requiring a valid config: audit it with the other objects.
pub fn check_backend<B: Backend>(backend: &B, secret: &Secret) -> Result<Report, Error> {
    let master = Repository::unlock(backend, secret)?;
    crate::repo::lock::with_audit(backend, master.subkeys(), |damage| {
        run_check(State::new(backend, master.subkeys()), damage)
    })
}
pub(super) fn name_id(name: &ObjectName) -> String {
    name.as_str()
        .rsplit('/')
        .next()
        .expect("validated object name")
        .into()
}

pub(super) fn tree_key(pack: &str, blob: &Blob) -> TreeLocation {
    (
        pack.into(),
        blob.id.clone(),
        blob.offset,
        blob.length,
        blob.raw_length,
    )
}

fn run_check<B: Backend>(
    mut state: State<'_, B>,
    damage: Vec<crate::damage::Damage>,
) -> Result<Report, Error> {
    state.report.damage.extend(damage);
    state.metadata()?;
    state.finish(false)
}
