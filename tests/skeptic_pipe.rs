use std::{
    fs,
    io::{BufRead, BufReader},
    process::{Command, Stdio},
};
#[test]
fn closed_stdout_is_quiet_success() {
    let root = tempfile::tempdir().unwrap();
    let key = root.path().join("key");
    fs::write(&key, [1; 32]).unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_keelhaul"))
            .env_clear()
            .env("HOME", root.path())
            .env("PATH", std::env::var_os("PATH").unwrap())
            .arg("-r")
            .arg(root.path().join("repo"))
            .arg("--key-file")
            .arg(&key)
            .args(args)
            .output()
            .unwrap()
    };
    assert!(run(&["init"]).status.success());
    let source = root.path().join("source");
    fs::create_dir(&source).unwrap();
    for n in 0..3000 {
        fs::write(source.join(format!("file-{n:04}")), []).unwrap();
    }
    assert!(run(&["backup", source.to_str().unwrap()]).status.success());
    let mut child = Command::new(env!("CARGO_BIN_EXE_keelhaul"))
        .env_clear()
        .env("HOME", root.path())
        .env("PATH", std::env::var_os("PATH").unwrap())
        .arg("-r")
        .arg(root.path().join("repo"))
        .arg("--key-file")
        .arg(&key)
        .args(["ls", "latest"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut pipe = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    pipe.read_line(&mut line).unwrap();
    assert!(!line.is_empty());
    drop(pipe);
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{:?}", output);
    assert!(output.stderr.is_empty(), "{:?}", output);
    // A disconnected socket makes BrokenPipe deterministic before the child
    // starts, including serde_json and repository error wrappers in key output.
    for args in [
        vec!["key", "list"],
        vec!["key", "list", "--json"],
        vec!["version", "--json"],
    ] {
        let (writer, reader) = std::os::unix::net::UnixStream::pair().unwrap();
        drop(reader);
        let output = Command::new(env!("CARGO_BIN_EXE_keelhaul"))
            .env_clear()
            .env("HOME", root.path())
            .env("PATH", std::env::var_os("PATH").unwrap())
            .arg("-r")
            .arg(root.path().join("repo"))
            .arg("--key-file")
            .arg(&key)
            .args(&args)
            .stdout(Stdio::from(std::os::fd::OwnedFd::from(writer)))
            .output()
            .unwrap();
        assert!(output.status.success(), "{args:?}: {output:?}");
        assert!(output.stderr.is_empty(), "{args:?}: {output:?}");
    }
}
