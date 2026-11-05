use super::*;
use std::sync::mpsc::{Receiver, SyncSender};

pub struct SealedBlob {
    pub id: String,
    pub kind: BlobKind,
    pub stored: Vec<u8>,
    pub raw_length: u64,
}
pub struct Upload {
    pub name: ObjectName,
    pub bytes: Vec<u8>,
}
pub(super) struct Uploads {
    sender: SyncSender<Upload>,
    completed: Receiver<Result<(), Error>>,
    outstanding: usize,
}
impl Uploads {
    pub fn send(&mut self, upload: Upload) -> Result<(), Error> {
        self.sender.send(upload).map_err(|error| {
            Error::Refused(format!(
                "upload pool stopped while sending {}",
                error.0.name
            ))
        })?;
        self.outstanding += 1;
        Ok(())
    }
    pub fn wait(&mut self) -> Result<(), Error> {
        while self.outstanding > 0 {
            self.completed
                .recv()
                .map_err(|_| Error::Refused("upload pool stopped".into()))??;
            self.outstanding -= 1;
        }
        Ok(())
    }
}
impl<'a, B: Backend> BlobStore<'a, B> {
    /// Fresh assembler for prune: retained data is explicitly copied, never deduplicated away.
    pub fn repack_writer(repo: &'a Repository<B>) -> Result<Self, Error> {
        let mut writer = Self::load(repo)?;
        writer.index.clear();
        writer.headers.clear();
        Ok(writer)
    }
    pub(crate) fn written_indexes(&self) -> &[String] {
        &self.flushed_indexes
    }
    pub fn written_packs(&self) -> Vec<crate::format::index::Pack> {
        self.headers
            .iter()
            .map(|(id, (_, blobs))| crate::format::index::Pack {
                id: id.clone(),
                blobs: blobs.clone(),
            })
            .collect()
    }

    pub fn upload_pool(
        &mut self,
        sender: SyncSender<Upload>,
        completed: Receiver<Result<(), Error>>,
    ) {
        self.uploads = Some(Uploads {
            sender,
            completed,
            outstanding: 0,
        });
    }
    /// The read workers own their keys; the repository's master key stays private.
    pub fn worker_keys(&self) -> ([u8; 32], [u8; 32]) {
        let keys = self.repo.master.subkeys();
        (keys.chunk_id, keys.encryption)
    }
}

/// Per-run verification state; a damaged header is cached too.
#[derive(Default)]
pub struct Healing {
    headers: HashMap<String, bool>,
}
impl<B: Backend> BlobStore<'_, B> {
    /// Verify one indexed copy before deduplicating. Only damage is healable;
    /// transport and resource failures still abort the backup.
    pub fn add_healing(&mut self, blob: SealedBlob, checked: &mut Healing) -> Result<bool, Error> {
        let key = (blob.kind, blob.id.clone());
        let mut healed = self.suspect.contains(&key);
        if let Some(location) = self.index.get(&key).cloned() {
            let name = pack_name(&location.pack)?;
            let keys = self.repo.master.subkeys();
            let header_ok = match checked.headers.get(&location.pack) {
                Some(ok) => *ok,
                None => {
                    let result = crate::repo::audit::header(&self.repo.backend, &keys, &name)
                        .and_then(|header| {
                            if self
                                .headers
                                .get(&location.pack)
                                .is_none_or(|(_, expected)| *expected != header.blobs)
                            {
                                return Err(damage(
                                    name.as_str(),
                                    "index-mismatch",
                                    "index differs from complete pack header",
                                ));
                            }
                            Ok(())
                        });
                    let ok = match result {
                        Ok(()) => true,
                        Err(Error::Damage { .. }) => false,
                        Err(error) => return Err(error),
                    };
                    checked.headers.insert(location.pack.clone(), ok);
                    ok
                }
            };
            let result = if header_ok {
                self.repo
                    .backend
                    .read_range(&name, location.blob.offset, location.blob.length)
                    .map_err(|e| missing(&name, e))
                    .and_then(|stored| {
                        crate::repo::audit::blob(
                            &keys,
                            &blob.id,
                            name.as_str(),
                            &location.blob,
                            &stored,
                        )
                        .map(|_| ())
                    })
            } else {
                Err(damage(
                    name.as_str(),
                    "bad-header",
                    "stored pack header failed verification",
                ))
            };
            match result {
                Ok(()) => return Ok(false),
                Err(Error::Damage { .. }) => {
                    self.suspect_blob(blob.kind, &blob.id);
                    healed = true;
                }
                Err(error) => return Err(error),
            }
        }
        self.add_sealed(blob)?;
        Ok(healed)
    }
}
