use super::*;
use crate::ops::restore::plan::Item;
use std::{
    collections::{HashMap, HashSet},
    os::unix::ffi::OsStrExt,
    path::PathBuf,
};

impl Destination {
    pub fn existing_file(&self, path: &Path) -> Result<Option<File>, Error> {
        let Some(parent) = path.parent() else {
            return Ok(None);
        };
        let parent = match self.existing_directory(parent) {
            Ok(parent) => parent,
            Err(_) => return Ok(None), // Preflight reports inaccessible ancestors.
        };
        let Some(name) = path.file_name() else {
            return Ok(None);
        };
        match fs::statat(&parent, name, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(stat) if fs::FileType::from_raw_mode(stat.st_mode) == fs::FileType::RegularFile => {}
            Ok(_) | Err(rustix::io::Errno::NOENT) => return Ok(None),
            Err(error) => return Err(os(error)),
        }
        // NONBLOCK also prevents a raced replacement with a FIFO from hanging.
        let file: File = fs::openat(
            &parent,
            name,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(os)?
        .into();
        Ok(file.metadata()?.is_file().then_some(file))
    }

    pub fn cleanup_temps(&self, items: &[Item]) -> Result<(), Error> {
        let mut directories: HashMap<PathBuf, HashSet<OsString>> = HashMap::new();
        for item in items {
            if let Some(parent) = item.path.parent() {
                directories
                    .entry(parent.into())
                    .or_default()
                    .insert(item.path.file_name().unwrap_or_default().into());
                for ancestor in parent.ancestors() {
                    directories.entry(ancestor.into()).or_default();
                }
            }
            if matches!(item.entry.node, crate::format::tree::Node::Dir { .. }) {
                directories.entry(item.path.clone()).or_default();
            }
        }
        for (path, protected) in directories {
            let Ok(parent) = self.existing_directory(&path) else {
                continue;
            };
            for entry in fs::Dir::read_from(&parent).map_err(os)? {
                let entry = entry.map_err(os)?;
                let name = std::ffi::OsStr::from_bytes(entry.file_name().to_bytes());
                let Some(hex) = name.as_bytes().strip_prefix(b".keelhaul-") else {
                    continue;
                };
                if hex.len() != 32
                    || !hex
                        .iter()
                        .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(c))
                    || protected.contains(name)
                {
                    continue;
                }
                let stat = fs::statat(&parent, name, AtFlags::SYMLINK_NOFOLLOW).map_err(os)?;
                if fs::FileType::from_raw_mode(stat.st_mode) == fs::FileType::RegularFile {
                    fs::unlinkat(&parent, name, AtFlags::empty()).map_err(os)?;
                }
            }
        }
        Ok(())
    }
}
