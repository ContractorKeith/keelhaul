//! Preflight every destination, then publish only completely verified files.
mod destination;
mod plan;
use crate::{
    backend::Backend,
    format::{pack::BlobKind, tree::Node},
    repo::{Error, Repository, blobs::BlobStore},
};
use serde::Serialize;
use std::{
    collections::HashMap,
    io::Write,
    path::{Path, PathBuf},
};
#[derive(Debug, Serialize)]
pub struct Damaged {
    pub object: String,
    pub problem: String,
}
#[derive(Debug, Default, Serialize)]
pub struct Report {
    pub restored: u64,
    pub damaged: Vec<Damaged>,
}
impl Report {
    pub(super) fn damage(&mut self, error: Error) -> Result<(), Error> {
        match error {
            Error::Damage {
                object,
                problem,
                detail,
            } => {
                eprintln!("damage: {object}: {problem}: {detail}");
                self.damaged.push(Damaged {
                    object,
                    problem: format!("{problem}: {detail}"),
                });
                Ok(())
            }
            other => Err(other),
        }
    }
}
pub fn restore<B: Backend>(
    repo: &Repository<B>,
    selector: &str,
    target: &Path,
    includes: &[PathBuf],
) -> Result<Report, Error> {
    restore_for_host(repo, selector, target, includes, None)
}
pub fn restore_for_host<B: Backend>(
    repo: &Repository<B>,
    selector: &str,
    target: &Path,
    includes: &[PathBuf],
    host: Option<&str>,
) -> Result<Report, Error> {
    let (_, snapshot) = repo.select_snapshot_for_host(selector, host)?;
    let mut store = BlobStore::load(repo)?;
    let mut report = Report::default();
    let items = plan::build(&mut store, &snapshot, includes, &mut report)?;
    let destination = destination::Destination::new(target)?;
    let preflight = destination.preflight(&items)?;
    let mut hardlinks: HashMap<(u64, u64), (PathBuf, u64, Vec<String>)> = HashMap::new();
    for item in &items {
        let path = &item.path;
        let outcome = (|| {
            match &item.entry.node {
                Node::Dir { .. } => {
                    if !preflight.reused.contains(path) {
                        destination.mkdir(path)?;
                    }
                }
                Node::Symlink { .. } => {
                    let raw = item
                        .entry
                        .target_bytes()
                        .map_err(|e| super::listing::damage(&item.object, &e.to_string()))?;
                    use std::os::unix::ffi::OsStringExt;
                    destination.symlink(path, &std::ffi::OsString::from_vec(raw), &item.entry)?;
                }
                Node::File { size, content } => {
                    item.entry
                        .validate_content_lengths(|id| {
                            store
                                .raw_length(BlobKind::Data, id)
                                .map_err(|e| crate::format::ParseError(e.to_string()))
                        })
                        .map_err(|e| super::listing::damage(&item.object, &e.to_string()))?;
                    let key = (item.entry.device, item.entry.inode);
                    if item.entry.links > 1
                        && let Some((first, first_size, first_content)) = hardlinks.get(&key)
                    {
                        if first_size != size || first_content != content {
                            return Err(super::listing::damage(
                                &item.object,
                                "inconsistent hardlink content",
                            ));
                        }
                        destination.hardlink(first, path)?;
                    } else {
                        let mut temp = destination.temporary(path)?;
                        for id in content {
                            let bytes = store.read(BlobKind::Data, id)?;
                            temp.file.write_all(&bytes)?;
                        }
                        temp.publish(&item.entry)?;
                        if item.entry.links > 1 {
                            hardlinks.insert(key, (path.clone(), *size, content.clone()));
                        }
                    }
                }
            }
            Ok(())
        })();
        match outcome {
            Ok(()) => report.restored += 1,
            Err(e) => report.damage(e)?,
        }
    }
    for item in items.iter().rev() {
        if matches!(item.entry.node, Node::Dir { .. }) && !preflight.reused.contains(&item.path) {
            destination.directory_metadata(&item.path, &item.entry)?;
        }
    }
    preflight.restore_mtimes()?;
    Ok(report)
}
