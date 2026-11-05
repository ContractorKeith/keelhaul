use super::*;
use crate::format::tree::Node;
use std::{
    collections::{HashMap, HashSet},
    os::unix::fs::MetadataExt,
    path::PathBuf,
};
pub(in crate::ops::restore) struct Preflight<'a> {
    destination: &'a Destination,
    pub reused: HashSet<PathBuf>,
    pub failed: Vec<(PathBuf, Error)>,
    preserved: HashMap<PathBuf, (FileIdentity, Timespec)>,
}
impl Preflight<'_> {
    fn remember(&mut self, path: PathBuf, file: &File) -> Result<(), Error> {
        if !self.reused.insert(path.clone()) {
            return Ok(());
        }
        let m = file.metadata()?;
        self.preserved.insert(
            path,
            (
                FileIdentity::from_stat(&fs::fstat(file).map_err(os)?),
                Timespec {
                    tv_sec: m.mtime(),
                    tv_nsec: m.mtime_nsec(),
                },
            ),
        );
        Ok(())
    }
    pub fn restore_mtimes(&self) -> Vec<(PathBuf, Error)> {
        let mut failed = Vec::new();
        for (path, (identity, mtime)) in &self.preserved {
            let result = (|| {
                let file = self.destination.existing_directory(path)?;
                if FileIdentity::from_stat(&fs::fstat(&file).map_err(os)?) != *identity {
                    return Err(Error::Refused(format!(
                        "reused directory was replaced: {}",
                        path.display()
                    )));
                }
                let current = file.metadata()?;
                if current.mtime() == mtime.tv_sec && current.mtime_nsec() == mtime.tv_nsec {
                    return Ok(());
                }
                fs::futimens(
                    &file,
                    &Timestamps {
                        last_access: Timespec {
                            tv_sec: 0,
                            tv_nsec: fs::UTIME_OMIT,
                        },
                        last_modification: *mtime,
                    },
                )
                .map_err(os)?;
                Ok(())
            })();
            if let Err(error) = result {
                failed.push((path.clone(), error));
            }
        }
        failed
    }
}
impl Drop for Preflight<'_> {
    fn drop(&mut self) {
        let _ = self.restore_mtimes();
    }
}
impl Destination {
    pub fn preflight(
        &self,
        items: &[super::super::plan::Item],
        skipped: &HashSet<PathBuf>,
    ) -> Result<Preflight<'_>, Error> {
        let mut result = Preflight {
            destination: self,
            reused: HashSet::new(),
            failed: Vec::new(),
            preserved: HashMap::new(),
        };
        result.remember(PathBuf::new(), &self.root)?;
        for item in items {
            let inspected = (|| {
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
                                .map_err(|e| Error::Refused(format!("restore conflict: {e}")))?
                                .into();
                            result.remember(prefix.clone(), &current)?;
                        }
                        Ok(_) if !directory && skipped.contains(&item.path) => break,
                        Ok(_) => {
                            return Err(Error::Refused("restore conflict".into()));
                        }
                        Err(rustix::io::Errno::NOENT) => break,
                        Err(e) => return Err(os(e)),
                    }
                }
                Ok(())
            })();
            if let Err(error) = inspected {
                result.failed.push((item.path.clone(), error));
            }
        }
        Ok(result)
    }
}
