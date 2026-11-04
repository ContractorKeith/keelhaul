use keelhaul::{
    backend::memory::MemoryBackend,
    ops::{backup::backup, listing::ls, restore::restore},
    repo::Repository,
    secret::Secret,
};
#[test]
fn three_hundred_directories_roundtrip_and_list() {
    let scratch = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(scratch.path()).unwrap();
    let source = root.as_path().join("source");
    std::fs::create_dir(&source).unwrap();
    let mut leaf = source.clone();
    for _ in 0..300 {
        leaf.push("d");
        std::fs::create_dir(&leaf).unwrap();
    }
    leaf.push("leaf");
    std::fs::write(&leaf, b"deep bytes").unwrap();
    let repo = Repository::init(MemoryBackend::new(), &Secret::KeyFile([1; 32])).unwrap();
    backup(&repo, std::slice::from_ref(&source), vec![]).unwrap();
    let output = root.as_path().join("out");
    let report = restore(&repo, "latest", &output, &[]).unwrap();
    assert!(report.damaged.is_empty());
    assert_eq!(
        std::fs::read(output.join(leaf.strip_prefix("/").unwrap())).unwrap(),
        b"deep bytes"
    );
    let entries = ls(&repo, "latest", None).unwrap();
    assert_eq!(entries.len(), 302);
    assert!(entries.iter().any(|e| e.path.ends_with("/leaf")));
}

#[test]
fn root_parent_dotdot_resolves_through_symlink_but_root_symlink_stays_a_symlink() {
    let scratch = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(scratch.path()).unwrap();
    std::fs::create_dir_all(root.join("actual/child")).unwrap();
    std::fs::write(root.join("actual/victim"), b"RIGHT").unwrap();
    std::fs::write(root.join("victim"), b"WRONG").unwrap();
    std::os::unix::fs::symlink("actual/child", root.join("alias")).unwrap();
    let repo = Repository::init(MemoryBackend::new(), &Secret::KeyFile([1; 32])).unwrap();
    backup(
        &repo,
        &[root.join("alias/../victim"), root.join("alias")],
        vec![],
    )
    .unwrap();
    let (_, s) = repo.select_snapshot("latest").unwrap();
    assert!(
        s.paths
            .contains(&root.join("actual/victim").to_string_lossy().to_string())
    );
    let output = root.join("out");
    restore(&repo, "latest", &output, &[]).unwrap();
    assert_eq!(
        std::fs::read(output.join(root.join("actual/victim").strip_prefix("/").unwrap())).unwrap(),
        b"RIGHT"
    );
    assert!(
        std::fs::symlink_metadata(output.join(root.join("alias").strip_prefix("/").unwrap()))
            .unwrap()
            .is_symlink()
    );
}

#[test]
fn percent_and_literal_slash_escape_in_root_name_roundtrip_without_aliasing() {
    let scratch = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(scratch.path()).unwrap();
    let mut paths = Vec::new();
    for (name, bytes) in [
        ("a%2Fb", b"literal slash escape".as_slice()),
        ("a%b", b"literal percent".as_slice()),
        ("a%25b", b"literal percent escape".as_slice()),
    ] {
        let root = root.as_path().join(name);
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("file"), bytes).unwrap();
        paths.push(root);
    }
    let repo = Repository::init(MemoryBackend::new(), &Secret::KeyFile([1; 32])).unwrap();
    backup(&repo, &paths, vec![]).unwrap();
    let target = root.as_path().join("out");
    restore(&repo, "latest", &target, &[]).unwrap();
    for path in paths {
        assert_eq!(
            std::fs::read(path.join("file")).unwrap(),
            std::fs::read(target.join(path.strip_prefix("/").unwrap()).join("file")).unwrap()
        );
    }
}
