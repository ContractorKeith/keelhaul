//! Owns the ssh child and a bounded synchronous request stream.
use super::{
    codec::{self, Fields, Packet, string, u32_field},
    url::SftpUrl,
};
use std::{
    collections::BTreeMap,
    io,
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    thread::{self, JoinHandle},
};

pub(super) const WINDOW: usize = 4;

pub struct Client {
    child: Child,
    command: Command,
    input: ChildStdin,
    output: ChildStdout,
    stderr: Option<JoinHandle<Vec<u8>>>,
    captured_stderr: Vec<u8>,
    next_id: u32,
    pub posix_rename: bool,
    failed: bool,
    pending: BTreeMap<u32, u8>,
}
impl Client {
    pub fn connect(url: &SftpUrl) -> io::Result<Self> {
        let executable = std::env::var_os("KEELHAUL_SSH_COMMAND").unwrap_or_else(|| "ssh".into());
        let args = match std::env::var("KEELHAUL_SSH_ARGS") {
            Ok(args) => args,
            Err(std::env::VarError::NotPresent) => String::new(),
            Err(error) => return Err(io::Error::new(io::ErrorKind::InvalidInput, error)),
        };
        Self::connect_with(
            url,
            executable,
            &shell_words::split(&args).map_err(io::Error::other)?,
        )
    }
    pub fn connect_with(
        url: &SftpUrl,
        executable: impl AsRef<std::ffi::OsStr>,
        args: &[String],
    ) -> io::Result<Self> {
        let mut command = Command::new(executable);
        command.args(args);
        if let Some(port) = url.port {
            command.args(["-p", &port.to_string()]);
        }
        command.arg(&url.host).args(["-s", "sftp"]);
        Self::spawn(command)
    }
    pub(super) fn spawn(mut command: Command) -> io::Result<Self> {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        // All three pipes are guaranteed by Stdio::piped above.
        let input = child.stdin.take().expect("piped stdin");
        let output = child.stdout.take().expect("piped stdout");
        let mut stderr = child.stderr.take().expect("piped stderr");
        let stderr = thread::Builder::new()
            .name("sftp-stderr".into())
            .spawn(move || super::stderr::drain(&mut stderr));
        let stderr = match stderr {
            Ok(thread) => thread,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        let mut client = Self {
            child,
            command,
            input,
            output,
            stderr: Some(stderr),
            captured_stderr: Vec::new(),
            next_id: 0,
            posix_rename: false,
            failed: false,
            pending: BTreeMap::new(),
        };
        let result = client.handshake();
        match result {
            Ok(()) => Ok(client),
            Err(error) => Err(client.abort(error)),
        }
    }
    fn handshake(&mut self) -> io::Result<()> {
        Packet {
            kind: 1,
            body: 3_u32.to_be_bytes().to_vec(),
        }
        .write(&mut self.input)?;
        let packet = Packet::read(&mut self.output)?;
        if packet.kind != 2 {
            return Err(codec::invalid("expected SFTP VERSION"));
        }
        let mut fields = Fields(&packet.body);
        if fields.u32()? != 3 {
            return Err(codec::invalid("server does not support SFTP v3"));
        }
        while !fields.0.is_empty() {
            let extension = fields.string()?;
            let value = fields.string()?;
            if extension == b"posix-rename@openssh.com" && value == b"1" {
                self.posix_rename = true;
            }
        }
        Ok(())
    }
    pub fn pid(&self) -> u32 {
        self.child.id()
    }
    pub(super) fn abort(&mut self, error: io::Error) -> io::Error {
        self.failed = true;
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(stderr) = self.stderr.take() {
            self.captured_stderr = stderr.join().unwrap_or_default();
        }
        io::Error::new(
            error.kind(),
            format!(
                "SFTP: {error}; ssh: {}",
                String::from_utf8_lossy(&self.captured_stderr)
            ),
        )
    }
    // A failed operation leaves no live child. The next request starts a fresh session.
    fn reconnect(&mut self) -> io::Result<()> {
        let mut command = Command::new(self.command.get_program());
        command.args(self.command.get_args());
        if let Some(directory) = self.command.get_current_dir() {
            command.current_dir(directory);
        }
        for (key, value) in self.command.get_envs() {
            match value {
                Some(value) => {
                    command.env(key, value);
                }
                None => {
                    command.env_remove(key);
                }
            }
        }
        *self = Self::spawn(command)?;
        Ok(())
    }
    pub(super) fn finish<T>(&mut self, result: io::Result<T>) -> io::Result<T> {
        // A valid STATUS is an operation result; it does not corrupt the stream.
        result.map_err(|error| {
            if error.kind() == io::ErrorKind::InvalidData {
                self.abort(error)
            } else {
                error
            }
        })
    }
    pub(super) fn send(&mut self, kind: u8, body: Vec<u8>) -> io::Result<u32> {
        if self.failed {
            self.reconnect()?;
        }
        if self.pending.len() >= WINDOW {
            return Err(self.protocol_error("SFTP request window exceeded"));
        }
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        let mut bytes = Vec::with_capacity(body.len() + 4);
        u32_field(&mut bytes, id);
        bytes.extend(body);
        Packet { kind, body: bytes }
            .write(&mut self.input)
            .map_err(|error| self.abort(error))?;
        self.pending.insert(id, kind);
        Ok(id)
    }
    pub(super) fn receive(&mut self) -> io::Result<(u32, Packet)> {
        let packet = Packet::read(&mut self.output).map_err(|error| self.abort(error))?;
        let mut fields = Fields(&packet.body);
        let id = fields.u32().map_err(|error| self.abort(error))?;
        let body = fields.0.to_vec();
        let Some(request) = self.pending.remove(&id) else {
            return Err(self.protocol_error("unexpected SFTP request ID"));
        };
        let response = Packet {
            kind: packet.kind,
            body,
        };
        super::response::validate(request, &response).map_err(|error| self.abort(error))?;
        Ok((id, response))
    }
    pub(super) fn protocol_error(&mut self, message: &str) -> io::Error {
        self.abort(codec::invalid(message))
    }
    pub(super) fn request(&mut self, kind: u8, body: Vec<u8>) -> io::Result<Packet> {
        let expected = self.send(kind, body)?;
        let (id, packet) = self.receive()?;
        if id != expected {
            return Err(self.protocol_error("unexpected SFTP request ID"));
        }
        Ok(packet)
    }
    pub fn close(&mut self, handle: &[u8]) -> io::Result<()> {
        // Handles from an aborted session cannot be closed on a fresh connection.
        if self.failed {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        let mut body = Vec::new();
        string(&mut body, handle);
        let packet = self.request(4, body)?;
        let result = status(&packet);
        self.finish(result)
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(stderr) = self.stderr.take() {
            let _ = stderr.join();
        }
    }
}

pub(super) use super::status::{payload, status};
