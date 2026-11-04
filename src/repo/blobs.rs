//! Pack assembler and durable index flushes.
mod parent;
mod writer;
pub use writer::{SealedBlob, Upload};
mod reader;
use super::{Error, Repository, objects::missing, open::damage};
use crate::{
    backend::{Backend, Kind, ObjectName},
    crypto::envelope::{self, Label},
    format::{
        index::{Index, Pack},
        pack::{Blob, BlobKind, Header, MAX_HEADER},
    },
};
use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};

#[derive(Clone)]
pub struct Location {
    pub pack: String,
    pub blob: Blob,
}
pub struct BlobStore<'a, B> {
    repo: &'a Repository<B>,
    uploads: Option<writer::Uploads>,
    pub index: HashMap<(BlobKind, String), Location>,
    pub(crate) suspect: HashSet<(BlobKind, String)>,
    parent_checked: HashSet<String>,
    pending: Vec<u8>,
    blobs: Vec<Blob>,
    pending_ids: HashSet<(BlobKind, String)>,
    header_bytes: u64,
    index_bytes: u64,
    packs: Vec<Pack>,
    flushed: Instant,
    verified: HashSet<String>,
    headers: HashMap<String, (String, Vec<Blob>)>,
    pub data_blobs_added: u64,
    pub bytes_added: u64,
}
impl<'a, B: Backend> BlobStore<'a, B> {
    pub fn load(repo: &'a Repository<B>) -> Result<Self, Error> {
        let mut indexes = Vec::new();
        let mut superseded = HashSet::new();
        for entry in repo.backend.list(Kind::Index)? {
            let bytes = repo.read_object(&entry.name, Label::Index)?;
            let index = Index::parse(&bytes)
                .map_err(|e| damage(entry.name.as_str(), "bad-format", &e.to_string()))?;
            superseded.extend(index.supersedes.iter().cloned());
            indexes.push((entry.name.as_str()[6..].to_owned(), index));
        }
        let mut index = HashMap::new();
        let mut headers = HashMap::new();
        for (id, value) in indexes {
            if superseded.contains(&id) {
                continue;
            }
            for pack in value.packs {
                if let Some((_, previous)) = headers.get(&pack.id)
                    && previous != &pack.blobs
                {
                    return Err(damage(
                        &format!("index/{id}"),
                        "index-mismatch",
                        "indexes disagree on pack header",
                    ));
                }
                headers.insert(pack.id.clone(), (format!("index/{id}"), pack.blobs.clone()));
                for blob in pack.blobs {
                    index
                        .entry((blob.kind, blob.id.clone()))
                        .or_insert(Location {
                            pack: pack.id.clone(),
                            blob,
                        });
                }
            }
        }
        Ok(Self {
            repo,
            uploads: None,
            index,
            suspect: HashSet::new(),
            parent_checked: HashSet::new(),
            pending: Vec::new(),
            blobs: Vec::new(),
            pending_ids: HashSet::new(),
            header_bytes: 32,
            index_bytes: 0,
            packs: Vec::new(),
            flushed: Instant::now(),
            verified: HashSet::new(),
            headers,
            data_blobs_added: 0,
            bytes_added: 0,
        })
    }
    pub fn add(&mut self, kind: BlobKind, payload: &[u8]) -> Result<String, Error> {
        let keys = self.repo.master.subkeys();
        let id = blake3::keyed_hash(&keys.chunk_id, payload)
            .to_hex()
            .to_string();
        if self.index.contains_key(&(kind, id.clone()))
            || self.pending_ids.contains(&(kind, id.clone()))
        {
            return Ok(id);
        }
        let stored = envelope::seal(&keys.encryption, kind.label(), payload)?;
        self.add_sealed(SealedBlob {
            id: id.clone(),
            kind,
            stored,
            raw_length: payload.len() as u64,
        })?;
        Ok(id)
    }
    pub fn add_sealed(&mut self, sealed: SealedBlob) -> Result<(), Error> {
        let SealedBlob {
            id,
            kind,
            stored,
            raw_length,
        } = sealed;
        let mut blob = Blob {
            id: id.clone(),
            kind,
            offset: self.pending.len() as u64,
            length: stored.len() as u64,
            raw_length,
        };
        let descriptor_bytes = serde_json::to_vec(&blob)?.len() as u64 + 1;
        // The raw header bounds the sealed header too, since seal only shrinks it.
        if self.header_bytes + descriptor_bytes > MAX_HEADER - 41 {
            self.close_pack()?;
            blob.offset = 0;
        }
        self.header_bytes += descriptor_bytes;
        self.suspect.remove(&(kind, id.clone()));
        self.pending_ids.insert((kind, id));
        if kind == BlobKind::Data {
            self.data_blobs_added += 1;
        }
        self.bytes_added += stored.len() as u64;
        self.pending.extend(stored);
        self.blobs.push(blob);
        if self.pending.len() as u64 >= self.repo.config.pack_target {
            self.close_pack()?;
        }
        if self.flushed.elapsed() >= Duration::from_secs(300) {
            self.close_pack()?;
            self.flush_index()?;
        }
        Ok(())
    }
    fn close_pack(&mut self) -> Result<(), Error> {
        if self.blobs.is_empty() {
            return Ok(());
        }
        let header = Header {
            version: 1,
            blobs: std::mem::take(&mut self.blobs),
        };
        let sealed = envelope::seal(
            &self.repo.master.subkeys().encryption,
            Label::PackHeader,
            &serde_json::to_vec(&header)?,
        )?;
        if sealed.len() as u64 > MAX_HEADER {
            return Err(Error::Refused("pack header exceeds 16 MiB".into()));
        }
        self.pending.extend(&sealed);
        self.pending.extend((sealed.len() as u32).to_le_bytes());
        let id = blake3::hash(&self.pending).to_hex().to_string();
        let bytes = std::mem::take(&mut self.pending);
        if let Some(uploads) = &mut self.uploads {
            uploads.send(Upload {
                name: pack_name(&id)?,
                bytes,
            })?;
        } else {
            self.repo.backend.write(&pack_name(&id)?, &bytes)?;
        }
        for blob in &header.blobs {
            self.index.insert(
                (blob.kind, blob.id.clone()),
                Location {
                    pack: id.clone(),
                    blob: blob.clone(),
                },
            );
        }
        self.headers.insert(
            id.clone(),
            (format!("data/{}/{id}", &id[..2]), header.blobs.clone()),
        );
        // Keep one index payload below the envelope's 64 MiB decompression cap.
        if self.index_bytes + self.header_bytes > 32 * 1024 * 1024 {
            self.flush_index()?;
        }
        self.index_bytes += self.header_bytes;
        self.header_bytes = 32;
        self.pending_ids.clear();
        self.packs.push(Pack {
            id,
            blobs: header.blobs,
        });
        if self.packs.len() >= 8 {
            self.flush_index()?;
        }
        Ok(())
    }
    fn flush_index(&mut self) -> Result<(), Error> {
        if let Some(uploads) = &mut self.uploads {
            uploads.wait()?;
        }
        if self.packs.is_empty() {
            self.index_bytes = 0;
            self.flushed = Instant::now();
            return Ok(());
        }
        let index = Index {
            version: 1,
            supersedes: Vec::new(),
            packs: std::mem::take(&mut self.packs),
        };
        self.repo
            .write_object("index", Label::Index, &serde_json::to_vec(&index)?)?;
        self.flushed = Instant::now();
        self.index_bytes = 0;
        Ok(())
    }
    pub fn finish(&mut self) -> Result<(), Error> {
        self.close_pack()?;
        self.flush_index()
    }
}

pub fn pack_name(id: &str) -> Result<ObjectName, crate::backend::BackendError> {
    ObjectName::new(&format!("data/{}/{id}", id.get(..2).unwrap_or("")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{backend::memory::MemoryBackend, secret::Secret};
    #[test]
    fn empty_index_flush_restarts_the_interval_without_forcing_the_next_blob_out() {
        let repo = Repository::init(MemoryBackend::new(), &Secret::KeyFile([7; 32])).unwrap();
        let mut store = BlobStore::load(&repo).unwrap();
        store.flushed = Instant::now() - Duration::from_secs(301);
        store.index_bytes = 17;
        store.flush_index().unwrap();
        assert_eq!(store.index_bytes, 0);
        store.add(BlobKind::Data, b"new data").unwrap();
        assert!(repo.backend.list(Kind::Data).unwrap().is_empty());
        assert!(store.flushed.elapsed() < Duration::from_secs(1));
    }
}
