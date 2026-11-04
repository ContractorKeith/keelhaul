//! Force a mixed data/tree pack: parallel backup scheduling must not define the oracle.
#[path = "damage_support/mod.rs"]
mod support;
use keelhaul::{
    backend::{Backend, Kind},
    format::{
        pack::BlobKind,
        tree::{Node, Tree},
    },
    repo::blobs::{BlobStore, pack_name},
};
use support::*;
#[path = "common/backup_fixture.rs"]
#[allow(dead_code, reason = "shared fixture helpers")]
mod backup_fixture;

#[test]
fn deleting_mixed_pack_does_not_guess_files_below_its_missing_tree() {
    let scratch = tempfile::tempdir().unwrap();
    let source = std::fs::canonicalize(scratch.path())
        .unwrap()
        .join("source");
    backup_fixture::damage_fixture(&source);
    let mut f = Fixture::new(&source);
    let mut store = BlobStore::load(&f.repo).unwrap();
    let locations = store.index.values().cloned().collect::<Vec<_>>();
    let mut payloads = Vec::new();
    let mut nested = None;
    for location in locations {
        let blob = location.blob;
        let payload = store.read(blob.kind, &blob.id).unwrap();
        if blob.kind == BlobKind::Tree {
            let tree = Tree::parse(&payload).unwrap();
            if tree.entries.iter().any(|entry| {
                entry.name.as_deref() == Some("shared") && matches!(entry.node, Node::File { .. })
            }) {
                nested = Some(blob.id.clone());
            }
        }
        payloads.push((blob.kind, blob.id, payload));
    }
    let nested = nested.expect("fixture has nested/shared");
    drop(store);
    f.repo.config.pack_target = 16 * 1024 * 1024;
    let mut writer = BlobStore::repack_writer(&f.repo).unwrap();
    for kind in [Kind::Index, Kind::Data] {
        for entry in f.repo.backend.list(kind).unwrap() {
            f.repo.backend.delete(&entry.name).unwrap();
        }
    }
    // All file data and the nested tree share a pack; its ancestors stay separate.
    for (kind, id, payload) in &payloads {
        if *kind == BlobKind::Data || *id == nested {
            writer.add(*kind, payload).unwrap();
        }
    }
    writer.finish().unwrap();
    let missing = pack_name(&writer.index[&(BlobKind::Tree, nested)].pack).unwrap();
    for (kind, id, payload) in &payloads {
        if *kind == BlobKind::Tree && !writer.index.contains_key(&(*kind, id.clone())) {
            writer.add(*kind, payload).unwrap();
        }
    }
    writer.finish().unwrap();
    drop(writer);
    f.recapture_files();
    assert_eq!(
        f.files
            .iter()
            .filter(|file| file.path.ends_with("/nested/shared"))
            .count(),
        3
    );
    assert_eq!(
        f.files
            .iter()
            .filter(|file| file.path.ends_with("/file0"))
            .count(),
        3
    );
    assert!(
        f.files
            .iter()
            .filter(|file| file.path.ends_with("/nested/shared"))
            .all(|file| file.tree_packs.contains(missing.as_str()))
    );
    assert!(
        f.files
            .iter()
            .filter(|file| file.path.ends_with("/file0"))
            .all(|file| !file.tree_packs.contains(missing.as_str()))
    );
    f.stored = Kind::ALL
        .into_iter()
        .flat_map(|kind| f.repo.backend.list(kind).unwrap())
        .map(|entry| {
            (
                entry.name.to_string(),
                f.repo.backend.read(&entry.name).unwrap(),
            )
        })
        .collect();
    support::byte_cases::delete_indexed_pack(&f, 0, missing.as_str());
}
