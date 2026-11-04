//! Descriptor-relative writes: every ancestor is opened with O_NOFOLLOW.
use crate::{format::tree::Entry, repo::Error};
use rustix::fs::{self, AtFlags, Gid, Mode, OFlags, RenameFlags, Timespec, Timestamps, Uid};
use std::{
    ffi::OsString,
    fs::File,
    io,
    path::{Component, Path},
};
pub(super) struct Destination {
    root: File,
}
const DIRECTORY: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);
fn os(error: rustix::io::Errno) -> Error {
    io::Error::from(error).into()
}
impl Destination {
    pub fn new(target: &Path) -> Result<Self, Error> {
        std::fs::create_dir_all(target)?;
        let canonical = std::fs::canonicalize(target)?;
        let root = fs::open(&canonical, DIRECTORY, Mode::empty())
            .map_err(os)?
            .into();
        Ok(Self { root })
    }
    fn directory(&self, path: &Path) -> Result<File, Error> {
        let mut current = self.root.try_clone()?;
        for component in path.components() {
            let Component::Normal(name) = component else {
                return Err(Error::Refused("unsafe restore component".into()));
            };
            match fs::openat(&current, name, DIRECTORY, Mode::empty()) {
                Ok(fd) => current = fd.into(),
                Err(rustix::io::Errno::NOENT) => {
                    fs::mkdirat(&current, name, Mode::from_raw_mode(0o700)).map_err(os)?;
                    current = fs::openat(&current, name, DIRECTORY, Mode::empty())
                        .map_err(os)?
                        .into();
                }
                Err(e) => return Err(os(e)),
            }
        }
        Ok(current)
    }
    fn parent(&self, path: &Path) -> Result<(File, OsString), Error> {
        let parent = path
            .parent()
            .ok_or_else(|| Error::Refused("restore path has no parent".into()))?;
        let name = path
            .file_name()
            .ok_or_else(|| Error::Refused("restore path has no name".into()))?;
        Ok((self.directory(parent)?, name.into()))
    }
    pub fn mkdir(&self, path: &Path) -> Result<(), Error> {
        if path.as_os_str().is_empty() {
            return Ok(());
        }
        let (parent, name) = self.parent(path)?;
        fs::mkdirat(&parent, &name, Mode::from_raw_mode(0o700)).map_err(os)
    }
    pub fn symlink(
        &self,
        path: &Path,
        target: &std::ffi::OsStr,
        entry: &Entry,
    ) -> Result<(), Error> {
        let (parent, name) = self.parent(path)?;
        fs::symlinkat(target, &parent, &name).map_err(os)?;
        if uzers::get_effective_uid() == 0 {
            let (uid, gid) = owners(entry)?;
            fs::chownat(
                &parent,
                &name,
                Some(uid),
                Some(gid),
                AtFlags::SYMLINK_NOFOLLOW,
            )
            .map_err(os)?;
        }
        #[cfg(target_os = "macos")]
        fs::chmodat(
            &parent,
            &name,
            Mode::from_raw_mode(entry.mode as fs::RawMode),
            AtFlags::SYMLINK_NOFOLLOW,
        )
        .map_err(os)?;
        fs::utimensat(&parent, &name, &times(entry)?, AtFlags::SYMLINK_NOFOLLOW).map_err(os)
    }
    pub fn hardlink(&self, first: &Path, path: &Path) -> Result<(), Error> {
        let (old_parent, old_name) = self.parent(first)?;
        let (parent, name) = self.parent(path)?;
        fs::linkat(&old_parent, &old_name, &parent, &name, AtFlags::empty()).map_err(os)
    }
    pub fn temporary(&self, path: &Path) -> Result<Temporary, Error> {
        let (parent, final_name) = self.parent(path)?;
        let name = OsString::from(format!(
            ".keelhaul-{}",
            hex::encode(crate::crypto::keys::random::<16>()?)
        ));
        let file = fs::openat(
            &parent,
            &name,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o600),
        )
        .map_err(os)?
        .into();
        Ok(Temporary {
            parent,
            name,
            final_name,
            file,
        })
    }
    pub fn directory_metadata(&self, path: &Path, entry: &Entry) -> Result<(), Error> {
        apply(&self.directory(path)?, entry)
    }
}
pub(super) struct Temporary {
    parent: File,
    name: OsString,
    final_name: OsString,
    pub file: File,
}
impl Temporary {
    pub fn publish(&self, entry: &Entry) -> Result<(), Error> {
        self.file.sync_all()?;
        fs::renameat_with(
            &self.parent,
            &self.name,
            &self.parent,
            &self.final_name,
            RenameFlags::NOREPLACE,
        )
        .map_err(os)?;
        apply(&self.file, entry)?;
        self.file.sync_all()?;
        self.parent.sync_all()?;
        Ok(())
    }
}
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = fs::unlinkat(&self.parent, &self.name, AtFlags::empty());
    }
}
fn owners(entry: &Entry) -> Result<(Uid, Gid), Error> {
    if entry.uid == u32::MAX || entry.gid == u32::MAX {
        return Err(Error::Refused("cannot restore sentinel owner ID".into()));
    }
    Ok((Uid::from_raw(entry.uid), Gid::from_raw(entry.gid)))
}
fn times(entry: &Entry) -> Result<Timestamps, Error> {
    let t =
        time::OffsetDateTime::parse(&entry.mtime, &time::format_description::well_known::Rfc3339)
            .map_err(|e| Error::Refused(e.to_string()))?;
    let stamp = Timespec {
        tv_sec: t.unix_timestamp(),
        tv_nsec: t.nanosecond().into(),
    };
    Ok(Timestamps {
        last_access: stamp,
        last_modification: stamp,
    })
}
fn apply(file: &File, entry: &Entry) -> Result<(), Error> {
    if uzers::get_effective_uid() == 0 {
        let (uid, gid) = owners(entry)?;
        fs::fchown(file, Some(uid), Some(gid)).map_err(os)?;
    }
    fs::fchmod(file, Mode::from_raw_mode(entry.mode as fs::RawMode)).map_err(os)?;
    fs::futimens(file, &times(entry)?).map_err(os)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replaced_directory_symlink_is_never_traversed() {
        let scratch = tempfile::tempdir().unwrap();
        let target = scratch.path().join("target");
        let outside = scratch.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        let destination = Destination::new(&target).unwrap();
        destination.mkdir(Path::new("child")).unwrap();
        std::fs::remove_dir(target.join("child")).unwrap();
        std::os::unix::fs::symlink(&outside, target.join("child")).unwrap();
        assert!(destination.temporary(Path::new("child/escape")).is_err());
        assert!(std::fs::read_dir(outside).unwrap().next().is_none());
    }
    #[test]
    fn publication_refuses_a_conflict_created_after_preflight() {
        let scratch = tempfile::tempdir().unwrap();
        let destination = Destination::new(scratch.path()).unwrap();
        let temporary = destination.temporary(Path::new("file")).unwrap();
        std::fs::write(scratch.path().join("file"), b"user bytes").unwrap();
        let entry: Entry = serde_json::from_value(serde_json::json!({
            "name":"file","type":"file","mode":420,
            "mtime":"2026-11-04T14:03:22.123456789Z", "ctime":"2026-11-04T14:03:22.123456789Z",
            "uid":0,"gid":0,"inode":1,"device":1,"links":1,"size":0,"content":[]
        }))
        .unwrap();
        assert!(temporary.publish(&entry).is_err());
        drop(temporary);
        assert_eq!(
            std::fs::read(scratch.path().join("file")).unwrap(),
            b"user bytes"
        );
        assert_eq!(std::fs::read_dir(scratch.path()).unwrap().count(), 1);
    }
}
