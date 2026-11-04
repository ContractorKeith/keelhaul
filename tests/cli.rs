use std::process::{Command, Output};

fn cli(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_keelhaul"))
        .env_remove("KEELHAUL_REPO")
        .env_remove("KEELHAUL_PASSWORD_FILE")
        .env_remove("KEELHAUL_PASSWORD_COMMAND")
        .env_remove("KEELHAUL_KEY_FILE")
        .args(args)
        .output()
        .expect("test binary must run")
}

#[test]
fn version() {
    let output = cli(&["version"]);
    assert!(output.status.success());
    assert_eq!(
        output.stdout,
        format!("keelhaul {}\n", env!("CARGO_PKG_VERSION")).as_bytes()
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn version_json_is_one_document() {
    for args in [&["version", "--json"][..], &["--json", "version"][..]] {
        let output = cli(args);
        assert!(output.status.success());
        assert!(output.stderr.is_empty());
        let value: serde_json::Value =
            serde_json::from_slice(&output.stdout).expect("exactly one JSON document");
        assert_eq!(
            value,
            serde_json::json!({"version": env!("CARGO_PKG_VERSION")})
        );
        assert_eq!(
            output.stdout,
            format!("{{\"version\":\"{}\"}}\n", env!("CARGO_PKG_VERSION")).as_bytes()
        );
    }
}

#[test]
fn usage_errors_are_two() {
    for args in [
        &["--bogus"][..],
        &["version", "--jobs", "0"][..],
        &["version", "--jobs", "bad"][..],
    ] {
        let output = cli(args);
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert!(!output.stderr.is_empty());
    }
    assert!(cli(&["--help"]).status.success());
}

#[test]
fn all_global_flags_are_accepted_after_subcommand() {
    let output = cli(&[
        "version",
        "-r",
        "/unused",
        "--password-file",
        "/password",
        "--password-command",
        "unused",
        "--key-file",
        "/key",
        "--json",
        "-q",
        "-vv",
        "--jobs",
        "8",
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
