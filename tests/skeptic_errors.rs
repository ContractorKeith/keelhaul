use keelhaul::backend::{BackendError, ObjectName};
#[test]
fn backend_io_display_names_paths_without_debug_wrappers() {
    let error = BackendError::Io {
        name: Some(ObjectName::new("config").unwrap()),
        path: Some("/scratch/config".into()),
        source: std::io::Error::other("transport failed"),
    };
    assert_eq!(
        error.to_string(),
        "I/O error for config at /scratch/config: transport failed"
    );
    let error = BackendError::from(std::io::Error::other("transport failed"));
    assert_eq!(error.to_string(), "I/O error: transport failed");
}

#[test]
fn busy_directory_display_names_the_object_and_path_once() {
    let name = ObjectName::new(&format!("locks/{}", "ab".repeat(32))).unwrap();
    let path = std::path::PathBuf::from("/scratch/repository/locks");
    let cause = format!(
        "repository directory busy: {}; another keelhaul process may be suspended",
        path.display()
    );
    let error = BackendError::Io {
        name: Some(name.clone()),
        path: Some(path),
        source: std::io::Error::other(cause.clone()),
    };
    assert_eq!(error.to_string(), format!("I/O error for {name}: {cause}"));
}

#[test]
fn busy_repository_lock_directory_has_one_clean_cli_diagnostic() {
    use keelhaul::{backend::local::LocalBackend, repo::Repository, secret::Secret};
    use std::{
        fs,
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    let scratch = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(scratch.path()).unwrap();
    let repository = root.join("repository");
    Repository::init(
        LocalBackend::new(&repository).unwrap(),
        &Secret::KeyFile([1; 32]),
    )
    .unwrap();
    let key = root.join("key");
    fs::write(&key, [1; 32]).unwrap();
    let locks = repository.join("locks");
    fs::create_dir(&locks).unwrap();
    let held = fs::File::open(&locks).unwrap();
    held.lock().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_keelhaul"))
        .env_clear()
        .env("HOME", &root)
        .env("PATH", std::env::var_os("PATH").unwrap())
        .arg("-r")
        .arg(&repository)
        .arg("--key-file")
        .arg(&key)
        .arg("snapshots")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let start = Instant::now();
    while child.try_wait().unwrap().is_none() {
        if start.elapsed() > Duration::from_secs(45) {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("busy directory command exceeded its deadline");
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    let output = child.wait_with_output().unwrap();
    drop(held);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    let name = stderr
        .strip_prefix("keelhaul: I/O error for ")
        .unwrap()
        .split(": repository directory busy: ")
        .next()
        .unwrap();
    assert!(name.starts_with("locks/"));
    ObjectName::new(name).unwrap();
    assert_eq!(
        stderr,
        format!(
            "keelhaul: I/O error for {name}: repository directory busy: {}; another keelhaul process may be suspended\n",
            locks.display()
        )
    );
    eprint!("{stderr}");
}
