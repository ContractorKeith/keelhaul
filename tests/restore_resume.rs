use keelhaul::{
    backend::memory::MemoryBackend,
    ops::{
        backup::backup,
        restore::{Options, restore, restore_with_options},
    },
    repo::Repository,
    secret::Secret,
};
#[test]
fn resume_skips_only_identical_regular_files_and_cleans_stale_temps() {
    let dir = tempfile::tempdir().unwrap();
    let source = std::fs::canonicalize(dir.path()).unwrap().join("source");
    std::fs::create_dir(&source).unwrap();
    for (name, bytes) in [("a", "snapshot"), ("b", "second"), ("empty", "")] {
        std::fs::write(source.join(name), bytes).unwrap();
    }
    let repo = Repository::init(MemoryBackend::new(), &Secret::KeyFile([7; 32])).unwrap();
    backup(&repo, std::slice::from_ref(&source), vec![]).unwrap();
    let target = dir.path().join("out");
    restore(&repo, "latest", &target, &[]).unwrap();
    let out = target.join(source.strip_prefix("/").unwrap());
    std::fs::remove_file(out.join("b")).unwrap();
    let stale = out.join(format!(".keelhaul-{}", "ab".repeat(16)));
    std::fs::write(&stale, b"partial").unwrap();
    let unrelated = out.join(".keelhaul-not-a-temp");
    std::fs::write(&unrelated, b"keep").unwrap();
    let options = Options {
        skip_identical: true,
        ..Options::default()
    };
    let report = restore_with_options(&repo, "latest", &target, &[], &options).unwrap();
    assert!(report.failed.is_empty(), "{report:?}");
    assert!(!stale.exists());
    assert!(unrelated.exists());
    assert_eq!(std::fs::read(out.join("b")).unwrap(), b"second");
    std::fs::write(out.join("a"), b"changed!").unwrap(); // Same size, wrong bytes.
    std::fs::remove_file(out.join("b")).unwrap();
    std::os::unix::fs::symlink(source.join("b"), out.join("b")).unwrap();
    let report = restore_with_options(&repo, "latest", &target, &[], &options).unwrap();
    assert_eq!(report.failed.len(), 2, "{report:?}");
    assert_eq!(std::fs::read(out.join("a")).unwrap(), b"changed!");
}
