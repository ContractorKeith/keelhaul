use crate::format::key_slot::SlotKind;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::{
    fs,
    io::{self, IsTerminal, Read, Write},
    path::Path,
    process::{Command, Stdio},
};

// Deliberately no Debug: secret bytes must never reach diagnostics.
pub enum Secret {
    Password(String),
    KeyFile([u8; 32]),
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("exactly one secret source is required")]
    ConflictingSources,
    #[error("password is empty")]
    EmptyPassword,
    #[error("new key file must be outside the repository")]
    KeyInRepository,
    #[error("password command output exceeds 1 MiB")]
    CommandOutputTooLarge,
    #[error("no secret source supplied and no TTY available")]
    NoTty,
    #[error("password command failed{0}")]
    CommandFailed(String),
    #[error("password is not UTF-8")]
    InvalidUtf8,
    #[error("key file must contain exactly 32 bytes")]
    KeyLength,
    #[error("passwords do not match")]
    Mismatch,
    #[error(transparent)]
    Io(#[from] io::Error),
}

impl Secret {
    pub fn validate_new(&self) -> Result<(), Error> {
        if matches!(self, Self::Password(password) if password.is_empty()) {
            return Err(Error::EmptyPassword);
        }
        Ok(())
    }

    pub fn kind(&self) -> SlotKind {
        match self {
            Self::Password(_) => SlotKind::Password,
            Self::KeyFile(_) => SlotKind::Keyfile,
        }
    }
}

fn password(mut bytes: Vec<u8>) -> Result<Secret, Error> {
    if bytes.last() == Some(&b'\n') {
        bytes.pop();
        if bytes.last() == Some(&b'\r') {
            bytes.pop();
        }
    }
    Ok(Secret::Password(
        String::from_utf8(bytes).map_err(|_| Error::InvalidUtf8)?,
    ))
}

pub fn load(
    file: Option<&Path>,
    command: Option<&str>,
    key: Option<&Path>,
) -> Result<Secret, Error> {
    load_with_confirmation(file, command, key, false)
}

pub fn load_for_init(
    file: Option<&Path>,
    command: Option<&str>,
    key: Option<&Path>,
) -> Result<Secret, Error> {
    let secret = load_with_confirmation(file, command, key, true)?;
    secret.validate_new()?;
    Ok(secret)
}

fn load_with_confirmation(
    file: Option<&Path>,
    command: Option<&str>,
    key: Option<&Path>,
    confirm: bool,
) -> Result<Secret, Error> {
    let count =
        usize::from(file.is_some()) + usize::from(command.is_some()) + usize::from(key.is_some());
    if count > 1 {
        return Err(Error::ConflictingSources);
    }
    if let Some(file) = file {
        return password(fs::read(file)?);
    }
    if let Some(command) = command {
        return password(command_password(command)?);
    }
    if let Some(key) = key {
        return Ok(Secret::KeyFile(
            fs::read(key)?.try_into().map_err(|_| Error::KeyLength)?,
        ));
    }
    prompt(confirm)
}

fn prompt(confirm: bool) -> Result<Secret, Error> {
    if !io::stdin().is_terminal() {
        return Err(Error::NoTty);
    }
    prompt_with(confirm, |message| rpassword::prompt_password(message))
}

fn prompt_with(
    confirm: bool,
    mut read: impl FnMut(&str) -> io::Result<String>,
) -> Result<Secret, Error> {
    let first = read("Password: ")?;
    if confirm && first != read("Confirm password: ")? {
        return Err(Error::Mismatch);
    }
    Ok(Secret::Password(first))
}

pub fn new_password(file: Option<&Path>) -> Result<Secret, Error> {
    let secret = if let Some(path) = file {
        password(fs::read(path)?)?
    } else {
        prompt(true)?
    };
    secret.validate_new()?;
    Ok(secret)
}

pub fn new_key_file(path: &Path) -> Result<Secret, Error> {
    let bytes = crate::crypto::keys::random::<32>()?;
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(path)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    Ok(Secret::KeyFile(bytes))
}

// Read at most one byte beyond the limit, then kill and reap the command on
// every early error. Drain stderr concurrently, retaining only a bounded tail.
fn command_password(command: &str) -> Result<Vec<u8>, Error> {
    const LIMIT: u64 = 1024 * 1024;
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(command)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| io::Error::other("password command stderr unavailable"))?;
    let diagnostics = std::thread::spawn(move || stderr_tail(stderr));
    let result = (|| {
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("password command stdout unavailable"))?;
        let mut bytes = Vec::new();
        stdout.take(LIMIT + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > LIMIT {
            return Err(Error::CommandOutputTooLarge);
        }
        Ok(bytes)
    })();
    if result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
        let _ = diagnostics.join();
        return result;
    }
    let status = child.wait();
    let tail = diagnostics
        .join()
        .map_err(|_| io::Error::other("stderr reader panicked"))??;
    if !status?.success() {
        return Err(Error::CommandFailed(if tail.is_empty() {
            String::new()
        } else {
            format!(": {tail}")
        }));
    }
    result
}

#[cfg(test)]
mod tests;

fn stderr_tail(mut input: impl Read) -> io::Result<String> {
    let mut tail = std::collections::VecDeque::with_capacity(2048);
    let mut buffer = [0; 4096];
    loop {
        let n = input.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        for byte in &buffer[..n] {
            if tail.len() == 2048 {
                tail.pop_front();
            }
            tail.push_back(*byte);
        }
    }
    let bytes = tail.into_iter().collect::<Vec<_>>();
    let text = String::from_utf8_lossy(&bytes);
    let lines = text.lines().collect::<Vec<_>>();
    let text = lines[lines.len().saturating_sub(10)..].join("\n");
    // Lossy UTF-8 decoding can expand a truncated codepoint; retain the byte cap.
    let mut start = text.len().saturating_sub(2048);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    Ok(text[start..].to_owned())
}
