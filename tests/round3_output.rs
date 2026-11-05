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
        let source = dir.path().join("file");
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
