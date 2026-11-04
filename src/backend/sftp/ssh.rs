use super::url::SftpUrl;
use std::{ffi::OsStr, process::Command};
pub(super) fn command(url: &SftpUrl, executable: impl AsRef<OsStr>, args: &[String]) -> Command {
    let mut command = Command::new(executable);
    // ssh uses the first value: explicit user arguments override these defaults.
    command.args(args).args([
        "-o",
        "ServerAliveInterval=15",
        "-o",
        "ServerAliveCountMax=4",
        "-o",
        "BatchMode=yes",
    ]);
    if let Some(port) = url.port {
        command.args(["-p", &port.to_string()]);
    }
    command.arg(&url.host).args(["-s", "sftp"]);
    command
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sftp_ssh_defaults_follow_user_overrides() {
        let url = SftpUrl::parse("sftp://user@host:2222/repo").unwrap();
        let defaults = [
            "-o",
            "ServerAliveInterval=15",
            "-o",
            "ServerAliveCountMax=4",
            "-o",
            "BatchMode=yes",
            "-p",
            "2222",
            "user@host",
            "-s",
            "sftp",
        ];
        let built = command(&url, "custom-ssh", &[]);
        assert_eq!(built.get_program(), "custom-ssh");
        assert_eq!(
            built.get_args().collect::<Vec<_>>(),
            defaults.iter().map(OsStr::new).collect::<Vec<_>>()
        );
        let overrides = [
            "-F",
            "test config",
            "-o",
            "ServerAliveInterval=1",
            "-o",
            "ServerAliveCountMax=2",
            "-o",
            "BatchMode=no",
        ];
        let built = command(
            &url,
            "ssh",
            &overrides
                .iter()
                .map(|s| (*s).to_owned())
                .collect::<Vec<_>>(),
        );
        assert_eq!(
            built.get_args().collect::<Vec<_>>(),
            overrides
                .iter()
                .chain(defaults.iter())
                .map(OsStr::new)
                .collect::<Vec<_>>()
        );
    }
}
