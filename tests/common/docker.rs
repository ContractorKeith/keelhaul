//! Shared opt-in Docker test gate. Services are started by the caller/CI.
#![allow(dead_code, reason = "each e2e binary uses only its own Docker helpers")]
pub fn enabled() -> bool {
    std::env::var("KEELHAUL_E2E").is_ok_and(|value| value == "1")
}

pub fn require_s3() {
    let response = ureq::get("http://127.0.0.1:7070/health")
        .config()
        .timeout_global(Some(std::time::Duration::from_secs(3)))
        .build()
        .call();
    assert!(
        response.is_ok(),
        "KEELHAUL_E2E=1 requires versitygw; run docker compose up -d --wait s3"
    );
}

/// Run one fixture command, retaining stdout and stderr in failure diagnostics.
pub fn run(command: &mut std::process::Command) -> std::process::Output {
    let output = command.output().expect("execute test command");
    assert!(
        output.status.success(),
        "{command:?}: {}\n{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    output
}
pub fn compose(args: &[&str]) {
    run(std::process::Command::new("docker")
        .args(["compose"])
        .args(args));
}
pub fn require_sshd() {
    compose(&["exec", "-T", "sshd", "true"]);
}
