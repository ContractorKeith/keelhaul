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
        self.verify_pack(&loc.pack)?;
        let name = pack_name(&loc.pack)?;
        let stored = self
            .repo
            .backend
            .read_range(&name, loc.blob.offset, loc.blob.length)
            .map_err(|e| missing(&name, e))?;
        if stored.len() as u64 != loc.blob.length {
            return Err(damage(name.as_str(), "truncated", "blob range is short"));
        }
        let payload = envelope::open(
            &self.repo.master.subkeys().encryption,
            kind.label(),
            &stored,
        )
        .map_err(|e| damage(name.as_str(), "undecryptable", &e.to_string()))?;
        if payload.len() as u64 != loc.blob.raw_length {
            return Err(damage(
                name.as_str(),
                "size-mismatch",
                "blob raw length differs",
            ));
        }
        if blake3::keyed_hash(&self.repo.master.subkeys().chunk_id, &payload)
            .to_hex()
            .as_str()
            != id
        {
            return Err(damage(
                name.as_str(),
                "hash-mismatch",
                "blob keyed ID differs",
            ));
        }
        Ok(payload)
    }
    fn verify_pack(&mut self, id: &str) -> Result<(), Error> {
        if self.verified.contains(id) {
            return Ok(());
        }
        let name = pack_name(id)?;
        let len = self
            .repo
            .backend
            .stat(&name)
            .map_err(|e| missing(&name, e))?;
        let trailer = self
            .repo
            .backend
            .read_range(&name, len.saturating_sub(4), 4)
            .map_err(|e| missing(&name, e))?;
        let header_len = header_length(&trailer, len)
            .map_err(|e| damage(name.as_str(), "bad-header", &e.to_string()))?;
        let stored = self
            .repo
            .backend
            .read_range(&name, len - 4 - header_len, header_len)
            .map_err(|e| missing(&name, e))?;
        let payload = envelope::open(
            &self.repo.master.subkeys().encryption,
            Label::PackHeader,
            &stored,
        )
        .map_err(|e| damage(name.as_str(), "bad-header", &e.to_string()))?;
        let header = Header::parse(&payload, len - 4 - header_len)
            .map_err(|e| damage(name.as_str(), "bad-header", &e.to_string()))?;
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
