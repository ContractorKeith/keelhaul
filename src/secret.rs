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
    load_with_confirmation(file, command, key, false)
}

pub fn load_for_init(
    file: Option<&Path>,
    command: Option<&str>,
    key: Option<&Path>,
) -> Result<Secret, Error> {
    load_with_confirmation(file, command, key, true)
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
    if let Some(path) = file {
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
    fn confirmation_checks_both_prompts() {
        for (answers, confirm, succeeds, calls) in [
            (vec!["same", "same"], true, true, 2),
            (vec!["first", "different"], true, false, 2),
            (vec!["single"], false, true, 1),
        ] {
            let mut answers = answers.into_iter();
            let mut prompts = Vec::new();
            let result = prompt_with(confirm, |message| {
                prompts.push(message.to_owned());
                Ok(answers.next().unwrap().into())
            });
            assert_eq!(result.is_ok(), succeeds);
            if !succeeds {
                assert!(matches!(result, Err(Error::Mismatch)));
            }
            assert_eq!(prompts.len(), calls);
            assert_eq!(prompts[0], "Password: ");
            if confirm {
                assert_eq!(prompts[1], "Confirm password: ");
            }
        }
        assert!(matches!(
            prompt_with(true, |_| Err(io::Error::other("read failed"))),
            Err(Error::Io(_))
        ));
    }

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
