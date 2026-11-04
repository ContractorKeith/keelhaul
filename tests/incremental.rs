use keelhaul::{
    backend::{Backend, Kind, memory::MemoryBackend},
    crypto::envelope::Label,
    ops::backup::{Options, backup, backup_with_options},
    repo::Repository,
    secret::Secret,
};
use std::{fs, path::Path};
fn run(
    repo: &Repository<MemoryBackend>,
    root: &Path,
    options: Options,
) -> keelhaul::ops::backup::Report {
    backup_with_options(repo, &[root.into()], vec![], &options).unwrap()
}
fn options(jobs: usize) -> Options {
    Options {
        jobs,
        ..Options::default()
    }
}
#[test]
fn one_of_a_thousand_and_ctime_and_force_rehash() {
    let scratch = tempfile::tempdir().unwrap();
    for n in 0..1000 {
        fs::write(
            scratch.path().join(format!("{n:04}")),
            format!("file {n:04}"),
        )
        .unwrap();
    }
    let repo = Repository::init(MemoryBackend::new(), &Secret::KeyFile([7; 32])).unwrap();
    let first = run(&repo, scratch.path(), options(4));
    assert_eq!(first.summary.files_read, 1000);
    let file = scratch.path().join("0042");
    let mtime = filetime::FileTime::from_last_modification_time(&fs::metadata(&file).unwrap());
    // A real content write changes ctime; restore mtime to defeat size+mtime alone.
    fs::write(&file, b"different").unwrap();
    filetime::set_file_mtime(&file, mtime).unwrap();
    let second = run(&repo, scratch.path(), options(8));
    assert_eq!(second.summary.files_read, 1);
    assert_eq!(second.summary.files_changed, 1);
    assert_eq!(second.summary.files_unchanged, 999);
    assert_eq!(second.summary.data_blobs_added, 1);
    let (_, snapshot) = repo.select_snapshot(&second.snapshot).unwrap();
    assert_eq!(snapshot.parent.as_deref(), Some(first.snapshot.as_str()));
    let mut force = options(4);
    force.force_rehash = true;
    let third = run(&repo, scratch.path(), force);
    assert_eq!(third.summary.files_read, 1000);
    assert_eq!(third.summary.files_unchanged, 1000);
    assert_eq!(third.summary.files_new, 0);
    assert_eq!(third.summary.data_blobs_added, 0);
}
#[test]
fn parent_host_paths_override_and_damaged_tree_fallback() {
    let scratch = tempfile::tempdir().unwrap();
    fs::write(scratch.path().join("a"), b"hello").unwrap();
    let repo = Repository::init(MemoryBackend::new(), &Secret::KeyFile([8; 32])).unwrap();
    let first = run(&repo, scratch.path(), options(1));
    backup(&repo, &[scratch.path().join("a")], vec![]).unwrap();
    let same_paths = run(&repo, scratch.path(), options(1));
    assert_eq!(same_paths.summary.files_read, 0);
    assert_eq!(
        repo.select_snapshot(&same_paths.snapshot)
            .unwrap()
            .1
            .parent
            .as_deref(),
        Some(first.snapshot.as_str())
    );
    let mut other = options(1);
    other.host = Some("different".into());
    assert_eq!(run(&repo, scratch.path(), other).summary.files_read, 1);
    let mut explicit = options(1);
    explicit.parent = Some(first.snapshot[..8].into());
    explicit.host = Some("third".into());
    assert_eq!(run(&repo, scratch.path(), explicit).summary.files_read, 0);
    let (_, snapshot) = repo.select_snapshot(&first.snapshot).unwrap();
    let store = keelhaul::repo::blobs::BlobStore::load(&repo).unwrap();
    let location = &store.index[&(keelhaul::format::pack::BlobKind::Tree, snapshot.tree)];
    let name = keelhaul::repo::blobs::pack_name(&location.pack).unwrap();
    repo.backend
        .plant_raw(name.as_str(), b"broken tree pack")
        .unwrap();
    let mut explicit = options(4);
    explicit.parent = Some(first.snapshot.clone());
    // Exercise the CLI warning contract against an opaque copy of this damaged repo.
    let copy_scratch = tempfile::tempdir().unwrap();
    let local_path = copy_scratch.path().join("damaged-repo");
    let local = keelhaul::backend::local::LocalBackend::new(&local_path).unwrap();
    for kind in Kind::ALL {
        for entry in repo.backend.list(kind).unwrap() {
            local
                .write(&entry.name, &repo.backend.read(&entry.name).unwrap())
                .unwrap();
        }
    }
    let recovered = run(&repo, scratch.path(), explicit);
    assert_eq!(recovered.summary.files_read, 1);
    assert_eq!(recovered.summary.data_blobs_added, 1);
    assert!(
        repo.select_snapshot(&recovered.snapshot)
            .unwrap()
            .1
            .parent
            .is_none()
    );
    let key = scratch.path().join("key");
    fs::write(&key, [8; 32]).unwrap();
    // Keep the backed-up root distinct from the copied repository and secret.
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_keelhaul"))
        .env("HOME", scratch.path())
        .env_remove("KEELHAUL_PASSWORD_FILE")
        .env_remove("KEELHAUL_PASSWORD_COMMAND")
        .env_remove("KEELHAUL_FAULT")
        .arg("-r")
        .arg(local_path)
        .arg("--key-file")
        .arg(key)
        .arg("backup")
        .arg(scratch.path().join("a"))
        .args(["--parent", &first.snapshot, "--json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let warning = String::from_utf8_lossy(&output.stderr);
    assert!(warning.contains("warning: parent") && warning.contains("reading all files"));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["summary"]["files_read"], 1);
}
#[test]
fn tree_id_is_independent_of_jobs_and_duplicate_chunks_are_stored_once() {
    let scratch = tempfile::tempdir().unwrap();
    fs::create_dir(scratch.path().join("nested")).unwrap();
    let data = vec![42; 1024 * 1024];
    for n in 0..32 {
        fs::write(scratch.path().join(format!("nested/{n}")), &data).unwrap();
    }
    let repo = Repository::init(MemoryBackend::new(), &Secret::KeyFile([9; 32])).unwrap();
    let a = run(&repo, scratch.path(), options(1));
    assert_eq!(a.summary.data_blobs_added, 1);
    let mut parallel = options(8);
    parallel.force_rehash = true;
    let b = run(&repo, scratch.path(), parallel);
    assert_eq!(b.summary.data_blobs_added, 0);
    assert_eq!(
        repo.select_snapshot(&a.snapshot).unwrap().1.tree,
        repo.select_snapshot(&b.snapshot).unwrap().1.tree
    );
    // Check fresh concurrent reservations as well, not only existing-index dedup.
    let fresh = Repository::init(MemoryBackend::new(), &Secret::KeyFile([9; 32])).unwrap();
    assert_eq!(
        run(&fresh, scratch.path(), options(8))
            .summary
            .data_blobs_added,
        1
    );
}
#[test]
fn excludes_golden_and_cli_exclude_file_comments() {
    let scratch = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(scratch.path()).unwrap().join("source");
    for dir in ["nested", "cache", "nested/cache", "sub"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    for file in [
        "a.tmp",
        "keep.tmp",
        "nested/a.tmp",
        "nested/keep.tmp",
        "cache/a",
        "nested/cache/a",
        "sub/root.txt",
        "root.txt",
        "directory",
        "nested/other",
        "nested/root.txt",
        "{a,b}",
        "a",
        "b",
        "trim",
        "trim ",
        "keep",
        "keep ",
    ] {
        fs::write(root.join(file), b"x").unwrap();
    }
    let key = scratch.path().join("key");
    fs::write(&key, [4; 32]).unwrap();
    let exclude_file = scratch.path().join("excludes");
    fs::write(
        &exclude_file,
        "# comment\n\n*.tmp\n!keep.tmp\ncache/\n/sub/root.txt\ndirectory/\n{a,b}\ntrim   \nkeep\\ \n",
    )
    .unwrap();
    let cli = |args: &[&str]| {
        std::process::Command::new(env!("CARGO_BIN_EXE_keelhaul"))
            .env("HOME", scratch.path())
            .env_remove("KEELHAUL_PASSWORD_FILE")
            .env_remove("KEELHAUL_PASSWORD_COMMAND")
            .arg("-r")
            .arg(scratch.path().join("repo"))
            .arg("--key-file")
            .arg(&key)
            .args(args)
            .output()
            .unwrap()
    };
    assert!(cli(&["init"]).status.success());
    let result = cli(&[
        "backup",
        root.to_str().unwrap(),
        "--exclude-file",
        exclude_file.to_str().unwrap(),
        "--json",
    ]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let repo = Repository::open(
        keelhaul::backend::local::LocalBackend::open(scratch.path().join("repo")).unwrap(),
        &Secret::KeyFile([4; 32]),
    )
    .unwrap();
    let (_, snapshot) = repo.select_snapshot("latest").unwrap();
    assert_eq!(
        snapshot.excludes,
        [
            "*.tmp",
            "!keep.tmp",
            "cache/",
            "/sub/root.txt",
            "directory/",
            "{a,b}",
            "trim   ",
            "keep\\ "
        ]
    );
    let output = scratch.path().join("out");
    assert!(
        keelhaul::ops::restore::restore(&repo, "latest", &output, &[])
            .unwrap()
            .damaged
            .is_empty()
    );
    let restored = output.join(root.strip_prefix("/").unwrap());
    for file in [
        "keep.tmp",
        "nested/keep.tmp",
        "root.txt",
        "directory",
        "nested/other",
        "nested/root.txt",
        "a",
        "b",
        "trim ",
        "keep",
    ] {
        assert!(restored.join(file).is_file(), "missing {file}");
    }
    for file in [
        "a.tmp",
        "nested/a.tmp",
        "cache",
        "nested/cache",
        "sub/root.txt",
        "{a,b}",
        "trim",
        "keep ",
    ] {
        assert!(!restored.join(file).exists(), "excluded {file}");
    }
    // `git check-ignore --no-index` agrees: {a,b} is literal, trailing
    // spaces are trimmed, and `keep\ ` matches a name with one final space.
    let result = cli(&[
        "backup",
        root.join("a.tmp").to_str().unwrap(),
        "--exclude",
        "*.tmp",
        "--json",
    ]);
    assert_eq!(result.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&result.stderr).contains("excluded by pattern"));
    assert_eq!(repo.snapshots().unwrap().len(), 1);
}
#[path = "common/incremental_damage.rs"]
mod damage_review;
#[path = "common/incremental_edges.rs"]
mod edges;
#[path = "common/exclude_review.rs"]
mod exclude_review;
