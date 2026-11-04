use super::*;
use crate::format::tree::Node;
use std::{
    collections::{HashMap, HashSet},
    os::unix::fs::MetadataExt,
    path::PathBuf,
};
pub(in crate::ops::restore) struct Preflight {
    pub reused: HashSet<PathBuf>,
    preserved: HashMap<PathBuf, (File, Timespec)>,
}
impl Preflight {
    fn remember(&mut self, path: PathBuf, file: &File) -> Result<(), Error> {
        if !self.reused.insert(path.clone()) {
            return Ok(());
        }
        let m = file.metadata()?;
        self.preserved.insert(
            path,
            (
                file.try_clone()?,
                Timespec {
                    tv_sec: m.mtime(),
                    tv_nsec: m.mtime_nsec(),
                },
            ),
        );
        Ok(())
    }
    pub fn restore_mtimes(&self) -> Result<(), Error> {
        for (file, mtime) in self.preserved.values() {
            let current = file.metadata()?;
            if current.mtime() == mtime.tv_sec && current.mtime_nsec() == mtime.tv_nsec {
                continue;
            }
            fs::futimens(
                file,
                &Timestamps {
                    last_access: Timespec {
                        tv_sec: 0,
                        tv_nsec: fs::UTIME_OMIT,
                    },
                    last_modification: *mtime,
                },
            )
            .map_err(os)?;
        }
        Ok(())
    }
}
impl Drop for Preflight {
    fn drop(&mut self) {
        let _ = self.restore_mtimes();
    }
}
impl Destination {
    pub fn preflight(&self, items: &[super::super::plan::Item]) -> Result<Preflight, Error> {
        let mut result = Preflight {
            reused: HashSet::new(),
            preserved: HashMap::new(),
        };
        result.remember(PathBuf::new(), &self.root)?;
        for item in items {
            let mut current = self.root.try_clone()?;
            let mut prefix = PathBuf::new();
            let components = item.path.components().collect::<Vec<_>>();
            for (i, component) in components.iter().enumerate() {
                let Component::Normal(name) = component else {
                    return Err(Error::Refused("unsafe restore component".into()));
                };
                prefix.push(name);
                let directory =
                    i + 1 < components.len() || matches!(item.entry.node, Node::Dir { .. });
                match fs::statat(&current, *name, AtFlags::SYMLINK_NOFOLLOW) {
                    Ok(stat)
                        if directory
                            && fs::FileType::from_raw_mode(stat.st_mode)
                                == fs::FileType::Directory =>
                    {
                        current = fs::openat(&current, *name, DIRECTORY, Mode::empty())
                            .map_err(|e| {
                                Error::Refused(format!(
                                    "restore conflict: {}: {e}",
                                    prefix.display()
                                ))
                            })?
                            .into();
                        result.remember(prefix.clone(), &current)?;
                    }
                    Ok(_) => {
                        return Err(Error::Refused(format!(
                            "restore conflict: {}",
                            prefix.display()
                        )));
                    }
                    Err(rustix::io::Errno::NOENT) => break,
                    Err(e) => return Err(os(e)),
                }
            }
        }
        Ok(result)
    }
}
