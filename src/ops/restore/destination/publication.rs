use super::*;
use rustix::io::Errno;
/// A hard link publishes the already verified inode atomically without replacing
/// an existing name, on filesystems that lack the no-replace rename operation.
pub(super) fn finish(
    renamed: Result<(), Errno>,
    name: &std::ffi::OsStr,
    link: impl FnOnce() -> Result<(), Errno>,
    unlink: impl FnOnce() -> Result<(), Errno>,
) -> Result<(), Error> {
    match renamed {
        Ok(()) => Ok(()),
        Err(Errno::INVAL | Errno::NOSYS | Errno::OPNOTSUPP) => match link() {
            Ok(()) => unlink().map_err(os),
            Err(Errno::EXIST) => Err(Error::Refused(format!(
                "restore conflict: {}",
                name.to_string_lossy()
            ))),
            Err(e @ (Errno::INVAL | Errno::NOSYS | Errno::OPNOTSUPP | Errno::PERM)) => {
                Err(Error::Refused(format!(
                    "filesystem cannot safely publish {}: no-replace rename and hard links are unsupported ({e})",
                    name.to_string_lossy()
                )))
            }
            Err(e) => Err(os(e)),
        },
        Err(Errno::EXIST) => Err(Error::Refused(format!(
            "restore conflict: {}",
            name.to_string_lossy()
        ))),
        Err(e) => Err(os(e)),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unavailable_rename_falls_back_atomically_and_preserves_conflicts() {
        for errno in [Errno::INVAL, Errno::NOSYS] {
            let scratch = tempfile::tempdir().unwrap();
            let parent = File::open(scratch.path()).unwrap();
            std::fs::write(scratch.path().join("temp"), b"verified").unwrap();
            finish(
                Err(errno),
                std::ffi::OsStr::new("final"),
                || fs::linkat(&parent, "temp", &parent, "final", AtFlags::empty()),
                || fs::unlinkat(&parent, "temp", AtFlags::empty()),
            )
            .unwrap();
            assert_eq!(
                std::fs::read(scratch.path().join("final")).unwrap(),
                b"verified"
            );
            assert!(!scratch.path().join("temp").exists());
            std::fs::write(scratch.path().join("temp"), b"another").unwrap();
            assert!(
                finish(
                    Err(errno),
                    std::ffi::OsStr::new("final"),
                    || fs::linkat(&parent, "temp", &parent, "final", AtFlags::empty()),
                    || panic!("must retain temp on conflict")
                )
                .is_err()
            );
            assert_eq!(
                std::fs::read(scratch.path().join("final")).unwrap(),
                b"verified"
            );
        }
    }
    #[test]
    fn unsupported_link_reports_filesystem_limitation_and_other_errors_do_not_fallback() {
        let error = finish(
            Err(Errno::NOSYS),
            std::ffi::OsStr::new("file"),
            || Err(Errno::OPNOTSUPP),
            || panic!("no link published"),
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("filesystem") && error.to_string().contains("unsupported")
        );
        assert!(
            finish(
                Err(Errno::ACCESS),
                std::ffi::OsStr::new("file"),
                || panic!("do not mask access failure"),
                || panic!()
            )
            .is_err()
        );
    }
}
