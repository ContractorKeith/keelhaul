use keelhaul::{
    backend::memory::MemoryBackend,
    ops::{backup::backup, listing::ls, restore::restore},
    repo::Repository,
    secret::Secret,
};
#[test]
fn include_and_listing_resolve_parent_aliases_and_missing_listing_fails() {
    let scratch = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(scratch.path()).unwrap();
    let source = root.join("source");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("file"), b"data").unwrap();
    std::os::unix::fs::symlink(&root, root.join("alias")).unwrap();
    let repo = Repository::init(MemoryBackend::new(), &Secret::KeyFile([1; 32])).unwrap();
    backup(&repo, std::slice::from_ref(&source), vec![]).unwrap();
    assert_eq!(
        ls(&repo, "latest", Some(&root.join("alias/source")))
            .unwrap()
            .len(),
        2
    );
    let target = root.join("out");
    restore(&repo, "latest", &target, &[root.join("alias/source/file")]).unwrap();
    assert_eq!(
        std::fs::read(target.join(source.strip_prefix("/").unwrap()).join("file")).unwrap(),
        b"data"
    );
    let error = ls(&repo, "latest", Some(&root.join("missing"))).unwrap_err();
    assert!(error.to_string().contains("path not found in snapshot:"));
    let invalid = source.join("file/child/grandchild");
    for error in [
        ls(&repo, "latest", Some(&invalid)).unwrap_err(),
        restore(
            &repo,
            "latest",
            &root.join("invalid-out"),
            std::slice::from_ref(&invalid),
        )
        .unwrap_err(),
    ] {
        assert!(error.to_string().contains(&invalid.display().to_string()));
    }
}
