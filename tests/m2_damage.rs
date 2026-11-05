use keelhaul::{
    backend::{Backend, Kind, ObjectName, memory::MemoryBackend},
    crypto::{
        envelope::{self, Label},
        slot,
    },
    format::{pack::BlobKind, tree::Tree},
    ops::{backup::backup, restore::restore},
    repo::{Repository, blobs::BlobStore},
    secret::Secret,
};
use serde_json::json;
fn repo() -> Repository<MemoryBackend> {
    Repository::init(MemoryBackend::new(), &Secret::KeyFile([1; 32])).unwrap()
}
fn tampered_snapshot(repo: &Repository<MemoryBackend>, tree: serde_json::Value) {
    let mut store = BlobStore::load(repo).unwrap();
    let id = store
        .add(BlobKind::Tree, &serde_json::to_vec(&tree).unwrap())
        .unwrap();
    store.finish().unwrap();
    let (_, mut s) = repo.select_snapshot("latest").unwrap();
    s.tree = id;
    s.time = "2099-01-01T00:00:00.000000000Z".into();
    repo.write_object(
        "snapshots",
        Label::Snapshot,
        &serde_json::to_vec(&s).unwrap(),
    )
    .unwrap();
}
#[test]
fn invalid_tree_entries_are_skipped_but_valid_neighbor_restores() {
    for invalid in [
        json!(".."),
        json!("/escape"),
        json!("a\u{0000}b"),
        json!("."),
    ] {
        let d = tempfile::tempdir().unwrap();
        let scratch_root = std::fs::canonicalize(d.path()).unwrap();
        let source = scratch_root.as_path().join("source");
        std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("good"), b"correct").unwrap();
        let repo = repo();
        backup(&repo, std::slice::from_ref(&source), vec![]).unwrap();
        let (_, s) = repo.select_snapshot("latest").unwrap();
        let mut store = BlobStore::load(&repo).unwrap();
        let mut root: serde_json::Value =
            serde_json::from_slice(&store.read(BlobKind::Tree, &s.tree).unwrap()).unwrap();
        let subtree = root["entries"][0]["subtree"].as_str().unwrap();
        let mut child: serde_json::Value =
            serde_json::from_slice(&store.read(BlobKind::Tree, subtree).unwrap()).unwrap();
        let mut bad = child["entries"][0].clone();
        bad["name"] = invalid;
        child["entries"].as_array_mut().unwrap().insert(0, bad);
        let new = store
            .add(BlobKind::Tree, &serde_json::to_vec(&child).unwrap())
            .unwrap();
        store.finish().unwrap();
        root["entries"][0]["subtree"] = new.into();
        tampered_snapshot(&repo, root);
        let target = scratch_root.as_path().join("out");
        let report = restore(&repo, "latest", &target, &[]).unwrap();
        assert_eq!(report.damaged.len(), 1);
        assert_eq!(
            std::fs::read(target.join(source.strip_prefix("/").unwrap()).join("good")).unwrap(),
            b"correct"
        );
        assert!(!scratch_root.as_path().join("escape").exists());
    }
}
#[test]
fn damaged_blob_has_no_final_or_temp_file_and_neighbor_survives() {
    let d = tempfile::tempdir().unwrap();
    let scratch_root = std::fs::canonicalize(d.path()).unwrap();
    std::fs::write(scratch_root.as_path().join("bad"), b"bad content").unwrap();
    std::fs::write(scratch_root.as_path().join("good"), b"good content").unwrap();
    let repo = repo();
    backup(&repo, &[scratch_root.as_path().to_path_buf()], vec![]).unwrap();
    let store = BlobStore::load(&repo).unwrap();
    let loc = store
        .index
        .values()
        .find(|loc| loc.blob.kind == BlobKind::Data && loc.blob.raw_length == 11)
        .unwrap();
    let name = keelhaul::repo::blobs::pack_name(&loc.pack).unwrap();
    let mut bytes = repo.backend.read(&name).unwrap();
    bytes[loc.blob.offset as usize + 24] ^= 1;
    repo.backend.plant_raw(name.as_str(), &bytes).unwrap();
    let target = tempfile::tempdir().unwrap();
    let report = restore(&repo, "latest", target.path(), &[]).unwrap();
    assert_eq!(report.damaged.len(), 2);
    let restored = target
        .path()
        .join(scratch_root.as_path().strip_prefix("/").unwrap());
    assert!(!restored.join("bad").exists());
    assert_eq!(
        std::fs::read(restored.join("good")).unwrap(),
        b"good content"
    );
    assert_eq!(std::fs::read_dir(restored).unwrap().count(), 1);
}
#[test]
fn keyed_hash_mismatch_is_damage_even_with_valid_aead() {
    let repo = repo();
    let mut store = BlobStore::load(&repo).unwrap();
    let id = store.add(BlobKind::Data, b"hello").unwrap();
    store.finish().unwrap();
    let location = store.index[&(BlobKind::Data, id.clone())].clone();
    let keys = slot::unwrap(&repo.list_keys().unwrap()[0].1, &Secret::KeyFile([1; 32]))
        .unwrap()
        .subkeys();
    let name = keelhaul::repo::blobs::pack_name(&location.pack).unwrap();
    let mut bytes = repo.backend.read(&name).unwrap();
    let sealed = envelope::seal(&keys.encryption, Label::Data, b"wrong").unwrap();
    assert_eq!(sealed.len() as u64, location.blob.length);
    bytes[..sealed.len()].copy_from_slice(&sealed);
    repo.backend.plant_raw(name.as_str(), &bytes).unwrap();
    assert!(matches!(
        store.read(BlobKind::Data, &id),
        Err(keelhaul::repo::Error::Damage {
            problem: "hash-mismatch",
            ..
        })
    ));
}
#[test]
fn superseded_index_is_not_live_but_pack_headers_remain_recoverable() {
    let repo = repo();
    let mut store = BlobStore::load(&repo).unwrap();
    let id = store.add(BlobKind::Data, b"hello").unwrap();
    store.finish().unwrap();
    let old = repo.backend.list(Kind::Index).unwrap()[0].name.as_str()[6..].to_string();
    repo.write_object(
        "index",
        Label::Index,
        &serde_json::to_vec(&json!({"version":1,"supersedes":[old],"packs":[]})).unwrap(),
    )
    .unwrap();
    let mut store = BlobStore::load(&repo).unwrap();
    assert_eq!(store.read(BlobKind::Data, &id).unwrap(), b"hello");
}
#[test]
fn missing_snapshot_and_bad_index_are_damage() {
    let repo = repo();
    let name = ObjectName::new(&format!("index/{}", "ab".repeat(32))).unwrap();
    repo.backend.write(&name, b"invalid").unwrap();
    let store = BlobStore::load(&repo).unwrap();
    assert_eq!(store.damage.len(), 1);
    assert_eq!(store.damage[0].object, name.as_str());
    assert_eq!(store.damage[0].problem, "hash-mismatch");
    assert!(Tree::parse(br#"{"version":1,"entries":[]}"#).is_ok());
}

#[test]
fn unreadable_tree_skips_subtree_and_missing_blob_skips_only_affected_file() {
    for missing_tree in [true, false] {
        let d = tempfile::tempdir().unwrap();
        let scratch_root = std::fs::canonicalize(d.path()).unwrap();
        let source = scratch_root.as_path().join("source");
        std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("file"), b"data").unwrap();
        let repo = repo();
        backup(&repo, std::slice::from_ref(&source), vec![]).unwrap();
        let (_, s) = repo.select_snapshot("latest").unwrap();
        let mut store = BlobStore::load(&repo).unwrap();
        let mut root: serde_json::Value =
            serde_json::from_slice(&store.read(BlobKind::Tree, &s.tree).unwrap()).unwrap();
        if missing_tree {
            root["entries"][0]["subtree"] = "ab".repeat(32).into();
        } else {
            let subtree = root["entries"][0]["subtree"].as_str().unwrap();
            let mut child: serde_json::Value =
                serde_json::from_slice(&store.read(BlobKind::Tree, subtree).unwrap()).unwrap();
            child["entries"][0]["content"] = json!(["ab".repeat(32)]);
            let id = store
                .add(BlobKind::Tree, &serde_json::to_vec(&child).unwrap())
                .unwrap();
            store.finish().unwrap();
            root["entries"][0]["subtree"] = id.into();
        }
        tampered_snapshot(&repo, root);
        let target = scratch_root.as_path().join("out");
        let report = restore(&repo, "latest", &target, &[]).unwrap();
        assert_eq!(report.damaged.len(), if missing_tree { 1 } else { 2 });
        let restored = target.join(source.strip_prefix("/").unwrap());
        assert!(!restored.join("file").exists());
        if missing_tree {
            assert!(!restored.exists());
        }
    }
}
