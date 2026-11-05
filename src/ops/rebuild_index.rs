//! Rebuild readable headers, preserving indexed locations for damaged headers.
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
    pub damage: Vec<crate::damage::Damage>,
}
pub fn rebuild<B: Backend>(repo: &Repository<B>, dry_run: bool) -> Result<Summary, Error> {
    lock::with(&repo.backend, repo.subkeys(), true, |_| {
        let old = repo.backend.list(Kind::Index)?;
        let supersedes = old
            .iter()
            .map(|e| e.name.as_str()[6..].to_owned())
            .collect::<Vec<_>>();
        let mut damage = crate::damage::Report::default();
        let mut cached = std::collections::HashMap::new();
        for entry in &old {
            let bytes = match repo.read_object(&entry.name, Label::Index) {
                Ok(bytes) => bytes,
                Err(Error::Damage { .. }) => continue,
                Err(error) => return Err(error),
            };
            if let Ok(index) = Index::parse(&bytes) {
                for pack in index.packs {
                    cached.entry(pack.id).or_insert(pack.blobs);
                }
            }
        }
        let mut indexed = 0;
        let mut batches = Vec::new();
        let mut current = Index {
            version: 1,
            supersedes: Vec::new(),
            packs: Vec::new(),
        };
        let mut size = serde_json::to_vec(&current)?.len();
        let packs = repo.backend.list(Kind::Data)?;
        for entry in &packs {
            let id = entry.name.as_str()[8..].to_owned();
            let blobs =
                match damage.collect(audit::header(&repo.backend, &repo.subkeys(), &entry.name))? {
                    Some(header) => header.blobs,
                    None => match cached.remove(&id) {
                        Some(blobs) => blobs,
                        None => continue,
                    },
                };
            indexed += 1;
            let pack = Pack { id, blobs };
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
            packs: indexed,
            damage: damage.damage,
            indexes_written: batches.len(),
            indexes_removed: old.len(),
            dry_run,
        })
    })
}
