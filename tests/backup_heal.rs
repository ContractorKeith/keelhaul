use keelhaul::{
    backend::local::LocalBackend,
    format::pack::BlobKind,
    ops::{
        backup::{Options, backup, backup_with_options},
        forget::{Options as Forget, forget},
    },
    repo::{
        Repository,
        blobs::{BlobStore, pack_name},
    },
    secret::Secret,
};
use std::{fs, process::Command};

#[test]
fn heal_data_only_pack_then_restore_prune_verify() {
    let scratch = tempfile::tempdir().unwrap();
    let source = fs::canonicalize(scratch.path()).unwrap().join("source");
    fs::create_dir(&source).unwrap();
    let mut bytes = vec![0; 1024 * 1024];
    blake3::Hasher::new()
        .update(b"heal A")
        .finalize_xof()
        .fill(&mut bytes);
    fs::write(source.join("A"), &bytes).unwrap();
    fs::write(source.join("B"), b"file B").unwrap();
    let key = scratch.path().join("key");
    fs::write(&key, [19; 32]).unwrap();
    let repository = scratch.path().join("repo");
    let mut repo = Repository::init(
        LocalBackend::new(&repository).unwrap(),
        &Secret::KeyFile([19; 32]),
    )
    .unwrap();
    repo.config.pack_target = 128 * 1024;
    backup(&repo, std::slice::from_ref(&source), vec![]).unwrap();
    fs::remove_file(source.join("B")).unwrap();
    backup(&repo, std::slice::from_ref(&source), vec![]).unwrap();
    forget(
        &repo,
        &Forget {
            keep_last: Some(1),
            ..Forget::default()
        },
    )
    .unwrap();
    let store = BlobStore::load(&repo).unwrap();
    let data = store
        .index
        .iter()
        .find(|((kind, _), loc)| {
            *kind == BlobKind::Data
                && loc.blob.raw_length > 6 // B is six bytes; corrupt retained A.
                && !store
                    .index
                    .iter()
                    .any(|((kind, _), other)| *kind == BlobKind::Tree && other.pack == loc.pack)
        })
        .unwrap()
        .1
        .clone();
    let name = pack_name(&data.pack).unwrap();
    let path = repository.join(name.as_str());
    let mut damaged = fs::read(&path).unwrap();
    damaged[data.blob.offset as usize + 24] ^= 1;
    fs::write(&path, damaged).unwrap();
    let ordinary = backup_with_options(
        &repo,
        std::slice::from_ref(&source),
        vec![],
        &Options {
            force_rehash: true,
            ..Options::default()
        },
    )
    .unwrap();
    assert_eq!(ordinary.summary.data_blobs_added, 0);
    let binary = std::env::var_os("KEELHAUL_TEST_BIN")
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_keelhaul").into());
    let run = |args: &[&str]| {
        let result = Command::new(&binary)
            .env("HOME", scratch.path())
            .env_remove("KEELHAUL_PASSWORD_FILE")
            .env_remove("KEELHAUL_PASSWORD_COMMAND")
            .env_remove("KEELHAUL_FAULT")
            .arg("-r")
            .arg(&repository)
            .arg("--key-file")
            .arg(&key)
            .args(args)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        println!(
            "{args:?}: exit 0 {}",
            String::from_utf8_lossy(&result.stdout).trim()
        );
        result
    };
    let healed = run(&["backup", "--heal", source.to_str().unwrap(), "--json"]);
    let report: serde_json::Value = serde_json::from_slice(&healed.stdout).unwrap();
    assert!(report["summary"]["blobs_healed"].as_u64().unwrap() >= 1);
    assert_eq!(report["summary"]["files_read"], 1);
    let output = scratch.path().join("out");
    run(&["restore", "latest", "--target", output.to_str().unwrap()]);
    assert_eq!(
        fs::read(output.join(source.strip_prefix("/").unwrap()).join("A")).unwrap(),
        bytes
    );
    println!("restore A: byte-equal");
    run(&["prune"]);
    assert!(!path.exists());
    run(&["verify"]);
    let second = run(&["backup", "--heal", source.to_str().unwrap(), "--json"]);
    let report: serde_json::Value = serde_json::from_slice(&second.stdout).unwrap();
    assert_eq!(report["summary"]["blobs_healed"], 0);
    assert_eq!(report["summary"]["data_blobs_added"], 0);
}
