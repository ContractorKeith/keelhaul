//! Descriptor-relative writes: every ancestor is opened with O_NOFOLLOW.
mod preflight;
mod publication;
use crate::{format::tree::Entry, repo::Error};
pub(super) use publication::Temporary;
use rustix::fs::{self, AtFlags, Gid, Mode, OFlags, Timespec, Timestamps, Uid};
use std::{
    ffi::OsString,
    fs::File,
    io,
    path::{Component, Path},
};
pub(super) struct Destination {
    root: File,
    created: std::cell::RefCell<std::collections::HashMap<(FileIdentity, FileIdentity), OsString>>,
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
        let root = match fs::open(target, DIRECTORY, Mode::empty()) {
            Ok(fd) => fd,
            Err(rustix::io::Errno::NOENT) => {
                std::fs::create_dir_all(target)?;
                fs::open(target, DIRECTORY, Mode::empty()).map_err(os)?
            }
            Err(error) => return Err(os(error)),
        }
        .into();
        Ok(Self {
            root,
            created: Default::default(),
        })
    }
    fn directory(&self, path: &Path) -> Result<File, Error> {
        self.walk_directory(path, true)
    }
    fn existing_directory(&self, path: &Path) -> Result<File, Error> {
        self.walk_directory(path, false)
    }
    fn walk_directory(&self, path: &Path, create: bool) -> Result<File, Error> {
        let mut current = self.root.try_clone()?;
        for component in path.components() {
            let Component::Normal(name) = component else {
                return Err(Error::Refused("unsafe restore component".into()));
            };
            self.check_name(&current, name)?;
            match fs::openat(&current, name, DIRECTORY, Mode::empty()) {
                Ok(fd) => current = fd.into(),
                Err(rustix::io::Errno::NOENT) if create => {
                    fs::mkdirat(&current, name, Mode::from_raw_mode(0o700)).map_err(os)?;
                    let next = fs::openat(&current, name, DIRECTORY, Mode::empty()).map_err(os)?;
                    self.record_name(&current, name)?;
                    current = next.into();
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
    // Compare filesystem identities, rather than assuming a particular case or
    // Unicode folding rule. Names are scoped to the pinned parent directory.
    pub fn check_collision(&self, path: &Path) -> Result<(), Error> {
        if path.as_os_str().is_empty() {
            return Ok(());
        }
        let (parent, name) = self.parent(path)?;
        self.check_name(&parent, &name)
    }
    fn check_name(&self, parent: &File, name: &std::ffi::OsStr) -> Result<(), Error> {
        let parent_id = FileIdentity::from_stat(&fs::fstat(parent).map_err(os)?);
        match fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(stat) => {
                let id = FileIdentity::from_stat(&stat);
                if let Some(other) = self.created.borrow().get(&(parent_id, id))
                    && other != name
                {
                    return Err(Error::Refused(format!(
                        "name collides with {} on this filesystem",
                        other.to_string_lossy()
                    )));
                }
                Ok(())
            }
            Err(rustix::io::Errno::NOENT) => Ok(()),
            Err(error) => Err(os(error)),
        }
    }
    fn record_name(&self, parent: &File, name: &std::ffi::OsStr) -> Result<(), Error> {
        let directory = FileIdentity::from_stat(&fs::fstat(parent).map_err(os)?);
        let identity = FileIdentity::from_stat(
            &fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW).map_err(os)?,
        );
        self.created
            .borrow_mut()
            .insert((directory, identity), name.into());
        Ok(())
    }
    pub fn remember(&self, path: &Path) -> Result<(), Error> {
        if path.as_os_str().is_empty() {
            return Ok(());
        }
        let (parent, name) = self.parent(path)?;
        self.record_name(&parent, &name)
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
    pub fn hardlink(
        &self,
        first: &Path,
        path: &Path,
        expected: FileIdentity,
    ) -> Result<bool, Error> {
        let Some(parent_path) = first.parent() else {
            return Ok(false);
        };
        let Some(old_name) = first.file_name() else {
            return Ok(false);
        };
        let old_parent = match self.existing_directory(parent_path) {
            Ok(parent) => parent,
            Err(_) => return Ok(false),
        };
        let stat = match fs::statat(&old_parent, old_name, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(stat) => stat,
            Err(_) => return Ok(false),
        };
        if fs::FileType::from_raw_mode(stat.st_mode) != fs::FileType::RegularFile
            || FileIdentity::from_stat(&stat) != expected
        {
            return Ok(false);
        }
        let (parent, name) = self.parent(path)?;
        // A writer with access to the target can still replace the source between
        // this identity check and linkat, causing the link to use their bytes.
        fs::linkat(&old_parent, old_name, &parent, &name, AtFlags::empty()).map_err(os)?;
        Ok(true)
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct FileIdentity {
    device: fs::Dev,
    inode: u64,
}
impl FileIdentity {
    fn from_stat(stat: &fs::Stat) -> Self {
        Self {
            device: stat.st_dev,
            inode: stat.st_ino,
        }
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
mod tests;
