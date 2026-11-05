use serde::Serialize;
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};

// One command-wide context also covers warnings from backend/backup workers.
static QUIET: AtomicBool = AtomicBool::new(false);
static JSON: AtomicBool = AtomicBool::new(false);
static WARNED: std::sync::LazyLock<std::sync::Mutex<std::collections::HashSet<String>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashSet::new()));
pub fn configure(quiet: bool, json: bool) {
    WARNED.lock().unwrap_or_else(|e| e.into_inner()).clear();
    QUIET.store(quiet, Ordering::Relaxed);
    JSON.store(json, Ordering::Relaxed);
}
fn informational() -> bool {
    !QUIET.load(Ordering::Relaxed) || JSON.load(Ordering::Relaxed)
}
pub fn warning(args: std::fmt::Arguments<'_>) {
    if informational() {
        eprintln!("{args}");
    }
}
pub fn damage(value: &impl std::fmt::Display) {
    let text = value.to_string();
    let text = text.strip_prefix("damage: ").unwrap_or(&text);
    if WARNED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(text.into())
    {
        eprintln!("{text}");
    }
}

#[derive(Serialize)]
struct Version<'a> {
    version: &'a str,
}

pub fn version(mut out: impl Write, json: bool) -> io::Result<()> {
    let version = env!("CARGO_PKG_VERSION");
    if json {
        serde_json::to_writer(&mut out, &Version { version })?;
        writeln!(out)
    } else {
        writeln!(out, "keelhaul {version}")
    }
}

/// Mark stdout failures separately from backend transport failures.
pub fn stdout() -> impl Write {
    StdoutWriter(io::stdout())
}
struct StdoutWriter(io::Stdout);
impl Write for StdoutWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if informational() {
            self.0.write(bytes).map_err(mark)
        } else {
            Ok(bytes.len())
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        self.0.flush().map_err(mark)
    }
}
// io::Error and serde_json forward source() past their own wrappers. Keep
// the stdout marker one level down so repository wrappers also preserve it.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
struct WriteFailure(#[source] StdoutError);
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
struct StdoutError(io::Error);
fn mark(error: io::Error) -> io::Error {
    io::Error::new(error.kind(), WriteFailure(StdoutError(error)))
}
pub fn line(args: std::fmt::Arguments<'_>) -> io::Result<()> {
    let mut out = stdout();
    out.write_fmt(args)?;
    writeln!(out)
}
pub fn broken_pipe(error: &(dyn std::error::Error + 'static)) -> bool {
    let mut current = Some(error);
    while let Some(cause) = current {
        if cause
            .downcast_ref::<StdoutError>()
            .is_some_and(|e| e.0.kind() == io::ErrorKind::BrokenPipe)
        {
            return true;
        }
        current = cause.source();
    }
    false
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn backend_broken_pipe_remains_an_operational_failure() {
        let error = crate::repo::Error::Backend(crate::backend::BackendError::from(
            io::Error::new(io::ErrorKind::BrokenPipe, "SSH transport closed"),
        ));
        assert!(!broken_pipe(&error));
        assert_eq!(crate::exit::failure(&error), crate::exit::Failure::Error);
        assert!(broken_pipe(&mark(io::Error::from(
            io::ErrorKind::BrokenPipe
        ))));
    }
}
