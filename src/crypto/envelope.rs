use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit, Payload},
};
use std::io::{self, Read};

pub const MAX_PAYLOAD: u64 = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Label {
    KeySlot,
    Config,
    Data,
    Tree,
    PackHeader,
    Index,
    Snapshot,
    Lock,
}

impl Label {
    pub const ALL: [Self; 8] = [
        Self::KeySlot,
        Self::Config,
        Self::Data,
        Self::Tree,
        Self::PackHeader,
        Self::Index,
        Self::Snapshot,
        Self::Lock,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::KeySlot => "keelhaul/key-slot",
            Self::Config => "keelhaul/config",
            Self::Data => "keelhaul/blob/data",
            Self::Tree => "keelhaul/blob/tree",
            Self::PackHeader => "keelhaul/pack-header",
            Self::Index => "keelhaul/index",
            Self::Snapshot => "keelhaul/snapshot",
            Self::Lock => "keelhaul/lock",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("undecryptable envelope")]
    Undecryptable,
    #[error("bad-format: {0}")]
    BadFormat(String),
    #[error(transparent)]
    Io(#[from] io::Error),
}

pub fn seal(key: &[u8; 32], label: Label, payload: &[u8]) -> Result<Vec<u8>, Error> {
    seal_inner(key, label, payload, &super::keys::random()?)
}

#[cfg(test)]
pub(super) fn seal_with_nonce(
    key: &[u8; 32],
    label: Label,
    payload: &[u8],
    nonce: &[u8; 24],
) -> Result<Vec<u8>, Error> {
    seal_inner(key, label, payload, nonce)
}

fn seal_inner(
    key: &[u8; 32],
    label: Label,
    payload: &[u8],
    nonce: &[u8; 24],
) -> Result<Vec<u8>, Error> {
    let plaintext = if label == Label::KeySlot {
        if payload.len() != 32 {
            return Err(Error::BadFormat("master key must be 32 bytes".into()));
        }
        payload.to_vec()
    } else {
        if payload.len() as u64 > MAX_PAYLOAD {
            return Err(Error::BadFormat("payload exceeds 64 MiB".into()));
        }
        let compressed = zstd::stream::encode_all(payload, 3)?;
        let (flag, bytes) = if compressed.len() < payload.len() {
            (1, compressed.as_slice())
        } else {
            (0, payload)
        };
        let mut plaintext = vec![flag];
        plaintext.extend_from_slice(bytes);
        plaintext
    };
    encrypt(key, label, &plaintext, nonce)
}

fn encrypt(
    key: &[u8; 32],
    label: Label,
    plaintext: &[u8],
    nonce: &[u8; 24],
) -> Result<Vec<u8>, Error> {
    let ciphertext = XChaCha20Poly1305::new(key.into())
        .encrypt(
            XNonce::from_slice(nonce),
            Payload {
                msg: plaintext,
                aad: label.as_str().as_bytes(),
            },
        )
        .map_err(|_| Error::Undecryptable)?;
    let mut stored = nonce.to_vec();
    stored.extend(ciphertext);
    Ok(stored)
}

pub fn open(key: &[u8; 32], label: Label, stored: &[u8]) -> Result<Vec<u8>, Error> {
    if stored.len() < 40 {
        return Err(Error::BadFormat("truncated envelope".into()));
    }
    let plaintext = XChaCha20Poly1305::new(key.into())
        .decrypt(
            XNonce::from_slice(&stored[..24]),
            Payload {
                msg: &stored[24..],
                aad: label.as_str().as_bytes(),
            },
        )
        .map_err(|_| Error::Undecryptable)?;
    if label == Label::KeySlot {
        return if plaintext.len() == 32 {
            Ok(plaintext)
        } else {
            Err(Error::BadFormat("master key must be 32 bytes".into()))
        };
    }
    match plaintext.split_first() {
        Some((0, bytes)) if bytes.len() as u64 <= MAX_PAYLOAD => Ok(bytes.to_vec()),
        Some((1, bytes)) => {
            let mut decoder = zstd::stream::read::Decoder::with_buffer(bytes)
                .map_err(|e| Error::BadFormat(e.to_string()))?
                .single_frame();
            decoder
                .window_log_max(27)
                .map_err(|e| Error::BadFormat(e.to_string()))?;
            let mut payload = Vec::new();
            (&mut decoder)
                .take(MAX_PAYLOAD + 1)
                .read_to_end(&mut payload)
                .map_err(|e| Error::BadFormat(e.to_string()))?;
            if payload.len() as u64 > MAX_PAYLOAD {
                return Err(Error::BadFormat(
                    "decompressed payload exceeds 64 MiB".into(),
                ));
            }
            if payload.is_empty() {
                return Err(Error::BadFormat("compressed payload is empty".into()));
            }
            if !decoder.finish().is_empty() {
                return Err(Error::BadFormat("trailing bytes after zstd frame".into()));
            }
            Ok(payload)
        }
        _ => Err(Error::BadFormat("invalid compression flag or size".into())),
    }
}

#[cfg(test)]
mod tests;
