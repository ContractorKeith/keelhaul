//! Seeded process kills with explicit durability and flushed-index reuse assertions.
#[path = "common/crash_support.rs"]
mod support;
use keelhaul::{
    backend::{Backend, Kind},
    format::pack::BlobKind,
};
use std::{
    fs,
    path::Path,
    process::Stdio,
    time::{Duration, Instant},
};
use support::*;
const FILES: usize = 32;
const CHUNK: usize = 512 * 1024;
fn initialize(root: &Path) {
    use keelhaul::{
        backend::local::LocalBackend,
        crypto::envelope::{self, Label},
        repo::{Repository, blobs::BlobStore},
        secret::Secret,
    };
    let mut repo = Repository::init(
        LocalBackend::new(root.join("repo")).unwrap(),
        &Secret::KeyFile([17; 32]),
    )
    .unwrap();
    repo.config.pack_target = 1024 * 1024;
    let encryption = BlobStore::load(&repo).unwrap().worker_keys().1;
    let config = envelope::seal(
        &encryption,
        Label::Config,
        &serde_json::to_vec(&repo.config).unwrap(),
    )
    .unwrap();
    // Fixture-only replacement, before any backup; normal writes remain create-once.
    fs::write(root.join("repo/config"), config).unwrap();
}
fn indexed(root: &Path) -> usize {
    use keelhaul::{
        backend::local::LocalBackend,
        crypto::envelope::Label,
        format::index::Index,
        repo::{
            Repository,
            blobs::{BlobStore, pack_name},
        },
        secret::Secret,
    };
    let repo = Repository::open(
        LocalBackend::open(root.join("repo")).unwrap(),
        &Secret::KeyFile([17; 32]),
    )
    .unwrap();
    for object in repo.backend.list(Kind::Index).unwrap() {
        let index = Index::parse(&repo.read_object(&object.name, Label::Index).unwrap()).unwrap();
        for pack in index.packs {
            let name = pack_name(&pack.id).unwrap();
            let bytes = repo.backend.read(&name).unwrap_or_else(|e| {
                panic!("index names a pack not durable at SIGKILL: {name}: {e}")
            });
            assert_eq!(
                blake3::hash(&bytes).to_hex().as_str(),
                pack.id,
                "indexed pack must be complete"
            );
        }
    }
    BlobStore::load(&repo)
        .unwrap()
        .index
        .keys()
        .filter(|(kind, _)| *kind == BlobKind::Data)
        .count()
}
fn exercise(root: &Path, delay: Duration, fault: Option<(&str, Kind, usize)>) -> (bool, bool) {
    use std::{os::unix::process::ExitStatusExt, process::Stdio};
    let mut cmd = command(root);
    cmd.arg("backup")
        .arg(root.join("source"))
        .arg("--force-rehash")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some((hook, _, _)) = fault {
        cmd.env("KEELHAUL_FAULT", hook);
    }
    let mut child = cmd.spawn().unwrap();
    if let Some((_, kind, minimum)) = fault {
        let backend = keelhaul::backend::local::LocalBackend::open(root.join("repo")).unwrap();
        let start = Instant::now();
        while backend.list(kind).unwrap().len() < minimum {
            if start.elapsed() > Duration::from_secs(30) {
                let _ = child.kill();
                let _ = child.wait();
                panic!("upload checkpoint timed out");
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    std::thread::sleep(delay);
    let _ = child.kill();
    let status = child.wait().unwrap();
    assert!(
        status.success() || status.signal() == Some(9),
        "backup child: {status}"
    );
    let flushed = indexed(root);
    check(root);
    let report = ok(command(root)
        .arg("backup")
        .arg(root.join("source"))
        .arg("--force-rehash")
        .output()
        .unwrap());
    // One exact MIN-sized chunk per file. Only the unindexed tail may be stored;
    // 64 KiB additionally bounds both directory trees, including envelope overhead.
    assert_eq!(
        report["summary"]["data_blobs_added"].as_u64().unwrap(),
        (FILES - flushed) as u64,
        "rerun must reuse every flushed chunk"
    );
    let tail = (FILES - flushed) * (CHUNK + 41) + 64 * 1024;
    assert!(
        report["summary"]["bytes_added"].as_u64().unwrap() <= tail as u64,
        "rerun exceeds unflushed-tail budget"
    );
    assert_eq!(indexed(root), FILES);
    let backend = keelhaul::backend::local::LocalBackend::open(root.join("repo")).unwrap();
    assert!(
        backend.list(Kind::Index).unwrap().len() >= 2,
        "fixture must cross multiple index flushes"
    );
    restored(root);
    (status.signal() == Some(9), flushed > 0)
}
#[test]
fn seeded_sigkill_backup_loop() {
    let seed = std::env::var("KEELHAUL_SEED")
        .ok()
        .map(|s| s.parse::<u64>().expect("numeric seed"))
        .unwrap_or(0x4b45454c);
    let iterations = if std::env::var_os("KEELHAUL_SOAK").is_some() {
        500
    } else {
        50
    };
    eprintln!("backup crash loop seed={seed}, iterations={iterations}");
    binary(); // Compile before measuring the actual initial backup duration.
    let scratch = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(scratch.path()).unwrap();
    fs::write(root.join("key"), [17; 32]).unwrap();
    fs::create_dir(root.join("source")).unwrap();
    let mut data = vec![0; CHUNK];
    for n in 0..FILES {
        blake3::Hasher::new()
            .update(&(n as u64).to_le_bytes())
            .finalize_xof()
            .fill(&mut data);
        fs::write(root.join(format!("source/{n}")), &data).unwrap();
    }
    initialize(&root);
    let start = Instant::now();
    backup(&root);
    let initial = start.elapsed();
    assert_eq!(indexed(&root), FILES);
    fs::remove_dir_all(root.join("repo")).unwrap();
    let start = Instant::now();
    // Durability/resume sentinels make the two ordering mutations deterministic:
    // a stalled first pack must not enter an index; a stalled ninth pack leaves
    // the first eight-pack index reusable. Random kills below cover normal I/O.
    for (hook, kind, minimum) in [
        ("write:1:hang", Kind::Data, 7),
        ("write:10:hang", Kind::Index, 1),
    ] {
        initialize(&root);
        exercise(
            &root,
            Duration::from_millis(100),
            Some((hook, kind, minimum)),
        );
        fs::remove_dir_all(root.join("repo")).unwrap();
    }
    let mut random = seed;
    let mut killed = 0;
    let mut reused = 0;
    for iteration in 0..iterations {
        initialize(&root);
        random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
        let nanos = (random as u128 * initial.as_nanos().max(1)) >> 64;
        let result = std::panic::catch_unwind(|| {
            exercise(&root, Duration::from_nanos(nanos.try_into().unwrap()), None)
        });
        let (was_killed, had_flushed) =
            result.unwrap_or_else(|_| panic!("KEELHAUL_SEED={seed}, iteration={iteration}"));
        killed += usize::from(was_killed);
        reused += usize::from(had_flushed);
        fs::remove_dir_all(root.join("repo")).unwrap();
        if (iteration + 1) % 50 == 0 {
            eprintln!(
                "crash progress: {}/{iterations}, SIGKILLs={killed}, reused={reused}, elapsed={:.2}s",
                iteration + 1,
                start.elapsed().as_secs_f64()
            );
        }
    }
    assert!(
        killed > 0 && reused > 0,
        "seed={seed}: random kills must exercise flushed reuse"
    );
    eprintln!(
        "backup crash loop: {iterations} iterations ok, SIGKILLs={killed}, reused={reused}, seed={seed}, elapsed={:.2}s",
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
#[path = "common/crash_errors.rs"]
mod errors;
