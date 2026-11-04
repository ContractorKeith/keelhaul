use keelhaul::{
    backend::{Backend, Kind, memory::MemoryBackend},
    crypto::{
        envelope::{self, Label},
        slot,
    },
    format::{
        index::{Index, Pack},
        pack::{BlobKind, Header},
        snapshot::Snapshot,
    },
    ops::{check::check, verify::verify},
    repo::{
        Repository,
        blobs::{BlobStore, pack_name},
    },
    secret::Secret,
};
fn repo() -> Repository<MemoryBackend> {
    Repository::init(MemoryBackend::new(), &Secret::KeyFile([7; 32])).unwrap()
}
fn snapshot(repo: &Repository<MemoryBackend>, tree: &str) -> String {
    let value = serde_json::json!({"version":1,"time":"2026-11-04T14:03:22.123456789Z","host":"test","username":"test","paths":["/source"],"tags":[],"excludes":[],"tree":tree,"parent":null,"program":"test","summary":{"files_new":0,"files_changed":0,"files_unchanged":0,"dirs":0,"files_skipped":0,"data_blobs_added":0,"bytes_added":0,"bytes_processed":0}});
    Snapshot::parse(&serde_json::to_vec(&value).unwrap()).unwrap();
    repo.write_object(
        "snapshots",
        Label::Snapshot,
        &serde_json::to_vec(&value).unwrap(),
    )
    .unwrap()
}
fn entry(node: serde_json::Value) -> serde_json::Value {
    let mut e = serde_json::json!({"name":"%2Fsource","mode":420,"mtime":"2026-11-04T14:03:22.123456789Z","ctime":"2026-11-04T14:03:22.123456789Z","uid":0,"gid":0,"inode":1,"device":1,"links":1});
    e.as_object_mut()
        .unwrap()
        .extend(node.as_object().unwrap().clone());
    e
}
#[test]
fn wrong_blob_type_file_sizes_and_missing_root_entries_are_metadata_damage() {
    for variant in 0..3 {
        let repo = repo();
        let mut store = BlobStore::load(&repo).unwrap();
        let id = store
            .add(BlobKind::Tree, br#"{"version":1,"entries":[]}"#)
            .unwrap();
        let content = if variant == 0 {
            id
        } else {
            store.add(BlobKind::Data, b"hello").unwrap()
        };
        let entries = if variant == 2 {
            vec![]
        } else {
            vec![entry(
                serde_json::json!({"type":"file","size":6,"content":[content.clone()]}),
            )]
        };
        let tree = store
            .add(
                BlobKind::Tree,
                &serde_json::to_vec(&serde_json::json!({"version":1,"entries":entries})).unwrap(),
            )
            .unwrap();
        store.finish().unwrap();
        snapshot(&repo, &tree);
        let report = check(&repo).unwrap();
        let expected = match variant {
            0 => "dangling-reference",
            1 => "size-mismatch",
            _ => "bad-format",
        };
        assert!(
            report.damage.iter().any(|d| d.problem == expected),
            "{report:?}"
        );
    }
}
#[test]
fn superseded_indexes_do_not_make_old_packs_missing_and_orphans_are_notes() {
    let repo = repo();
    let mut store = BlobStore::load(&repo).unwrap();
    store.add(BlobKind::Data, b"unused").unwrap();
    store.finish().unwrap();
    let old = repo.backend.list(Kind::Index).unwrap()[0].name.clone();
    repo.write_object(
        "index",
        Label::Index,
        &serde_json::to_vec(&Index {
            version: 1,
            supersedes: vec![old.as_str()[6..].into()],
            packs: vec![],
        })
        .unwrap(),
    )
    .unwrap();
    let report = check(&repo).unwrap();
    assert!(report.damage.is_empty());
    assert!(report.notes.iter().any(|n| n.problem == "orphan"));
    assert!(
        report
            .notes
            .iter()
            .any(|n| n.object == old.as_str() && n.problem == "unused-index")
    );
    repo.backend
        .delete(&repo.backend.list(Kind::Data).unwrap()[0].name)
        .unwrap();
    assert!(check(&repo).unwrap().damage.is_empty());
}
#[test]
fn healthy_duplicate_prevents_dangling_blob_and_file_reports() {
    let repo = repo();
    let mut store = BlobStore::load(&repo).unwrap();
    let id = store.add(BlobKind::Data, b"hello").unwrap();
    store.finish().unwrap();
    let loc = store.index[&(BlobKind::Data, id)].clone();
    let master = slot::unwrap(&repo.list_keys().unwrap()[0].1, &Secret::KeyFile([7; 32])).unwrap();
    let blob = envelope::seal(&master.subkeys().encryption, Label::Data, b"hello").unwrap();
    let header = envelope::seal(
        &master.subkeys().encryption,
        Label::PackHeader,
        &serde_json::to_vec(&Header {
            version: 1,
            blobs: vec![loc.blob.clone()],
        })
        .unwrap(),
    )
    .unwrap();
    let mut bytes = blob;
    bytes.extend(&header);
    bytes.extend((header.len() as u32).to_le_bytes());
    let second = blake3::hash(&bytes).to_hex().to_string();
    repo.backend
        .write(&pack_name(&second).unwrap(), &bytes)
        .unwrap();
    repo.write_object(
        "index",
        Label::Index,
        &serde_json::to_vec(&Index {
            version: 1,
            supersedes: vec![],
            packs: vec![Pack {
                id: second,
                blobs: vec![loc.blob.clone()],
            }],
        })
        .unwrap(),
    )
    .unwrap();
    let missing = pack_name(&loc.pack).unwrap();
    repo.backend.delete(&missing).unwrap();
    for report in [check(&repo).unwrap(), verify(&repo, None).unwrap()] {
        assert_eq!(report.damage.len(), 1, "{report:?}");
        assert_eq!(report.damage[0].object, missing.as_str());
        assert_eq!(report.damage[0].problem, "missing");
    }
}
#[test]
fn locks_are_hashed_decrypted_and_validated_and_foreign_names_are_notes() {
    let repo = repo();
    let invalid = repo
        .write_object("locks", Label::Lock, br#"{"version":1}"#)
        .unwrap();
    repo.backend
        .plant_raw("data/ab/unsafe�", b"foreign")
        .unwrap();
    repo.backend
        .plant_raw("config.tmp-0123456789abcdef", b"partial")
        .unwrap();
    let report = check(&repo).unwrap();
    assert!(
        report
            .damage
            .iter()
            .any(|d| d.object == format!("locks/{invalid}") && d.problem == "bad-format")
    );
    assert!(report.notes.iter().all(|n| n.problem == "unknown-object"));
    assert_eq!(report.notes.len(), 1);
    let lock = repo.backend.list(Kind::Locks).unwrap()[0].name.clone();
    repo.backend.plant_raw(lock.as_str(), b"bad").unwrap();
    assert!(
        check(&repo)
            .unwrap()
            .damage
            .iter()
            .any(|d| d.object == lock.as_str() && d.problem == "hash-mismatch")
    );
}

#[test]
fn stale_tree_descriptor_damages_the_index_not_the_valid_stored_tree() {
    let repo = repo();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("file"), b"good").unwrap();
    keelhaul::ops::backup::backup(&repo, &[source.path().to_path_buf()], vec![]).unwrap();
    let name = repo.backend.list(Kind::Index).unwrap()[0].name.clone();
    let mut value = Index::parse(&repo.read_object(&name, Label::Index).unwrap()).unwrap();
    value
        .packs
        .iter_mut()
        .flat_map(|p| &mut p.blobs)
        .find(|b| b.kind == BlobKind::Tree)
        .unwrap()
        .raw_length += 1;
    repo.backend.delete(&name).unwrap();
    let id = repo
        .write_object("index", Label::Index, &serde_json::to_vec(&value).unwrap())
        .unwrap();
    for report in [check(&repo).unwrap(), verify(&repo, None).unwrap()] {
        assert_eq!(report.damage.len(), 1, "{report:?}");
        assert_eq!(report.damage[0].object, format!("index/{id}"));
        assert_eq!(report.damage[0].problem, "index-mismatch");
    }
}
