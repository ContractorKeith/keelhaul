use super::*;
use std::os::unix::process::ExitStatusExt;
fn copy_repo(from: &Path, to: &Path) {
    fs::create_dir(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_repo(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}
fn prune(root: &Path) {
    ok(command(root).arg("prune").output().unwrap());
}
fn verify(root: &Path) {
    assert!(
        ok(command(root).arg("verify").output().unwrap())["damage"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
#[test]
fn seeded_sigkill_prune_loop_and_backup_exclusion() {
    let seed = crate::seed::seed();
    let iterations = if std::env::var_os("KEELHAUL_SOAK").is_some() {
        500
    } else {
        50
    };
    eprintln!("prune crash loop seed={seed}, iterations={iterations}");
    binary();
    let scratch = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(scratch.path()).unwrap();
    fs::write(root.join("key"), [17; 32]).unwrap();
    fs::create_dir(root.join("source")).unwrap();
    initialize(&root);
    let mut data = vec![0; 64 * 1024];
    for generation in 0..3u64 {
        for n in 0..12u64 {
            // Keep four chunks from the first pack so prune must repack it.
            if generation > 0 && n >= 8 {
                continue;
            }
            if generation == 2 && n < 4 {
                fs::remove_file(root.join(format!("source/{n}"))).unwrap();
                continue;
            }
            blake3::Hasher::new()
                .update(&(generation * 12 + n).to_le_bytes())
                .finalize_xof()
                .fill(&mut data);
            fs::write(root.join(format!("source/{n}")), &data).unwrap();
        }
        backup(&root);
    }
    ok(command(&root)
        .args(["forget", "--keep-last", "1"])
        .output()
        .unwrap());
    copy_repo(&root.join("repo"), &root.join("template"));
    let start = Instant::now();
    let summary = ok(command(&root).arg("prune").output().unwrap());
    assert!(summary["packs_repacked"].as_u64().unwrap() > 0);
    let baseline = start.elapsed();
    fs::remove_dir_all(root.join("repo")).unwrap();
    copy_repo(&root.join("template"), &root.join("repo"));
    for hook in ["delete:1:panic", "write:4:panic"] {
        let output = command(&root)
            .env("KEELHAUL_FAULT", hook)
            .arg("prune")
            .output()
            .unwrap();
        assert!(!output.status.success(), "fault {hook} must fire");
        check(&root);
        verify(&root);
        restored(&root);
        prune(&root);
        fs::remove_dir_all(root.join("repo")).unwrap();
        copy_repo(&root.join("template"), &root.join("repo"));
    }
    // Stall after the exclusive lock, while uploading the replacement pack.
    let mut child = command(&root)
        .env("KEELHAUL_FAULT", "write:2:hang")
        .arg("prune")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let backend = keelhaul::backend::local::LocalBackend::open(root.join("repo")).unwrap();
    let start = Instant::now();
    while backend.list(Kind::Locks).unwrap().is_empty() {
        if start.elapsed() > Duration::from_secs(20) {
            let _ = child.kill();
            let _ = child.wait();
            panic!("prune never acquired lock");
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    let output = command(&root)
        .arg("backup")
        .arg(root.join("source"))
        .output()
        .unwrap();
    let status = output.status.code();
    child.kill().unwrap();
    child.wait().unwrap();
    assert_eq!(
        status,
        Some(4),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "backup must fail immediately"
    );
    check(&root);
    verify(&root);
    prune(&root);
    let start = Instant::now();
    let mut random = seed;
    let mut killed = 0;
    for iteration in 0..iterations {
        fs::remove_dir_all(root.join("repo")).unwrap();
        copy_repo(&root.join("template"), &root.join("repo"));
        random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
        let nanos = (u128::from(random) * baseline.as_nanos().max(1) * 2) >> 64;
        let mut child = command(&root)
            .arg("prune")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        std::thread::sleep(Duration::from_nanos(nanos.try_into().unwrap()));
        let _ = child.kill();
        let status = child.wait().unwrap();
        assert!(
            status.success() || status.signal() == Some(9),
            "seed={seed}, iteration={iteration}: {status}"
        );
        killed += usize::from(status.signal() == Some(9));
        check(&root);
        verify(&root);
        prune(&root);
        verify(&root);
        if (iteration + 1) % 50 == 0 {
            eprintln!(
                "prune progress: {}/{iterations}, SIGKILLs={killed}, elapsed={:.2}s",
                iteration + 1,
                start.elapsed().as_secs_f64()
            );
        }
    }
    restored(&root);
    assert!(killed > 0, "seed={seed}: must exercise SIGKILL");
    eprintln!(
        "prune crash loop: {iterations} iterations ok, SIGKILLs={killed}, seed={seed}, elapsed={:.2}s",
        start.elapsed().as_secs_f64()
    );
}

#[path = "m5_prune_fixture.rs"]
mod setup;
#[test]
fn panic_after_first_delete_keeps_snapshot_restorable() {
    let scratch = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(scratch.path()).unwrap();
    fs::write(root.join("key"), [13; 32]).unwrap();
    let (_, snapshot, contents) =
        setup::fixture(keelhaul::backend::local::LocalBackend::new(root.join("repo")).unwrap());
    let output = command(&root)
        .env("KEELHAUL_FAULT", "delete:1:panic")
        .args(["prune", "--max-unused", "100"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("injected delete worker panic"));
    check(&root);
    verify(&root);
    ok(command(&root)
        .args(["restore", &snapshot, "--target"])
        .arg(root.join("restored"))
        .output()
        .unwrap());
    assert_eq!(
        fs::read(root.join("restored/file")).unwrap(),
        contents[..7].concat()
    );
}
