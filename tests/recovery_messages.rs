use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_keelhaul"))
        .env_clear()
        .env("HOME", root)
        .env("PATH", std::env::var_os("PATH").unwrap())
        .arg("-r")
        .arg(root.join("repo"))
        .arg("--key-file")
        .arg(root.join("key"))
        .args(args)
        .output()
        .unwrap()
}
#[test]
fn restore_conflicts_go_to_stderr_once_with_absolute_paths() {
    let scratch = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(scratch.path()).unwrap();
    fs::write(root.join("key"), [7; 32]).unwrap();
    let source = root.join("file");
    fs::write(&source, b"saved").unwrap();
    assert!(run(&root, &["init"]).status.success());
    assert!(
        run(&root, &["backup", source.to_str().unwrap()])
            .status
            .success()
    );
    let out = root.join("out");
    let args = ["restore", "latest", "--target", out.to_str().unwrap()];
    assert!(run(&root, &args).status.success());
    let conflict = run(&root, &args);
    assert_eq!(conflict.status.code(), Some(1));
    let stderr = String::from_utf8(conflict.stderr).unwrap();
    assert_eq!(
        stderr.matches(source.to_str().unwrap()).count(),
        1,
        "{stderr}"
    );
    assert!(
        stderr.contains(&format!("{}: restore conflict", source.display())),
        "{stderr}"
    );
    assert!(!String::from_utf8_lossy(&conflict.stdout).contains("restore conflict"));
    let alias = root.join("alias");
    fs::create_dir(root.join("real")).unwrap();
    std::os::unix::fs::symlink(root.join("real"), &alias).unwrap();
    let symlink = run(
        &root,
        &["restore", "latest", "--target", alias.to_str().unwrap()],
    );
    assert_eq!(symlink.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&symlink.stderr)
            .contains("target is a symlink; pass the directory it points to")
    );
    let forget = run(&root, &["forget", "--keep-last", "1", "--prune"]);
    assert!(forget.status.success(), "{forget:?}");
    assert!(
        String::from_utf8_lossy(&forget.stdout).contains("packs removed, 0 repacked"),
        "{forget:?}"
    );
}
#[test]
fn missing_repository_is_usage() {
    let root = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_keelhaul"))
        .env_clear()
        .env("HOME", root.path())
        .arg("snapshots")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2), "{output:?}");
}

#[test]
fn nested_audit_damage_displays_its_object_and_cause_once() {
    let error = keelhaul::repo::Error::Damage {
        object: "snapshots/abc".into(),
        problem: "bad-format",
        detail: "snapshots/abc: bad-format: unreadable snapshot".into(),
    };
    assert_eq!(
        error.to_string(),
        "damage: snapshots/abc: bad-format: unreadable snapshot"
    );
}
