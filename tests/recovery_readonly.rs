use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::{Command, Output},
};
fn run(root: &Path, args: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_keelhaul"));
    if uzers::get_effective_uid() == 0 {
        use std::os::unix::process::CommandExt;
        let mut pending = vec![root.to_owned()];
        while let Some(path) = pending.pop() {
            if path.is_dir() {
                for entry in fs::read_dir(&path).unwrap() {
                    pending.push(entry.unwrap().path());
                }
            }
            std::os::unix::fs::chown(path, Some(65534), Some(65534)).unwrap();
        }
        command.gid(65534).uid(65534);
    }
    command
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
fn permissions(path: &Path, writable: bool) {
    let metadata = fs::symlink_metadata(path).unwrap();
    fs::set_permissions(
        path,
        fs::Permissions::from_mode(if metadata.is_dir() {
            if writable { 0o755 } else { 0o555 }
        } else if writable {
            0o644
        } else {
            0o444
        }),
    )
    .unwrap();
    if metadata.is_dir() {
        for entry in fs::read_dir(path).unwrap() {
            permissions(&entry.unwrap().path(), writable);
        }
    }
}
#[test]
fn read_only_commands_skip_repository_writes_and_writers_reject_no_lock() {
    let scratch = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(scratch.path()).unwrap();
    fs::write(root.join("key"), [7; 32]).unwrap();
    let source = root.join("source");
    fs::write(&source, b"recovery").unwrap();
    assert!(run(&root, &["init"]).status.success());
    assert!(
        run(&root, &["backup", source.to_str().unwrap()])
            .status
            .success()
    );
    let repo = root.join("repo");
    permissions(&repo, false);
    let out = root.join("out");
    let results = [
        run(
            &root,
            &[
                "restore",
                "latest",
                "--target",
                out.to_str().unwrap(),
                "--no-lock",
            ],
        ),
        run(&root, &["snapshots", "--no-lock"]),
        run(&root, &["ls", "latest", "--no-lock"]),
        run(&root, &["check", "--no-lock"]),
        run(&root, &["verify", "--no-lock"]),
    ];
    permissions(&repo, true);
    for result in results {
        assert!(result.status.success(), "{result:?}");
        let warning = String::from_utf8_lossy(&result.stderr);
        assert_eq!(warning.lines().count(), 1, "{warning}");
        assert!(warning.contains("--no-lock"), "{warning}");
    }
    assert_eq!(
        fs::read(out.join(source.strip_prefix("/").unwrap())).unwrap(),
        b"recovery"
    );
    assert!(fs::read_dir(repo.join("locks")).unwrap().next().is_none());
    for args in [
        vec!["backup", source.to_str().unwrap()],
        vec!["prune"],
        vec!["rebuild-index"],
        vec!["key", "list"],
        vec!["unlock"],
        vec!["init"],
        vec!["forget", "--keep-last", "1"],
    ] {
        let mut args = args;
        args.push("--no-lock");
        let output = run(&root, &args);
        assert_eq!(output.status.code(), Some(2), "{output:?}");
    }
}

#[test]
fn reading_or_rebuilding_an_incomplete_backup_never_returns_exit_five() {
    let scratch = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(scratch.path()).unwrap();
    fs::write(root.join("key"), [7; 32]).unwrap();
    let source = root.join("source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("file"), b"backed up").unwrap();
    let denied = source.join("denied");
    fs::write(&denied, b"unreadable").unwrap();
    fs::set_permissions(&denied, fs::Permissions::from_mode(0o000)).unwrap();
    assert!(run(&root, &["init"]).status.success());
    let output = run(&root, &["backup", source.to_str().unwrap(), "--json"]);
    fs::set_permissions(&denied, fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(output.status.code(), Some(5), "{output:?}");
    let backup: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(backup["incomplete"], true);
    for args in [
        vec!["rebuild-index"],
        vec!["rebuild-index", "--dry-run"],
        vec!["snapshots", "--no-lock"],
        vec!["ls", "latest", "--no-lock"],
        vec!["check", "--no-lock"],
        vec!["verify", "--no-lock"],
    ] {
        let output = run(&root, &args);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
    }
    let out = root.join("out");
    let output = run(
        &root,
        &[
            "restore",
            "latest",
            "--target",
            out.to_str().unwrap(),
            "--no-lock",
        ],
    );
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        fs::read(out.join(source.strip_prefix("/").unwrap()).join("file")).unwrap(),
        b"backed up"
    );
}
