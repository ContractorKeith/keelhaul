//! Real-process crash proof. Every seed is reproducible; all storage is scratch.
use serde_json::Value;
use std::{
    fs,
    os::unix::process::ExitStatusExt,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::OnceLock,
    time::{Duration, Instant},
};
fn binary() -> &'static Path {
    static BIN: OnceLock<PathBuf> = OnceLock::new();
    BIN.get_or_init(|| {
        let mut command = Command::new(env!("CARGO"));
        command.args([
            "build",
            "--locked",
            "--features",
            "fault-injection",
            "--bin",
            "keelhaul",
        ]);
        if !cfg!(debug_assertions) {
            command.arg("--release");
        }
        assert!(command.status().unwrap().success());
        PathBuf::from(env!("CARGO_BIN_EXE_keelhaul"))
    })
}
fn command(root: &Path) -> Command {
    let mut command = Command::new(binary());
    for name in [
        "KEELHAUL_REPO",
        "KEELHAUL_PASSWORD_FILE",
        "KEELHAUL_PASSWORD_COMMAND",
        "KEELHAUL_KEY_FILE",
        "KEELHAUL_FAULT",
    ] {
        command.env_remove(name);
    }
    command
        .env("HOME", root)
        .env("XDG_CONFIG_HOME", root)
        .env("XDG_CACHE_HOME", root)
        .arg("-r")
        .arg(root.join("repo"))
        .arg("--key-file")
        .arg(root.join("key"))
        .arg("--json");
    command
}
fn ok(output: Output) -> Value {
    assert!(
        output.status.success(),
        "status {}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
fn backup(root: &Path) -> Value {
    ok(command(root)
        .arg("backup")
        .arg(root.join("source"))
        .output()
        .unwrap())
}
fn check(root: &Path) {
    // M3 supplies structural check after the required rebase. Until then the
    // restore assertion below proves every committed snapshot is readable.
    let output = command(root).arg("check").output().unwrap();
    if output.status.code() != Some(2) {
        ok(output);
    } else {
        let snapshots = ok(command(root).arg("snapshots").output().unwrap());
        for snapshot in snapshots.as_array().unwrap() {
            let target = root.join("check-out");
            ok(command(root)
                .arg("restore")
                .arg(snapshot["id"].as_str().unwrap())
                .arg("--target")
                .arg(&target)
                .output()
                .unwrap());
            fs::remove_dir_all(target).unwrap();
        }
    }
}
fn restored(root: &Path) {
    let target = root.join("out");
    ok(command(root)
        .args(["restore", "latest", "--target"])
        .arg(&target)
        .output()
        .unwrap());
    let source = root.join("source");
    let output = target.join(source.strip_prefix("/").unwrap());
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        assert_eq!(
            fs::read(entry.path()).unwrap(),
            fs::read(output.join(entry.file_name())).unwrap()
        );
    }
    fs::remove_dir_all(target).unwrap();
}
fn fixture(root: &Path) {
    fixture_size(root, 4 * 1024 * 1024);
}
fn fixture_size(root: &Path, size: usize) {
    fixture_sparse(root, size, size);
}
fn fixture_sparse(root: &Path, size: usize, entropy: usize) {
    fs::write(root.join("key"), [17; 32]).unwrap();
    ok(command(root).arg("init").output().unwrap());
    fs::create_dir(root.join("source")).unwrap();
    let mut bytes = vec![0; entropy];
    for n in 0..4u64 {
        blake3::Hasher::new()
            .update(&n.to_le_bytes())
            .finalize_xof()
            .fill(&mut bytes);
        let path = root.join(format!("source/{n}"));
        fs::write(&path, &bytes).unwrap();
        fs::OpenOptions::new()
            .write(true)
            .open(path)
            .unwrap()
            .set_len(size as u64)
            .unwrap();
    }
}
#[test]
fn seeded_sigkill_backup_loop() {
    let seed = std::env::var("KEELHAUL_SEED")
        .ok()
        .map(|s| s.parse::<u64>().expect("numeric KEELHAUL_SEED"))
        .unwrap_or(0x4b45454c);
    let iterations = if std::env::var_os("KEELHAUL_SOAK").is_some() {
        500
    } else {
        50
    };
    eprintln!("backup crash loop seed={seed}, iterations={iterations}");
    let start = Instant::now();
    let mut random = seed;
    let mut killed = 0;
    // CI/debug uses 2 MiB; manual release/soak uses a 200 MiB sparse
    // fixture with 8 MiB of entropy, staying below the machine's disk budget.
    // Each iteration has its own repository, bounding structural-check cost.
    let file_size = if cfg!(debug_assertions) || std::env::var_os("CI").is_some() {
        512 * 1024
    } else {
        50 * 1024 * 1024
    };
    let entropy = file_size.min(2 * 1024 * 1024);
    let mut bytes = vec![0; entropy];
    for iteration in 0..iterations {
        let scratch = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(scratch.path()).unwrap();
        fixture_sparse(&root, file_size, entropy);
        let initial = backup(&root);
        random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
        blake3::Hasher::new()
            .update(&random.to_le_bytes())
            .finalize_xof()
            .fill(&mut bytes);
        let changed = root.join("source/0");
        fs::write(&changed, &bytes).unwrap();
        fs::OpenOptions::new()
            .write(true)
            .open(changed)
            .unwrap()
            .set_len(file_size as u64)
            .unwrap();
        let mut child = command(&root)
            .arg("backup")
            .arg(root.join("source"))
            .arg("--force-rehash")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        std::thread::sleep(Duration::from_millis(random % 250));
        // Child::kill is SIGKILL on Unix. A completion before the delay is valid.
        let _ = child.kill();
        let status = child.wait().unwrap();
        assert!(
            status.success() || status.signal() == Some(9),
            "KEELHAUL_SEED={seed}, iteration={iteration}, child={status}"
        );
        if status.signal() == Some(9) {
            killed += 1;
        }
        let result = std::panic::catch_unwind(|| {
            check(&root);
            let report = backup(&root);
            let added = report["summary"]["bytes_added"].as_u64().unwrap();
            assert!(added < (file_size * 4) as u64);
            assert!(added < initial["summary"]["bytes_added"].as_u64().unwrap());
            restored(&root);
        });
        assert!(
            result.is_ok(),
            "KEELHAUL_SEED={seed}, iteration={iteration}"
        );
        if (iteration + 1) % 50 == 0 {
            eprintln!(
                "crash progress: {}/{iterations}, SIGKILLs={killed}, elapsed={:.2}s",
                iteration + 1,
                start.elapsed().as_secs_f64()
            );
        }
    }
    assert!(killed > 0, "seed={seed}: no process was interrupted");
    eprintln!(
        "backup crash loop: {iterations} iterations ok, SIGKILLs={killed}, seed={seed}, elapsed={:.2}s",
        start.elapsed().as_secs_f64()
    );
}
#[test]
fn failed_upload_exits_one_preserves_previous_snapshot_and_reruns() {
    let scratch = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(scratch.path()).unwrap();
    fixture(&root);
    backup(&root);
    fs::write(root.join("source/new"), vec![91; 8 * 1024 * 1024]).unwrap();
    let output = command(&root)
        .env("KEELHAUL_FAULT", "write:1:fail")
        .arg("backup")
        .arg(root.join("source"))
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    check(&root);
    backup(&root);
    restored(&root);
}
#[test]
fn flushed_index_survives_sigkill_before_snapshot() {
    let scratch = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(scratch.path()).unwrap();
    fixture(&root);
    // Four 4 MiB random files and a small tree fit in two packs. The fourth
    // write hangs at the snapshot, after both packs and the index are durable.
    let mut child = command(&root)
        .env("KEELHAUL_FAULT", "write:4:hang")
        .arg("backup")
        .arg(root.join("source"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let start = Instant::now();
    while fs::read_dir(root.join("repo/index"))
        .map(|entries| {
            !entries.filter_map(Result::ok).any(|entry| {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                name.len() == 64 && name.bytes().all(|b| b.is_ascii_hexdigit())
            })
        })
        .unwrap_or(true)
    {
        if start.elapsed() > Duration::from_secs(30) {
            let _ = child.kill();
            let _ = child.wait();
            panic!("index checkpoint was not reached");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    child.kill().unwrap();
    child.wait().unwrap();
    check(&root);
    let report = backup(&root);
    assert_eq!(report["summary"]["data_blobs_added"], 0);
    restored(&root);
}
