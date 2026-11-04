//! Authenticated ranged reads and complete index/header agreement.
use super::*;
impl<B: Backend> BlobStore<'_, B> {
    pub fn raw_length(&self, kind: BlobKind, id: &str) -> Result<u64, Error> {
        self.index
            .get(&(kind, id.into()))
            .map(|loc| loc.blob.raw_length)
            .ok_or_else(|| damage(id, "dangling-reference", "blob is absent from live index"))
    }
    pub fn read(&mut self, kind: BlobKind, id: &str) -> Result<Vec<u8>, Error> {
        let loc =
            self.index.get(&(kind, id.into())).cloned().ok_or_else(|| {
                damage(id, "dangling-reference", "blob is absent from live index")
            })?;
        let mut result = self.read_location(id, &loc);
        if !matches!(result, Err(Error::Damage { .. })) {
            return result;
        }
        let mut alternatives = self
            .headers
            .iter()
            .filter(|(pack, _)| *pack != &loc.pack)
            .flat_map(|(pack, (_, blobs))| {
                blobs
                    .iter()
                    .filter(move |b| b.kind == kind && b.id == id)
                    .map(move |blob| Location {
                        pack: pack.clone(),
                        blob: blob.clone(),
                    })
            })
            .collect::<Vec<_>>();
        alternatives.sort_by(|a, b| a.pack.cmp(&b.pack));
        for alternative in alternatives {
            result = self.read_location(id, &alternative);
            if !matches!(result, Err(Error::Damage { .. })) {
                return result;
            }
        }
        result
    }
    fn read_location(&mut self, id: &str, loc: &Location) -> Result<Vec<u8>, Error> {
        self.verify_pack(&loc.pack)?;
        let name = pack_name(&loc.pack)?;
        let stored = self
            .repo
            .backend
            .read_range(&name, loc.blob.offset, loc.blob.length)
            .map_err(|e| missing(&name, e))?;
        crate::repo::audit::blob(
            &self.repo.master.subkeys(),
            id,
            name.as_str(),
            &loc.blob,
            &stored,
        )
    }

    fn verify_pack(&mut self, id: &str) -> Result<(), Error> {
        if self.verified.contains(id) {
            return Ok(());
        }
        let name = pack_name(id)?;
        let header =
            crate::repo::audit::header(&self.repo.backend, &self.repo.master.subkeys(), &name)?;
        if let Some((object, expected)) = self.headers.get(id)
            && expected != &header.blobs
        {
            return Err(damage(
                object,
                "index-mismatch",
                "index differs from complete pack header",
            ));
        }
        // Blob AEAD and keyed IDs authenticate ranged reads independently; M3
        // checks the whole pack ID so one damaged blob need not block its neighbors.
        self.verified.insert(id.into());
        Ok(())
    }
}
