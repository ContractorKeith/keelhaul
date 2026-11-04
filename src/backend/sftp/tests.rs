use super::{client::Client, url::SftpUrl};
use std::{io, process::Command};
fn fake(mode: &str) -> io::Result<Client> {
    let mut command = Command::new("python3");
    command.args([
        "-u",
        "-c",
        include_str!("../../../tests/common/sftp_server.py"),
        mode,
    ]);
    Client::spawn(command)
}
#[test]
fn sftp_client_pipelines_and_orders_replies() {
    let mut client = fake("normal").unwrap();
    assert!(client.posix_rename);
    assert_eq!(
        client.read_file("/file", 0, 131072).unwrap(),
        (0..131072).map(|i| (i % 251) as u8).collect::<Vec<_>>()
    );
    client.write_handle(b"h", &vec![b'x'; 131072]).unwrap();
    client.rename("/a", "/b", true).unwrap();
    client.rename("/a", "/b", false).unwrap();
    let pid = client.pid();
    drop(client);
    assert_reaped(pid);
}
#[test]
fn sftp_client_short_reads() {
    let mut client = fake("short").unwrap();
    assert_eq!(
        client.read_file("/file", 0, 131072).unwrap(),
        (0..131072).map(|i| (i % 251) as u8).collect::<Vec<_>>()
    );
}
#[test]
fn sftp_client_errors_reap_child() {
    for mode in ["oversize", "truncated"] {
        assert!(fake(mode).is_err());
    }
    let mut client = fake("wrongid").unwrap();
    let pid = client.pid();
    assert!(
        client
            .open("/file", 1)
            .unwrap_err()
            .to_string()
            .contains("request ID")
    );
    assert_reaped(pid);
    let mut client = fake("write-error").unwrap();
    let pid = client.pid();
    assert_eq!(
        client
            .write_handle(b"h", &vec![b'x'; 131072])
            .unwrap_err()
            .kind(),
        io::ErrorKind::PermissionDenied
    );
    assert_reaped(pid);
    let mut client = fake("malformed").unwrap();
    let pid = client.pid();
    assert!(client.open("/file", 1).is_err());
    assert_reaped(pid);
    let mut client = fake("missing").unwrap();
    let pid = client.pid();
    assert_eq!(
        client.open("/file", 1).unwrap_err().kind(),
        io::ErrorKind::NotFound
    );
    assert_reaped(pid);
}
#[test]
fn sftp_ssh_stderr_is_preserved() {
    let mut command = Command::new("sh");
    command.args(["-c", "echo fixture-ssh-error >&2; exit 1"]);
    let error = match Client::spawn(command) {
        Ok(_) => panic!("accepted dead server"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("fixture-ssh-error"));
    // URL validation takes place before launching ssh.
    assert!(SftpUrl::parse("sftp:-bad:/repo").is_err());
}
fn assert_reaped(pid: u32) {
    #[cfg(target_os = "linux")]
    assert!(
        !std::path::Path::new(&format!("/proc/{pid}")).exists(),
        "child {pid} not reaped"
    );
    #[cfg(not(target_os = "linux"))]
    let _ = pid;
}

#[test]
fn sftp_expected_absence_does_not_close_session_and_errors_can_reconnect() {
    let mut client = fake("missing").unwrap();
    let original = client.pid();
    assert!(client.read_optional("/file").unwrap().is_none());
    assert_eq!(client.pid(), original);
    assert!(client.open("/file", 1).is_err());
    assert_reaped(original);
    assert!(client.read_optional("/file").unwrap().is_none());
    assert_ne!(client.pid(), original);
}
