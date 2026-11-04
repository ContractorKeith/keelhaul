#![allow(
    dead_code,
    reason = "fixture variants are shared across prune integration targets"
)]
use keelhaul::{
    backend::Backend,
    crypto::envelope::Label,
    format::{
        pack::BlobKind,
        snapshot::{Snapshot, Summary},
        tree::Tree,
    },
    repo::{Repository, blobs::BlobStore},
    secret::Secret,
};
const TIME: &str = "2024-01-01T00:00:00.000000000Z";
pub fn fixture<B: Backend>(backend: B) -> (Repository<B>, String, Vec<Vec<u8>>) {
    fixture_with_packs(backend, false)
}
pub fn fixture_with_packs<B: Backend>(
    backend: B,
    separate: bool,
) -> (Repository<B>, String, Vec<Vec<u8>>) {
    let repo = Repository::init(backend, &Secret::KeyFile([13; 32])).unwrap();
    let mut store = BlobStore::load(&repo).unwrap();
    let mut ids = Vec::new();
    let mut contents = Vec::new();
    for n in 0..10u64 {
        let mut bytes = vec![0; 1000];
        blake3::Hasher::new()
            .update(&n.to_le_bytes())
            .finalize_xof()
            .fill(&mut bytes);
        ids.push(store.add(BlobKind::Data, &bytes).unwrap());
        contents.push(bytes);
        if separate {
            store.finish().unwrap();
        }
    }
    store.finish().unwrap();
    let tree = serde_json::json!({"version":1,"entries":[{"name":"%2Ffile","mode":420,"mtime":TIME,"ctime":TIME,"uid":0,"gid":0,"inode":1,"device":1,"links":1,"type":"file","size":7000,"content":ids[..7]}]});
    let raw = serde_json::to_vec(&tree).unwrap();
    Tree::parse(&raw).unwrap();
    let tree = store.add(BlobKind::Tree, &raw).unwrap();
    store.finish().unwrap();
    let snapshot = Snapshot {
        version: 1,
        time: TIME.into(),
        host: keelhaul::host::hostname(),
        username: "test".into(),
        paths: vec!["/file".into()],
        tags: vec![],
        excludes: vec![],
        tree,
        parent: None,
        program: "test".into(),
        summary: Summary::default(),
    };
    let id = repo
        .write_object(
            "snapshots",
            Label::Snapshot,
            &serde_json::to_vec(&snapshot).unwrap(),
        )
        .unwrap();
    (repo, id, contents)
}
