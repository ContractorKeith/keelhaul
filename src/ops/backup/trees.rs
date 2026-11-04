//! Complete directories as their children arrive; completion order never changes JSON.
use crate::{
    backend::Backend,
    format::{
        pack::BlobKind,
        tree::{Entry, Node, Tree},
    },
    repo::{Error, blobs::BlobStore},
};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
};
type Key = (usize, PathBuf);
struct Directory {
    entry: Entry,
    remaining: usize,
    children: Vec<Entry>,
}
pub(super) struct Trees {
    pending: HashMap<Key, Directory>,
    roots: HashSet<Key>,
    saved: HashSet<usize>,
    entries: Vec<Entry>,
}
impl Trees {
    pub fn new(roots: &[(PathBuf, &PathBuf)]) -> Self {
        Self {
            pending: HashMap::new(),
            roots: roots
                .iter()
                .enumerate()
                .map(|(n, (p, _))| (n, p.clone()))
                .collect(),
            saved: HashSet::new(),
            entries: Vec::new(),
        }
    }
    pub fn directory<B: Backend>(
        &mut self,
        key: Key,
        mut entry: Entry,
        count: usize,
        store: &mut BlobStore<'_, B>,
    ) -> Result<(), Error> {
        if count == 0 {
            entry.node = Node::Dir {
                subtree: tree(store, Vec::new())?,
            };
            self.complete(key, Some(entry), store)
        } else {
            self.pending.insert(
                key,
                Directory {
                    entry,
                    remaining: count,
                    children: Vec::new(),
                },
            );
            Ok(())
        }
    }
    pub fn complete<B: Backend>(
        &mut self,
        mut key: Key,
        mut entry: Option<Entry>,
        store: &mut BlobStore<'_, B>,
    ) -> Result<(), Error> {
        loop {
            if self.roots.contains(&key) {
                if let Some(entry) = entry {
                    self.saved.insert(key.0);
                    self.entries.push(entry);
                }
                return Ok(());
            }
            let parent = (
                key.0,
                key.1
                    .parent()
                    .ok_or_else(super::pipeline::closed)?
                    .to_path_buf(),
            );
            let directory = self
                .pending
                .get_mut(&parent)
                .ok_or_else(super::pipeline::closed)?;
            directory.remaining = directory
                .remaining
                .checked_sub(1)
                .ok_or_else(super::pipeline::closed)?;
            if let Some(entry) = entry {
                directory.children.push(entry);
            }
            if directory.remaining != 0 {
                return Ok(());
            }
            let mut directory = self
                .pending
                .remove(&parent)
                .expect("directory was just found and completed");
            directory.entry.node = Node::Dir {
                subtree: tree(store, directory.children)?,
            };
            key = parent;
            entry = Some(directory.entry);
        }
    }
    pub fn finish<B: Backend>(
        self,
        strings: &[String],
        store: &mut BlobStore<'_, B>,
    ) -> Result<(String, Vec<String>), Error> {
        if !self.pending.is_empty() {
            return Err(super::pipeline::closed());
        }
        let paths = strings
            .iter()
            .enumerate()
            .filter(|(n, _)| self.saved.contains(n))
            .map(|(_, s)| s.clone())
            .collect::<Vec<_>>();
        if paths.is_empty() {
            return Err(Error::Refused(
                "no backup roots could be read; no snapshot written".into(),
            ));
        }
        Ok((tree(store, self.entries)?, paths))
    }
}
fn tree<B: Backend>(store: &mut BlobStore<'_, B>, entries: Vec<Entry>) -> Result<String, Error> {
    let mut named = entries
        .into_iter()
        .map(|entry| {
            Ok((
                entry
                    .name_bytes()
                    .map_err(|e| Error::Refused(e.to_string()))?,
                entry,
            ))
        })
        .collect::<Result<Vec<_>, Error>>()?;
    named.sort_by(|a, b| a.0.cmp(&b.0));
    store.add(
        BlobKind::Tree,
        &serde_json::to_vec(&Tree {
            version: 1,
            entries: named.into_iter().map(|(_, e)| e).collect(),
        })?,
    )
}
