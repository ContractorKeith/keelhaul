use super::{Skipped, Summary};
use crate::{
    backend::Backend,
    format::{
        pack::BlobKind,
        tree::{Entry, Node, Tree, encode_bytes},
    },
    ops::metadata,
    repo::{Error, blobs::BlobStore},
};
use std::{
    fs,
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
};
mod files;
pub(super) struct Walker<'a, B> {
    pub store: BlobStore<'a, B>,
    pub summary: Summary,
    pub skipped: Vec<Skipped>,
    pub gear: [u8; 32],
}
enum Work {
    Visit(PathBuf, Vec<u8>),
    Finish {
        name: Vec<u8>,
        meta: fs::Metadata,
        children: usize,
    },
}
impl<B: Backend> Walker<'_, B> {
    fn skip(&mut self, path: &Path, reason: impl ToString) {
        let reason = reason.to_string();
        eprintln!("warning: {}: {reason}", path.display());
        self.summary.files_skipped += 1;
        self.skipped.push(Skipped {
            path: path.to_string_lossy().into_owned(),
            reason,
        });
    }
    pub fn walk(&mut self, path: &Path, name: &[u8]) -> Result<Option<Entry>, Error> {
        let mut work = vec![Work::Visit(path.into(), name.into())];
        let mut results: Vec<Option<Entry>> = Vec::new();
        while let Some(task) = work.pop() {
            match task {
                Work::Finish {
                    name,
                    meta,
                    children,
                } => {
                    let entries = results
                        .split_off(results.len() - children)
                        .into_iter()
                        .flatten()
                        .collect();
                    let subtree = self.store.add(
                        BlobKind::Tree,
                        &serde_json::to_vec(&Tree {
                            version: 1,
                            entries,
                        })?,
                    )?;
                    self.summary.dirs += 1;
                    results.push(Some(metadata::entry(&name, &meta, Node::Dir { subtree })?));
                }
                Work::Visit(path, name) => {
                    let meta = match fs::symlink_metadata(&path) {
                        Ok(m) => m,
                        Err(e) => {
                            self.skip(&path, e);
                            results.push(None);
                            continue;
                        }
                    };
                    if meta.is_dir() {
                        let children = match fs::read_dir(&path) {
                            Ok(c) => c,
                            Err(e) => {
                                self.skip(&path, e);
                                results.push(None);
                                continue;
                            }
                        };
                        let mut paths = Vec::new();
                        for child in children {
                            match child {
                                Ok(c) => paths.push(c.path()),
                                Err(e) => self.skip(&path, e),
                            }
                        }
                        paths.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
                        work.push(Work::Finish {
                            name,
                            meta,
                            children: paths.len(),
                        });
                        for child in paths.into_iter().rev() {
                            let name = child
                                .file_name()
                                .expect("read_dir children have names")
                                .as_bytes()
                                .to_vec();
                            work.push(Work::Visit(child, name));
                        }
                        continue;
                    }
                    let node = if meta.is_symlink() {
                        match fs::read_link(&path) {
                            Ok(t) => {
                                let (target, target_b64) = encode_bytes(t.as_os_str().as_bytes());
                                Some(Node::Symlink { target, target_b64 })
                            }
                            Err(e) => {
                                self.skip(&path, e);
                                None
                            }
                        }
                    } else if meta.is_file() {
                        self.file(&path, &meta)?
                    } else {
                        self.skip(&path, "unsupported file type");
                        None
                    };
                    results.push(node.map(|n| metadata::entry(&name, &meta, n)).transpose()?);
                }
            }
        }
        Ok(results.pop().flatten())
    }
}
