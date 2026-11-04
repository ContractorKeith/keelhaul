//! Parent metadata is a reuse hint. A failed verification invalidates dedup hints.
use super::*;
impl<B: Backend> BlobStore<'_, B> {
    pub(crate) fn read_parent_tree(&mut self, id: &str) -> Result<Vec<u8>, Error> {
        if let Some(location) = self.index.get(&(BlobKind::Tree, id.into())).cloned() {
            let name = pack_name(&location.pack)?;
            if !self.parent_checked.contains(&location.pack) {
                let checked = self
                    .repo
                    .backend
                    .read(&name)
                    .map_err(|e| missing(&name, e))
                    .and_then(|bytes| {
                        if blake3::hash(&bytes).to_hex().as_str() != location.pack {
                            return Err(damage(
                                name.as_str(),
                                "hash-mismatch",
                                "parent pack stored ID differs",
                            ));
                        }
                        Ok(())
                    });
                if let Err(error) = checked {
                    if matches!(error, Error::Damage { .. }) {
                        self.distrust_pack(&location.pack);
                    }
                    return Err(error);
                }
                self.parent_checked.insert(location.pack.clone());
            }
            match self.read(BlobKind::Tree, id) {
                Ok(bytes) => return Ok(bytes),
                Err(error) => {
                    if matches!(error, Error::Damage { .. }) {
                        self.distrust_pack(&location.pack);
                    }
                    return Err(error);
                }
            }
        }
        self.suspect_blob(BlobKind::Tree, id);
        self.read(BlobKind::Tree, id)
    }
    pub(crate) fn suspect_blob(&mut self, kind: BlobKind, id: &str) {
        self.suspect.insert((kind, id.into()));
        self.index.remove(&(kind, id.into()));
    }
    fn distrust_pack(&mut self, pack: &str) {
        let ids = self
            .headers
            .get(pack)
            .map(|(_, blobs)| {
                blobs
                    .iter()
                    .map(|b| (b.kind, b.id.clone()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for (kind, id) in ids {
            self.suspect_blob(kind, &id);
        }
        self.verified.remove(pack);
        self.parent_checked.remove(pack);
    }
}
