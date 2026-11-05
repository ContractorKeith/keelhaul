#[test]
fn parent_include_is_usage_even_before_writing_the_target() {
    use keelhaul::{
        backend::memory::MemoryBackend,
        ops::{backup::backup, restore::restore},
        repo::Repository,
        secret::Secret,
    };
    let dir = tempfile::tempdir().unwrap();
    let file = std::fs::canonicalize(dir.path()).unwrap().join("file");
    std::fs::write(&file, b"data").unwrap();
    let repo = Repository::init(MemoryBackend::new(), &Secret::KeyFile([7; 32])).unwrap();
    backup(&repo, &[file], vec![]).unwrap();
    let error = restore(
        &repo,
        "latest",
        &dir.path().join("out"),
        &["../../etc".into()],
    )
    .unwrap_err();
    assert_eq!(
        keelhaul::exit::failure(&error),
        keelhaul::exit::Failure::Usage
    );
    assert!(!dir.path().join("out").exists());
}
