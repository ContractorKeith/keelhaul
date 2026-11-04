#![forbid(unsafe_code)]
#[path = "common/backend_edges.rs"]
mod backend_edges;
#[path = "common/backend_suite.rs"]
mod backend_suite;
#[path = "common/e2e_sftp_links.rs"]
mod links;
use keelhaul::backend::{
    Backend, Kind, ObjectName,
    sftp::{SftpBackend, client::Client, url::SftpUrl},
};
use std::{
    fs,
    path::Path,
    process::{Command, Output},
    time::{Duration, Instant},
};

fn run(command: &mut Command) -> Output {
    let output = command.output().expect("execute test command");
    assert!(
        output.status.success(),
        "{command:?}: {}\n{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    output
}
fn compose(args: &[&str]) {
    run(Command::new("docker").args(["compose"]).args(args));
}
fn connect(config: &Path, root: &str) -> Client {
    Client::connect_with(
        &SftpUrl::parse(&format!("sftp:keelhaul-e2e:{root}")).unwrap(),
        "ssh",
        &["-F".into(), config.to_string_lossy().into_owned()],
    )
    .unwrap()
}
fn config(scratch: &Path, hosts: &Path) -> String {
    format!(
        "Host keelhaul-e2e\n HostName 127.0.0.1\n Port 2222\n User keelhaul\n IdentityFile {}\n IdentitiesOnly yes\n UserKnownHostsFile {}\n StrictHostKeyChecking accept-new\n BatchMode yes\n ConnectTimeout 5\n",
        scratch.join("id_ed25519").display(),
        hosts.display()
    )
}
#[test]
fn sftp_e2e_conformance_and_failures() {
    if std::env::var("KEELHAUL_E2E").as_deref() != Ok("1") {
        return;
    }
    // A requested run against a stopped container must fail before fixture setup.
    compose(&["exec", "-T", "sshd", "true"]);
    let scratch = tempfile::tempdir().unwrap();
    run(Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-f"])
        .arg(scratch.path().join("id_ed25519")));
    fs::copy(
        scratch.path().join("id_ed25519.pub"),
        scratch.path().join("authorized_keys"),
    )
    .unwrap();
    run(Command::new("docker")
        .args(["compose", "up", "-d", "--wait", "--force-recreate", "sshd"])
        .env("KEELHAUL_SSHD_KEYS", scratch.path()));
    let config_path = scratch.path().join("ssh_config");
    fs::write(
        &config_path,
        config(scratch.path(), &scratch.path().join("known_hosts")),
    )
    .unwrap();
    let root = format!(
        "/config/repo-{}",
        scratch.path().file_name().unwrap().to_string_lossy()
    );
    let backend = SftpBackend::with_client(root.clone(), connect(&config_path, &root)).unwrap();
    run(Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "sftp_env_conformance", "--nocapture"])
        .env("KEELHAUL_SFTP_FIXTURE_ROOT", &root)
        .env(
            "KEELHAUL_SSH_ARGS",
            format!("-F {}", shell_words::quote(&config_path.to_string_lossy())),
        ));

    links::run(&config_path, &root);

    // Independent connections also obey create-once publication.
    let other = SftpBackend::with_client(root.clone(), connect(&config_path, &root)).unwrap();
    let object = ObjectName::new("config").unwrap();
    let barrier = std::sync::Barrier::new(2);
    std::thread::scope(|scope| {
        let a = scope.spawn(|| {
            barrier.wait();
            backend.write(&object, &vec![1; 262144])
        });
        let b = scope.spawn(|| {
            barrier.wait();
            other.write(&object, &vec![2; 262144])
        });
        assert_ne!(a.join().unwrap().is_ok(), b.join().unwrap().is_ok());
    });
    backend.delete(&object).unwrap();

    // Observe the actual temporary upload before interrupting its transport.
    let doomed_root = format!("{root}-interrupted");
    let doomed =
        SftpBackend::with_client(doomed_root.clone(), connect(&config_path, &doomed_root)).unwrap();
    #[cfg(target_os = "linux")]
    let pid = doomed.child_pid().unwrap();
    let mut observer = connect(&config_path, &doomed_root);
    std::thread::scope(|scope| {
        let writer = scope.spawn(|| doomed.write(&object, &vec![7; 64 * 1024 * 1024]));
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if observer
                .directory(&doomed_root)
                .unwrap()
                .iter()
                .any(|e| e.name.starts_with(b"config.tmp-"))
            {
                break;
            }
            assert!(Instant::now() < deadline, "upload did not start");
            std::thread::sleep(Duration::from_millis(5));
        }
        compose(&["kill", "sshd"]);
        let error = writer.join().unwrap().unwrap_err();
        assert!(
            matches!(error, keelhaul::backend::BackendError::Io { .. }),
            "{error}"
        );
        assert_eq!(keelhaul::exit::code(Err(keelhaul::exit::Failure::Error)), 1);
        #[cfg(target_os = "linux")]
        assert!(
            !Path::new(&format!("/proc/{pid}")).exists(),
            "ssh child not reaped"
        );
    });
    run(Command::new("docker")
        .args(["compose", "up", "-d", "--wait", "sshd"])
        .env("KEELHAUL_SSHD_KEYS", scratch.path()));
    let recovered =
        SftpBackend::with_client(doomed_root.clone(), connect(&config_path, &doomed_root)).unwrap();
    assert!(recovered.list(Kind::Config).unwrap().is_empty());
    assert!(
        recovered
            .list_foreign(Kind::Config)
            .unwrap()
            .iter()
            .any(|e| e.raw.starts_with("config.tmp-"))
    );
    recovered
        .write(&object, b"retry after interruption")
        .unwrap();
    assert_eq!(
        recovered.read(&object).unwrap(),
        b"retry after interruption"
    );

    run(Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-f"])
        .arg(scratch.path().join("wrong")));
    let key = fs::read_to_string(scratch.path().join("wrong.pub")).unwrap();
    let wrong_hosts = scratch.path().join("wrong_hosts");
    fs::write(&wrong_hosts, format!("[127.0.0.1]:2222 {key}")).unwrap();
    let wrong_config = scratch.path().join("wrong_config");
    fs::write(&wrong_config, config(scratch.path(), &wrong_hosts)).unwrap();
    let result = Client::connect_with(
        &SftpUrl::parse("sftp:keelhaul-e2e:/config").unwrap(),
        "ssh",
        &["-F".into(), wrong_config.to_string_lossy().into_owned()],
    );
    let error = match result {
        Ok(_) => panic!("wrong host key accepted"),
        Err(error) => error,
    };
    assert!(
        error
            .to_string()
            .contains("REMOTE HOST IDENTIFICATION HAS CHANGED"),
        "{error}"
    );
}

#[test]
fn sftp_env_conformance() {
    let Ok(root) = std::env::var("KEELHAUL_SFTP_FIXTURE_ROOT") else {
        return;
    };
    let backend = SftpBackend::new(&format!("sftp:keelhaul-e2e:{root}")).unwrap();
    backend_suite::run_suite(&backend, true, |raw, bytes| {
        backend.plant_raw(raw, bytes).unwrap()
    });
}
