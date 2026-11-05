//! Resource exhaustion is a run failure, never a reason to omit a file.
use crate::repo::Error;
use std::{io, path::Path};

// Refuse a worker count that cannot fit even the ordinary descriptor budget.
// Runtime classification below also covers inherited descriptors and system limits.
pub(super) fn jobs(jobs: usize) -> Result<(), Error> {
    let limit = rustix::process::getrlimit(rustix::process::Resource::Nofile).current;
    if limit.is_some_and(|limit| jobs as u64 * 2 + 16 > limit) {
        return Err(Error::Refused("worker file-descriptor budget exceeds ulimit -n; lower --jobs or raise ulimit -n; no snapshot written".into()));
    }
    Ok(())
}

pub(super) fn check(path: &Path, error: &io::Error) -> Result<(), Error> {
    if error.raw_os_error().is_some_and(|code| {
        [
            rustix::io::Errno::MFILE,
            rustix::io::Errno::NFILE,
            rustix::io::Errno::NOMEM,
        ]
        .iter()
        .any(|errno| errno.raw_os_error() == code)
    }) {
        return Err(Error::Refused(format!(
            "{}: {error}; lower --jobs or raise ulimit -n; no snapshot written",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resource_errors_abort_but_file_races_remain_skips() {
        for errno in [
            rustix::io::Errno::MFILE,
            rustix::io::Errno::NFILE,
            rustix::io::Errno::NOMEM,
        ] {
            let message = check(
                Path::new("source"),
                &io::Error::from_raw_os_error(errno.raw_os_error()),
            )
            .unwrap_err()
            .to_string();
            assert!(message.contains("lower --jobs or raise ulimit -n"));
        }
        for errno in [
            rustix::io::Errno::ACCESS,
            rustix::io::Errno::NOENT,
            rustix::io::Errno::LOOP,
            rustix::io::Errno::NOTDIR,
        ] {
            assert!(
                check(
                    Path::new("source"),
                    &io::Error::from_raw_os_error(errno.raw_os_error())
                )
                .is_ok()
            );
        }
    }
}
