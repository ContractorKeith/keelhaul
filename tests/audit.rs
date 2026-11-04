use keelhaul::{
    backend::{
        Backend, BackendError, Entry, ForeignEntry, Kind, ObjectName, memory::MemoryBackend,
    },
    crypto::{
        envelope::{self, Label},
        slot,
    },
    format::pack::BlobKind,
    ops::{
        check::check,
        verify::{Subset, verify},
    },
    repo::{
        Repository,
        blobs::{BlobStore, pack_name},
    },
    secret::Secret,
};
use std::{path::Path, sync::Mutex};
struct Reads {
    inner: MemoryBackend,
    ranges: Mutex<Vec<(String, u64, u64)>>,
    whole: Mutex<Vec<String>>,
}
impl Reads {
    fn new() -> Self {
        Self {
            inner: MemoryBackend::new(),
            ranges: Mutex::new(vec![]),
            whole: Mutex::new(vec![]),
        }
    }
}
impl Backend for Reads {
    fn contains_destination(&self, p: &Path) -> Result<bool, BackendError> {
        self.inner.contains_destination(p)
    }
    fn write(&self, n: &ObjectName, b: &[u8]) -> Result<(), BackendError> {
        self.inner.write(n, b)
    }
    fn read(&self, n: &ObjectName) -> Result<Vec<u8>, BackendError> {
        self.whole.lock().unwrap().push(n.to_string());
        self.inner.read(n)
    }
    fn read_range(&self, n: &ObjectName, o: u64, l: u64) -> Result<Vec<u8>, BackendError> {
        self.ranges.lock().unwrap().push((n.to_string(), o, l));
        self.inner.read_range(n, o, l)
    }
    fn list(&self, k: Kind) -> Result<Vec<Entry>, BackendError> {
        self.inner.list(k)
    }
    fn list_foreign(&self, k: Kind) -> Result<Vec<ForeignEntry>, BackendError> {
        self.inner.list_foreign(k)
    }
    fn delete_foreign(&self, n: &str) -> Result<(), BackendError> {
        self.inner.delete_foreign(n)
    }
    fn stat(&self, n: &ObjectName) -> Result<u64, BackendError> {
        self.inner.stat(n)
    }
    fn delete(&self, n: &ObjectName) -> Result<(), BackendError> {
        self.inner.delete(n)
    }
}
#[test]
fn check_never_reads_data_and_subsets_partition_full_pack_reads() {
    let repo = Repository::init(Reads::new(), &Secret::KeyFile([7; 32])).unwrap();
    let mut store = BlobStore::load(&repo).unwrap();
    for n in 0..12 {
        store.add(BlobKind::Data, &[n; 100]).unwrap();
        store.finish().unwrap();
    }
    let packs = repo.backend.list(Kind::Data).unwrap();
    for e in &packs {
        let mut b = repo.backend.inner.read(&e.name).unwrap();
        b[24] ^= 1;
        repo.backend.inner.plant_raw(e.name.as_str(), &b).unwrap();
    }
    assert!(check(&repo).unwrap().damage.is_empty());
    assert!(
        !repo
            .backend
            .whole
            .lock()
            .unwrap()
            .iter()
            .any(|n| n.starts_with("data/"))
    );
    for (name, offset, _) in &*repo.backend.ranges.lock().unwrap() {
        let b = repo
            .backend
            .inner
            .read(&ObjectName::new(name).unwrap())
            .unwrap();
        let h = u32::from_le_bytes(b[b.len() - 4..].try_into().unwrap()) as u64;
        assert!(
            *offset >= b.len() as u64 - 4 - h,
            "data body read at {offset}"
        );
    }
    let mut seen = std::collections::HashSet::new();
    for n in 1..=3 {
        repo.backend.whole.lock().unwrap().clear();
        let subset = format!("{n}/3").parse::<Subset>().unwrap();
        let result = verify(&repo, Some(subset)).unwrap();
        let expected = packs
            .iter()
            .filter(|p| subset.contains(p.name.as_str().rsplit('/').next().unwrap()))
            .map(|e| e.name.to_string())
            .collect::<std::collections::BTreeSet<_>>();
        let actual = repo
            .backend
            .whole
            .lock()
            .unwrap()
            .iter()
            .filter(|n| n.starts_with("data/"))
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(actual, expected);
        for name in actual {
            assert!(seen.insert(name.clone()));
            assert!(
                result
                    .damage
                    .iter()
                    .any(|d| d.object == name && d.problem == "hash-mismatch")
            );
        }
    }
    assert_eq!(seen.len(), packs.len());
}
#[test]
fn valid_ciphertext_with_wrong_keyed_id_and_raw_length_is_rejected() {
    for wrong_length in [false, true] {
        let secret = Secret::KeyFile([7; 32]);
        let repo = Repository::init(MemoryBackend::new(), &secret).unwrap();
        let mut store = BlobStore::load(&repo).unwrap();
        let id = store.add(BlobKind::Data, b"hello").unwrap();
        store.finish().unwrap();
        let loc = &store.index[&(BlobKind::Data, id.clone())];
        let name = pack_name(&loc.pack).unwrap();
        let master = slot::unwrap(&repo.list_keys().unwrap()[0].1, &secret).unwrap();
        let replacement = if wrong_length {
            b"four".as_slice()
        } else {
            b"wrong".as_slice()
        };
        let stored =
            envelope::seal(&master.subkeys().encryption, Label::Data, replacement).unwrap();
        let mut bytes = repo.backend.read(&name).unwrap();
        if wrong_length {
            let mut descriptor = loc.blob.clone();
            descriptor.raw_length = 6;
            let header = envelope::seal(
                &master.subkeys().encryption,
                Label::PackHeader,
                &serde_json::to_vec(&keelhaul::format::pack::Header {
                    version: 1,
                    blobs: vec![descriptor.clone()],
                })
                .unwrap(),
            )
            .unwrap();
            let mut pack = bytes[..loc.blob.length as usize].to_vec();
            pack.extend(&header);
            pack.extend((header.len() as u32).to_le_bytes());
            let pack_id = blake3::hash(&pack).to_hex().to_string();
            repo.backend
                .write(&pack_name(&pack_id).unwrap(), &pack)
                .unwrap();
            for e in repo.backend.list(Kind::Index).unwrap() {
                repo.backend.delete(&e.name).unwrap();
            }
            repo.backend.delete(&name).unwrap();
            let index = keelhaul::format::index::Index {
                version: 1,
                supersedes: vec![],
                packs: vec![keelhaul::format::index::Pack {
                    id: pack_id,
                    blobs: vec![descriptor],
                }],
            };
            repo.write_object("index", Label::Index, &serde_json::to_vec(&index).unwrap())
                .unwrap();
            assert!(check(&repo).unwrap().damage.is_empty());
            assert!(
                verify(&repo, None)
                    .unwrap()
                    .damage
                    .iter()
                    .any(|d| d.object == id && d.problem == "size-mismatch")
            );
        } else {
            bytes[..stored.len()].copy_from_slice(&stored);
            repo.backend.plant_raw(name.as_str(), &bytes).unwrap();
            let report = verify(&repo, None).unwrap();
            assert!(
                report
                    .damage
                    .iter()
                    .any(|d| d.object == id && d.problem == "hash-mismatch")
            );
        }
    }
}
