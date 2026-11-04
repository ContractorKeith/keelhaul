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
fn sftp_protocol_errors_reap_and_status_errors_keep_session() {
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
    assert_eq!(client.pid(), pid);
    client.close(b"h").unwrap();
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
    assert_eq!(client.pid(), pid);
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
fn sftp_expected_absence_and_missing_errors_keep_session() {
    let mut client = fake("missing").unwrap();
    let original = client.pid();
    assert!(client.matches_file("/file", b"x").unwrap().is_none());
    assert_eq!(client.pid(), original);
    assert!(client.open("/file", 1).is_err());
    assert_eq!(client.pid(), original);
    assert!(client.matches_file("/file", b"x").unwrap().is_none());
    assert_eq!(client.pid(), original);
}

fn logged_fake(mode: &str, log: &std::path::Path) -> Client {
    let mut command = Command::new("python3");
    command
        .args([
            "-u",
            "-c",
            include_str!("../../../tests/common/sftp_server.py"),
            mode,
        ])
        .arg(log);
    Client::spawn(command).unwrap()
}
#[test]
fn sftp_missing_objects_keep_one_session() {
    use crate::backend::{Backend, Kind, ObjectName};
    let scratch = tempfile::tempdir().unwrap();
    let log = scratch.path().join("requests");
    let backend =
        super::SftpBackend::with_client("/repo".into(), logged_fake("backend-missing", &log))
            .unwrap();
    for kind in Kind::ALL {
        assert!(backend.list(kind).unwrap().is_empty());
    }
    for _ in 0..5 {
        assert!(matches!(
            backend.stat(&ObjectName::new("config").unwrap()),
            Err(crate::backend::BackendError::NotFound(_))
        ));
    }
    assert_eq!(
        std::fs::read_to_string(&log)
            .unwrap()
            .lines()
            .filter(|line| *line == "spawn")
            .count(),
        1
    );
}
#[test]
fn sftp_conflicting_length_does_not_read_existing_bytes() {
    use crate::backend::{Backend, BackendError, ObjectName};
    let scratch = tempfile::tempdir().unwrap();
    let log = scratch.path().join("requests");
    let backend =
        super::SftpBackend::with_client("/repo".into(), logged_fake("backend-existing", &log))
            .unwrap();
    assert!(matches!(
        backend.write(&ObjectName::new("config").unwrap(), b"x"),
        Err(BackendError::AlreadyExists(_))
    ));
    assert!(
        !std::fs::read_to_string(&log)
            .unwrap()
            .lines()
            .any(|line| line == "5"),
        "unexpected READ request"
    );
}

#[test]
fn sftp_second_abort_keeps_stderr_tail() {
    let mut command = Command::new("python3");
    command.args(["-u","-c","import sys,struct; sys.stdin.buffer.read(9); print('saved ssh tail',file=sys.stderr,flush=True); sys.stdout.buffer.write(bytes.fromhex('000000050200000003')); sys.stdout.buffer.flush(); sys.stdin.buffer.read()"]);
    let mut client = Client::spawn(command).unwrap();
    let first = client.abort(io::Error::other("first"));
    let second = client.abort(io::Error::other("second"));
    assert!(first.to_string().contains("saved ssh tail"), "{first}");
    assert!(second.to_string().contains("saved ssh tail"), "{second}");
}

#[test]
fn sftp_pipelined_read_status_errors_drain_pending_responses() {
    let mut client = fake("read-error").unwrap();
    let pid = client.pid();
    assert_eq!(
        client.read_handle(b"h", 0, 131072).unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
    client.close(b"h").unwrap();
    assert_eq!(client.pid(), pid);
}

#[test]
fn sftp_directories_cannot_be_objects() {
    use crate::backend::{Backend, BackendError, ObjectName};
    let scratch = tempfile::tempdir().unwrap();
    let log = scratch.path().join("requests");
    let backend =
        super::SftpBackend::with_client("/repo".into(), logged_fake("backend-directory", &log))
            .unwrap();
    let name = ObjectName::new("config").unwrap();
    for result in [
        backend.read(&name).map(|_| ()),
        backend.stat(&name).map(|_| ()),
        backend.write(&name, b""),
    ] {
        let error = result.unwrap_err();
        assert!(
            matches!(&error,BackendError::Io { path:Some(path),.. } if path.to_str() == Some("/repo/config")),
            "{error}"
        );
    }
}
