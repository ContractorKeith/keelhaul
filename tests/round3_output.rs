use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_keelhaul"))
        .env_clear()
        .arg("-r")
        .arg(root.join("repo"))
        .arg("--key-file")
        .arg(root.join("key"))
        .args(args)
        .output()
        .unwrap()
}
fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("key"), [7; 32]).unwrap();
    assert!(run(dir.path(), &["init"]).status.success());
    dir
}
#[test]
fn verify_summary_counts_only_selected_packs() {
    let dir = fixture();
    for bytes in ["first", "other"] {
        let source = fs::canonicalize(dir.path()).unwrap().join("file");
        fs::write(&source, bytes).unwrap();
        assert!(
            run(dir.path(), &["backup", source.to_str().unwrap()])
                .status
                .success()
        );
    }
    let ids: Vec<_> = fs::read_dir(dir.path().join("repo/data"))
        .unwrap()
        .flat_map(|e| fs::read_dir(e.unwrap().path()).unwrap())
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    let subset: keelhaul::ops::verify::Subset = "1/100000".parse().unwrap();
    let expected = ids.iter().filter(|id| subset.contains(id)).count();
    assert!(expected < ids.len());
    let out = run(dir.path(), &["verify", "--subset", "1/100000"]);
    assert!(out.status.success(), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stdout).contains(&format!("no damage: {expected} packs")),
        "{out:?}"
    );
}

#[test]
fn snapshot_columns_align_to_the_widest_host() {
    let dir = fixture();
    let source = fs::canonicalize(dir.path()).unwrap().join("file");
    fs::write(&source, b"contents").unwrap();
    for host in ["a", "very-long-hostname"] {
        assert!(
            run(
                dir.path(),
                &["backup", source.to_str().unwrap(), "--host", host]
            )
            .status
            .success()
        );
    }
    let out = run(dir.path(), &["snapshots"]);
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    let lines: Vec<_> = text.lines().collect();
    assert_eq!(lines.len(), 3);
    let column = lines[0].find("PATHS").unwrap();
    for row in &lines[1..] {
        assert_eq!(row.find(source.to_str().unwrap()), Some(column), "{text}");
    }
}

#[test]
fn backup_summary_includes_unsupported_types_in_the_total() {
    let dir = fixture();
    let source = dir.path().join("source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("file"), b"data").unwrap();
    assert!(
        Command::new("mkfifo")
            .arg(source.join("fifo"))
            .status()
            .unwrap()
            .success()
    );
    let missing = dir.path().join("missing");
    let out = run(
        dir.path(),
        &[
            "backup",
            source.to_str().unwrap(),
            missing.to_str().unwrap(),
        ],
    );
    assert_eq!(out.status.code(), Some(5), "{out:?}");
    for bytes in [&out.stdout, &out.stderr] {
        assert!(
            String::from_utf8_lossy(bytes).contains("2 skipped (1 unsupported types)"),
            "{out:?}"
        );
    }
}
