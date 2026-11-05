use super::*;
use crate::{backend::memory::MemoryBackend, ops::backup::backup, secret::Secret};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
#[test]
fn per_item_conflict_continues_and_directory_metadata_is_applied() {
    let scratch = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(scratch.path()).unwrap();
    let source = root.join("source");
    std::fs::create_dir(&source).unwrap();
    for name in ["a", "b", "z"] {
        std::fs::write(source.join(name), name).unwrap();
    }
    std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o751)).unwrap();
    let repo = Repository::init(MemoryBackend::new(), &Secret::KeyFile([1; 32])).unwrap();
    backup(&repo, std::slice::from_ref(&source), vec![]).unwrap();
    let target = root.join("out");
    let report = restore_with_hook(&repo, "latest", &target, &[], None, |path| {
        if path.file_name() == Some(std::ffi::OsStr::new("b")) {
            std::fs::write(target.join(path), "existing").unwrap();
        }
    })
    .unwrap();
    assert!(report.damaged.is_empty());
    assert_eq!(report.failed.len(), 1);
    assert_eq!(
        report.failed[0].path,
        source
            .strip_prefix("/")
            .unwrap()
            .join("b")
            .to_string_lossy()
    );
    let restored = target.join(source.strip_prefix("/").unwrap());
    assert_eq!(std::fs::read(restored.join("z")).unwrap(), b"z");
    assert_eq!(std::fs::read(restored.join("b")).unwrap(), b"existing");
    assert_eq!(std::fs::metadata(restored).unwrap().mode() & 0o777, 0o751);
}
#[cfg(target_os = "macos")]
#[test]
fn folding_directory_and_file_names_are_refused_without_merging() {
    // Use distinct Linux-compatible tree entries without needing a case-sensitive
    // source volume: mutate the authenticated tree through the fixture writer.
    use crate::{
        crypto::envelope::Label,
        format::{pack::BlobKind, tree::Tree},
    };
    let scratch = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(scratch.path()).unwrap();
    let source = root.join("source");
    std::fs::create_dir_all(source.join("Dir")).unwrap();
    std::fs::write(source.join("A"), b"first").unwrap();
    std::fs::write(source.join("Dir/x"), b"first").unwrap();
    std::fs::write(source.join("z"), b"last").unwrap();
    let repo = Repository::init(MemoryBackend::new(), &Secret::KeyFile([1; 32])).unwrap();
    let original = backup(&repo, std::slice::from_ref(&source), vec![]).unwrap();
    let (_, mut snapshot) = repo.select_snapshot(&original.snapshot).unwrap();
    let mut store = BlobStore::load(&repo).unwrap();
    let mut roots = Tree::parse(&store.read(BlobKind::Tree, &snapshot.tree).unwrap()).unwrap();
    let Node::Dir { subtree } = &mut roots.entries[0].node else {
        panic!()
    };
    let mut tree = Tree::parse(&store.read(BlobKind::Tree, subtree).unwrap()).unwrap();
    for (old, new) in [("A", "a"), ("Dir", "dir")] {
        let mut entry = tree
            .entries
            .iter()
            .find(|e| e.name.as_deref() == Some(old))
            .unwrap()
            .clone();
        entry.name = Some(new.into());
        if let Node::Dir { subtree } = &mut entry.node {
            let mut children = Tree::parse(&store.read(BlobKind::Tree, subtree).unwrap()).unwrap();
            children.entries[0].name = Some("y".into());
            *subtree = store
                .add(BlobKind::Tree, &serde_json::to_vec(&children).unwrap())
                .unwrap();
        }
        tree.entries.push(entry);
    }
    tree.entries.sort_by(|a, b| a.name.cmp(&b.name));
    *subtree = store
        .add(BlobKind::Tree, &serde_json::to_vec(&tree).unwrap())
        .unwrap();
    snapshot.tree = store
        .add(BlobKind::Tree, &serde_json::to_vec(&roots).unwrap())
        .unwrap();
    store.finish().unwrap();
    let id = repo
        .write_object(
            "snapshots",
            Label::Snapshot,
            &serde_json::to_vec(&snapshot).unwrap(),
        )
        .unwrap();
    let target = root.join("out");
    let report = restore(&repo, &id, &target, &[]).unwrap();
    assert_eq!(report.failed.len(), 2);
    assert!(
        report
            .failed
            .iter()
            .all(|f| f.error.contains("name collides with"))
    );
    let restored = target.join(source.strip_prefix("/").unwrap());
    assert!(restored.join("z").exists());
    assert_eq!(std::fs::read(restored.join("A")).unwrap(), b"first");
    assert!(restored.join("Dir/x").exists());
    assert!(!restored.join("Dir/y").exists());
}
