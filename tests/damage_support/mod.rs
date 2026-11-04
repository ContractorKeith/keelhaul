//! Expected-set oracle: built from the original graph, never from audit output.
#![allow(
    dead_code,
    reason = "fixture module also serves other integration targets"
)]
use keelhaul::{
    backend::{Backend, Kind, ObjectName, memory::MemoryBackend},
    format::{
        pack::BlobKind,
        tree::{Node, Tree},
    },
    ops::backup::backup,
    repo::{Repository, blobs::BlobStore},
    secret::Secret,
};
use std::collections::BTreeSet;
use std::{ffi::OsString, os::unix::ffi::OsStringExt, path::PathBuf};
pub type Objects = BTreeSet<String>;
pub struct Fixture {
    pub repo: Repository<MemoryBackend>,
    pub stored: Vec<(String, Vec<u8>)>,
    pub files: Vec<(String, Vec<String>)>,
    pub roots: Vec<String>,
    pub data: (String, u64, String),
    pub tree: (String, u64, String),
    pub header: (String, u64),
    pub trailer: String,
    pub index: String,
    pub snapshot: String,
    pub slot: String,
    pub key: Secret,
}
impl Fixture {
    pub fn new(source: &std::path::Path) -> Self {
        let key = Secret::KeyFile([7; 32]);
        let mut repo = Repository::init(MemoryBackend::new(), &key).unwrap();
        repo.config.pack_target = 1024 * 1024;
        let slot = repo.add_key(&Secret::KeyFile([8; 32])).unwrap();
        for _ in 0..3 {
            backup(&repo, &[source.to_path_buf()], vec![]).unwrap();
        }
        let mut store = BlobStore::load(&repo).unwrap();
        let mut locations = store.index.values().cloned().collect::<Vec<_>>();
        locations.sort_by(|a, b| (&a.pack, a.blob.offset).cmp(&(&b.pack, b.blob.offset)));
        // Corrupt the root tree so descendants are intentionally unreachable.
        let roots = repo
            .snapshots()
            .unwrap()
            .iter()
            .map(|(_, s)| s.tree.clone())
            .collect::<Vec<_>>();
        let tree = store.index[&(BlobKind::Tree, roots[0].clone())].clone();
        let data = locations
            .iter()
            .find(|l| l.blob.kind == BlobKind::Data && l.pack != tree.pack)
            .unwrap();
        let mut files = Vec::new();
        for (snapshot, s) in repo.snapshots().unwrap() {
            let mut work = vec![(s.tree, PathBuf::new(), true)];
            while let Some((id, base, root)) = work.pop() {
                let tree = Tree::parse(&store.read(BlobKind::Tree, &id).unwrap()).unwrap();
                for entry in tree.entries {
                    let path = if root {
                        PathBuf::from(
                            keelhaul::format::snapshot::decode_root(entry.name.as_ref().unwrap())
                                .unwrap(),
                        )
                    } else {
                        base.join(OsString::from_vec(entry.name_bytes().unwrap()))
                    };
                    match entry.node {
                        Node::Dir { subtree } => work.push((subtree, path, false)),
                        Node::File { content, .. } => files
                            .push((format!("snapshots/{snapshot}:{}", path.display()), content)),
                        _ => {}
                    }
                }
            }
        }
        let data = (
            keelhaul::repo::blobs::pack_name(&data.pack)
                .unwrap()
                .to_string(),
            data.blob.offset,
            data.blob.id.clone(),
        );
        let tree = (
            keelhaul::repo::blobs::pack_name(&tree.pack)
                .unwrap()
                .to_string(),
            tree.blob.offset,
            tree.blob.id,
        );
        let pure = repo
            .backend
            .list(Kind::Data)
            .unwrap()
            .into_iter()
            .find(|e| e.name.as_str() != tree.0)
            .unwrap();
        let bytes = repo.backend.read(&pure.name).unwrap();
        let len = u32::from_le_bytes(bytes[bytes.len() - 4..].try_into().unwrap()) as usize;
        let header = (pure.name.to_string(), (bytes.len() - 4 - len) as u64);
        let trailer = tree.0.clone();
        let index = repo.backend.list(Kind::Index).unwrap()[0].name.to_string();
        let snapshot = repo.backend.list(Kind::Snapshots).unwrap()[0]
            .name
            .to_string();
        let stored = Kind::ALL
            .into_iter()
            .flat_map(|kind| repo.backend.list(kind).unwrap())
            .map(|e| (e.name.to_string(), repo.backend.read(&e.name).unwrap()))
            .collect();
        Self {
            repo,
            stored,
            files,
            roots,
            data,
            tree,
            header,
            trailer,
            index,
            snapshot,
            slot: format!("keys/{slot}"),
            key,
        }
    }
    pub fn reset(&self) {
        for kind in Kind::ALL {
            for e in self.repo.backend.list(kind).unwrap() {
                self.repo.backend.delete(&e.name).unwrap();
            }
        }
        for (n, b) in &self.stored {
            self.repo.backend.plant_raw(n, b).unwrap();
        }
    }
    pub fn bytes(&self, name: &str) -> Vec<u8> {
        self.repo
            .backend
            .read(&ObjectName::new(name).unwrap())
            .unwrap()
    }
    pub fn plant(&self, name: &str, bytes: &[u8]) {
        self.repo.backend.plant_raw(name, bytes).unwrap();
    }
    pub fn files_using(&self, id: &str) -> Objects {
        self.files
            .iter()
            .filter(|(_, chunks)| chunks.iter().any(|c| c == id))
            .map(|(p, _)| p.clone())
            .collect()
    }
}
pub fn objects(report: &keelhaul::damage::Report) -> Objects {
    report.damage.iter().map(|d| d.object.clone()).collect()
}
