//! Read primitives shared by structural and full audits. Format parsing stays in format/.
use super::{Error, objects::missing, open::damage};
use crate::{
    backend::{Backend, ObjectName},
    crypto::{
        envelope::{self, Label},
        keys::Subkeys,
    },
    format::pack::{Blob, Header, header_length},
};

pub(crate) fn read<B: Backend>(backend: &B, name: &ObjectName) -> Result<Vec<u8>, Error> {
    backend.read(name).map_err(|e| missing(name, e))
}
pub(crate) fn size<B: Backend>(backend: &B, name: &ObjectName) -> Result<u64, Error> {
    backend.stat(name).map_err(|e| missing(name, e))
}
pub(crate) fn range<B: Backend>(
    backend: &B,
    name: &ObjectName,
    offset: u64,
    len: u64,
) -> Result<Vec<u8>, Error> {
    backend
        .read_range(name, offset, len)
        .map_err(|e| missing(name, e))
}
pub(crate) fn open(
    keys: &Subkeys,
    object: &str,
    label: Label,
    stored: &[u8],
) -> Result<Vec<u8>, Error> {
    envelope::open(&keys.encryption, label, stored).map_err(|e| {
        let problem = match e {
            envelope::Error::Undecryptable => "undecryptable",
            _ => "bad-format",
        };
        damage(object, problem, &e.to_string())
    })
}
pub(crate) fn object<B: Backend>(
    backend: &B,
    keys: &Subkeys,
    name: &ObjectName,
    label: Label,
) -> Result<Vec<u8>, Error> {
    let stored = read(backend, name)?;
    let id = name
        .as_str()
        .rsplit('/')
        .next()
        .expect("validated object name");
    if blake3::hash(&stored).to_hex().as_str() != id {
        return Err(damage(
            name.as_str(),
            "hash-mismatch",
            "stored object ID differs",
        ));
    }
    open(keys, name.as_str(), label, &stored)
}
pub(crate) fn header<B: Backend>(
    backend: &B,
    keys: &Subkeys,
    name: &ObjectName,
) -> Result<Header, Error> {
    let len = size(backend, name)?;
    let trailer = backend
        .read_range(name, len.saturating_sub(4), 4)
        .map_err(|e| missing(name, e))?;
    let header_len = header_length(&trailer, len).map_err(|e| {
        damage(
            name.as_str(),
            if len < 4 { "truncated" } else { "bad-header" },
            &e.to_string(),
        )
    })?;
    let stored = backend
        .read_range(name, len - 4 - header_len, header_len)
        .map_err(|e| missing(name, e))?;
    if stored.len() as u64 != header_len {
        return Err(damage(
            name.as_str(),
            "truncated",
            "pack header range is short",
        ));
    }
    let payload = open(keys, name.as_str(), Label::PackHeader, &stored).map_err(|e| match e {
        Error::Damage { detail, .. } => damage(name.as_str(), "bad-header", &detail),
        other => other,
    })?;
    Header::parse(&payload, len - 4 - header_len)
        .map_err(|e| damage(name.as_str(), "bad-header", &e.to_string()))
}
pub(crate) fn blob(
    keys: &Subkeys,
    object: &str,
    pack: &str,
    blob: &Blob,
    stored: &[u8],
) -> Result<Vec<u8>, Error> {
    let result = (|| {
        if stored.len() as u64 != blob.length {
            return Err(damage(object, "truncated", "blob range is short"));
        }
        let payload = open(keys, object, blob.kind.label(), stored)?;
        if payload.len() as u64 != blob.raw_length {
            return Err(damage(object, "size-mismatch", "blob raw length differs"));
        }
        if blake3::keyed_hash(&keys.chunk_id, &payload)
            .to_hex()
            .as_str()
            != blob.id
        {
            return Err(damage(object, "hash-mismatch", "blob keyed ID differs"));
        }
        Ok(payload)
    })();
    result.map_err(|error| match error {
        Error::Damage {
            object,
            problem,
            detail,
        } => Error::Damage {
            object,
            problem,
            detail: format!("{detail}; pack {pack}"),
        },
        other => other,
    })
}
