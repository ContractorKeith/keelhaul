use std::{fs, process::Command};
#[test]
fn clean_audits_and_unlock_have_one_summary_line() {
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
    for command in ["check", "verify"] {
        let output = run(&[command]);
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            "no damage: 0 packs, 0 snapshots checked\n"
        );
    }
    assert_eq!(
        String::from_utf8(run(&["unlock"]).stdout).unwrap(),
        "removed 0 locks\n"
    );
}
