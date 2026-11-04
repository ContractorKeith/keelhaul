use crate::format::key_slot::SlotKind;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::{
    fs,
    io::{self, IsTerminal, Write},
    path::Path,
    process::Command,
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
    #[error("no secret source supplied and no TTY available")]
    NoTty,
    #[error("password command failed")]
    CommandFailed,
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
    let count =
        usize::from(file.is_some()) + usize::from(command.is_some()) + usize::from(key.is_some());
    if count > 1 {
        return Err(Error::ConflictingSources);
    }
    if let Some(file) = file {
        return password(fs::read(file)?);
    }
    if let Some(command) = command {
        let output = Command::new("sh").arg("-c").arg(command).output()?;
        if !output.status.success() {
            return Err(Error::CommandFailed);
        }
        return password(output.stdout);
    }
    if let Some(key) = key {
        return Ok(Secret::KeyFile(
            fs::read(key)?.try_into().map_err(|_| Error::KeyLength)?,
        ));
    }
    prompt(false)
}

fn prompt(confirm: bool) -> Result<Secret, Error> {
    if !io::stdin().is_terminal() {
        return Err(Error::NoTty);
    }
    let first = rpassword::prompt_password("Password: ")?;
    if confirm && first != rpassword::prompt_password("Confirm password: ")? {
        return Err(Error::Mismatch);
    }
    Ok(Secret::Password(first))
}

pub fn new_password() -> Result<Secret, Error> {
    if let Some(path) = std::env::var_os("KEELHAUL_NEW_PASSWORD_FILE") {
        return password(fs::read(path)?);
    }
    prompt(true)
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn strips_exactly_one_line_ending() {
        for (input, expected) in [
            ("pw\n", "pw"),
            ("pw\r\n", "pw"),
            ("pw\n\n", "pw\n"),
            ("pw\r", "pw\r"),
        ] {
            let Secret::Password(actual) = password(input.as_bytes().to_vec()).unwrap() else {
                panic!()
            };
            assert_eq!(actual, expected);
        }
    }
}
