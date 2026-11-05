//! Replace the index cache only after every pack header has been read successfully.
use crate::{
    backend::{Backend, Kind},
    crypto::envelope::Label,
    format::index::{Index, Pack},
    repo::{Error, Repository, audit, lock},
};
use serde::Serialize;
const LIMIT: usize = 16 * 1024 * 1024;
#[derive(Debug, Serialize)]
pub struct Summary {
    pub packs: usize,
    pub indexes_written: usize,
    pub indexes_removed: usize,
    pub dry_run: bool,
}
pub fn rebuild<B: Backend>(repo: &Repository<B>, dry_run: bool) -> Result<Summary, Error> {
    lock::with(&repo.backend, repo.subkeys(), true, |_| {
        let old = repo.backend.list(Kind::Index)?;
        let supersedes = old
            .iter()
            .map(|e| e.name.as_str()[6..].to_owned())
            .collect::<Vec<_>>();
        let mut batches = Vec::new();
        let mut current = Index {
            version: 1,
            supersedes: Vec::new(),
            packs: Vec::new(),
        };
        let mut size = serde_json::to_vec(&current)?.len();
        let packs = repo.backend.list(Kind::Data)?;
        for entry in &packs {
            let header = audit::header(&repo.backend, &repo.subkeys(), &entry.name)?;
            let pack = Pack {
                id: entry.name.as_str()[8..].into(),
                blobs: header.blobs,
            };
            let added = serde_json::to_vec(&pack)?.len() + usize::from(!current.packs.is_empty());
            if size + added > LIMIT && !current.packs.is_empty() {
                batches.push(current);
                current = Index {
                    version: 1,
                    supersedes: Vec::new(),
                    packs: Vec::new(),
                };
                size = serde_json::to_vec(&current)?.len();
            }
            size += added;
            current.packs.push(pack);
            if size > LIMIT {
                return Err(Error::Refused(
                    "pack header cannot fit a 16 MiB index payload".into(),
                ));
            }
        }
        // The final commit object supersedes ALL old names, including unreadable ones.
        // Earlier batches must remain live after it is published.
        current.supersedes = supersedes;
        if serde_json::to_vec(&current)?.len() > LIMIT {
            let supersedes = std::mem::take(&mut current.supersedes);
            batches.push(current);
            current = Index {
                version: 1,
                supersedes,
                packs: Vec::new(),
            };
        }
        if serde_json::to_vec(&current)?.len() > LIMIT {
            return Err(Error::Refused(
                "supersedes list exceeds 16 MiB index payload".into(),
            ));
        }
        batches.push(current);
        if !dry_run {
            for index in &batches {
                repo.write_object("index", Label::Index, &serde_json::to_vec(index)?)?;
            }
            for entry in &old {
                repo.backend.delete(&entry.name)?;
            }
        }
        Ok(Summary {
            packs: packs.len(),
            indexes_written: batches.len(),
            indexes_removed: old.len(),
            dry_run,
        })
    })
}
