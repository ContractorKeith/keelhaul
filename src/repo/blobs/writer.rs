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
    completed: Receiver<Result<(), crate::backend::BackendError>>,
    outstanding: usize,
}
impl Uploads {
    pub fn send(&mut self, upload: Upload) -> Result<(), Error> {
        self.sender
            .send(upload)
            .map_err(|_| Error::Refused("upload pool stopped".into()))?;
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
impl<B: Backend> BlobStore<'_, B> {
    pub fn upload_pool(
        &mut self,
        sender: SyncSender<Upload>,
        completed: Receiver<Result<(), crate::backend::BackendError>>,
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
