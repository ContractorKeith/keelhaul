//! Validate existing destination bytes without following symlinks.
use super::{destination::Destination, plan::Item};
use crate::{
    backend::Backend,
    format::{pack::BlobKind, tree::Node},
    repo::{Error, blobs::BlobStore},
};
use std::io::Read;

pub(super) fn identical<B: Backend>(
    destination: &Destination,
    store: &BlobStore<'_, B>,
    item: &Item,
) -> Result<bool, Error> {
    let Node::File { size, content } = &item.entry.node else {
        return Ok(false);
    };
    let Some(mut file) = destination.existing_file(&item.path)? else {
        return Ok(false);
    };
    if file.metadata()?.len() != *size {
        return Ok(false);
    }
    for id in content {
        let len = store.raw_length(BlobKind::Data, id)?;
        let mut chunk = (&mut file).take(len);
        let mut hash = blake3::Hasher::new_keyed(&store.chunk_id_key());
        let mut read = 0;
        let mut buffer = [0; 65536];
        loop {
            let n = chunk.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            read += n as u64;
            hash.update(&buffer[..n]);
        }
        if read != len || hash.finalize().to_hex().as_str() != id {
            return Ok(false);
        }
    }
    let mut extra = [0];
    Ok(file.read(&mut extra)? == 0)
}
