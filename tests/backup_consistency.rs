use keelhaul::{
    backend::memory::MemoryBackend,
    crypto::envelope::Label,
    format::{pack::BlobKind, snapshot::Snapshot},
    ops::{backup::backup, check::check, restore::restore, verify::verify},
    repo::{Repository, blobs::BlobStore},
    secret::Secret,
};
use serde_json::{Value, json};
use std::fs;
fn repo() -> Repository<MemoryBackend> {
    Repository::init(MemoryBackend::new(), &Secret::KeyFile([7; 32])).unwrap()
}
fn entry(name: &str, node: Value) -> Value {
    let mut value = json!({"name":name,"mode":493,"mtime":"2026-11-04T14:03:22.123456789Z","ctime":"2026-11-04T14:03:22.123456789Z","uid":0,"gid":0,"inode":9,"device":1,"links":1});
    value
        .as_object_mut()
        .unwrap()
        .extend(node.as_object().unwrap().clone());
    value
}
fn tree(store: &mut BlobStore<'_, MemoryBackend>, entries: Vec<Value>) -> String {
    store
        .add(
            BlobKind::Tree,
            &serde_json::to_vec(&json!({"version":1,"entries":entries})).unwrap(),
        )
        .unwrap()
}
fn snapshot(repo: &Repository<MemoryBackend>, tree: String, paths: Vec<&str>) -> String {
    let value = json!({"version":1,"time":"2026-11-04T14:03:22.123456789Z","host":keelhaul::host::hostname(),"username":"test","paths":paths,"tags":[],"excludes":[],"tree":tree,"parent":null,"program":"test","summary":{"files_new":0,"files_changed":0,"files_unchanged":0,"dirs":0,"files_skipped":0,"data_blobs_added":0,"bytes_added":0,"bytes_processed":0}});
    let bytes = serde_json::to_vec(&value).unwrap();
    Snapshot::parse(&bytes).unwrap();
    repo.write_object("snapshots", Label::Snapshot, &bytes)
        .unwrap()
}
#[test]
fn overlapping_inputs_are_backed_up_once() {
    let scratch = tempfile::tempdir().unwrap();
    let source = fs::canonicalize(scratch.path()).unwrap().join("source");
    fs::create_dir_all(source.join("nested")).unwrap();
    fs::write(source.join("nested/file"), b"bytes").unwrap();
    let repo = repo();
    let report = backup(&repo, &[source.clone(), source.join("nested")], vec![]).unwrap();
    assert_eq!(report.summary.files_new, 1);
    assert_eq!(
        repo.snapshots().unwrap()[0].1.paths,
        vec![source.to_str().unwrap()]
    );
    let out = scratch.path().join("out");
    assert!(
        restore(&repo, &report.snapshot, &out, &[])
            .unwrap()
            .damaged
            .is_empty()
    );
    assert_eq!(
        fs::read(
            out.join(source.strip_prefix("/").unwrap())
                .join("nested/file")
        )
        .unwrap(),
        b"bytes"
    );
}
#[test]
fn crafted_overlaps_damage_only_affected_subtree_and_all_readers_agree() {
    for symlink in [false, true] {
        let repo = repo();
        let mut store = BlobStore::load(&repo).unwrap();
        let data = store.add(BlobKind::Data, b"good").unwrap();
        let file = json!({"type":"file","size":4,"content":[data]});
        let nested = tree(&mut store, vec![entry("file", file.clone())]);
        let parent = tree(
            &mut store,
            vec![
                entry("keep", file.clone()),
                entry("nested", json!({"type":"dir","subtree":nested})),
            ],
        );
        let root_node = if symlink {
            json!({"type":"symlink","target":"outside"})
        } else {
            json!({"type":"dir","subtree":parent})
        };
        let root = tree(
            &mut store,
            vec![
                entry("%2Fa", root_node),
                entry("%2Fa%2Fnested", json!({"type":"dir","subtree":nested})),
                entry("%2Fother", file),
            ],
        );
        store.finish().unwrap();
        let id = snapshot(&repo, root, vec!["/a", "/a/nested", "/other"]);
        let scratch = tempfile::tempdir().unwrap();
        let restored = restore(&repo, &id, scratch.path(), &[]).unwrap();
        let audited = check(&repo).unwrap();
        let verified = verify(&repo, None).unwrap();
        assert_eq!(restored.damaged.len(), 1, "{restored:?}");
        assert_eq!(audited.damage, verified.damage);
        assert_eq!(restored.damaged, audited.damage);
        assert_eq!(fs::read(scratch.path().join("other")).unwrap(), b"good");
        if !symlink {
            assert_eq!(fs::read(scratch.path().join("a/keep")).unwrap(), b"good");
        }
        assert!(fs::symlink_metadata(scratch.path().join("a/nested")).is_err());
    }
}
#[test]
fn malformed_root_names_have_shared_damage_and_leave_good_roots_restorable() {
    let repo = repo();
    let mut store = BlobStore::load(&repo).unwrap();
    let data = store.add(BlobKind::Data, b"good").unwrap();
    let file = json!({"type":"file","size":4,"content":[data]});
    let root = tree(
        &mut store,
        vec![entry("%2Fbad%XX", file.clone()), entry("%2Fgood", file)],
    );
    store.finish().unwrap();
    let id = snapshot(&repo, root, vec!["/bad", "/good"]);
    let scratch = tempfile::tempdir().unwrap();
    let restored = restore(&repo, &id, scratch.path(), &[]).unwrap();
    let checked = check(&repo).unwrap();
    assert_eq!(checked.damage, verify(&repo, None).unwrap().damage);
    let mut damaged = restored.damaged;
    damaged.sort();
    assert_eq!(damaged, checked.damage);
    assert_eq!(fs::read(scratch.path().join("good")).unwrap(), b"good");
}
