use serde::Serialize;
use std::io::{self, Write};

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
        self.0.write(bytes).map_err(mark)
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
