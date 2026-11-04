use super::{
    envelope::{self, Label},
    keys::MasterKey,
};
use crate::{
    format::key_slot::{Kdf, KeySlot},
    secret::Secret,
};
use argon2::{Algorithm, Argon2, Params, Version};
use base64::{Engine, prelude::BASE64_STANDARD};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("bad-format: {0}")]
    BadFormat(String),
    #[error(transparent)]
    Envelope(#[from] envelope::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("secret kind does not match slot")]
    WrongKind,
}

pub(super) fn kek(secret: &Secret, kdf: Option<&Kdf>) -> Result<[u8; 32], Error> {
    match (secret, kdf) {
        (Secret::Password(password), Some(kdf)) => {
            // Validate attacker-controlled cost parameters before constructing Argon2.
            let salt = kdf.validate().map_err(Error::BadFormat)?;
            let params = Params::new(kdf.m_kib, kdf.t, kdf.p, Some(32))
                .map_err(|e| Error::BadFormat(e.to_string()))?;
            let mut key = [0; 32];
            Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
                .hash_password_into(password.as_bytes(), &salt, &mut key)
                .map_err(|e| Error::BadFormat(e.to_string()))?;
            Ok(key)
        }
        (Secret::KeyFile(bytes), None) => {
            Ok(blake3::derive_key("keelhaul 2026-11 v1 key file", bytes))
        }
        _ => Err(Error::WrongKind),
    }
}

pub fn create(master: &MasterKey, secret: &Secret) -> Result<KeySlot, Error> {
    let kdf = match secret {
        Secret::Password(_) => {
            Some(Kdf::new(65536, 3, 4, &super::keys::random::<16>()?).map_err(Error::BadFormat)?)
        }
        Secret::KeyFile(_) => None,
    };
    wrap(master, secret, kdf)
}

pub(crate) fn wrap(
    master: &MasterKey,
    secret: &Secret,
    kdf: Option<Kdf>,
) -> Result<KeySlot, Error> {
    let key = kek(secret, kdf.as_ref())?;
    Ok(KeySlot {
        version: 1,
        kind: secret.kind(),
        created: crate::format::timestamp(),
        host: std::env::var("HOSTNAME").unwrap_or_else(|_| "unknown".into()),
        username: std::env::var("USER").unwrap_or_else(|_| "unknown".into()),
        kdf,
        wrapped: BASE64_STANDARD.encode(envelope::seal(&key, Label::KeySlot, &master.0)?),
    })
}

pub fn unwrap(slot: &KeySlot, secret: &Secret) -> Result<MasterKey, Error> {
    if slot.kind != secret.kind() {
        return Err(Error::WrongKind);
    }
    let key = kek(secret, slot.kdf.as_ref())?;
    let bytes = BASE64_STANDARD
        .decode(&slot.wrapped)
        .map_err(|e| Error::BadFormat(e.to_string()))?;
    let plaintext = envelope::open(&key, Label::KeySlot, &bytes)?;
    let master = plaintext
        .try_into()
        .map_err(|_| Error::BadFormat("invalid master key length".into()))?;
    Ok(MasterKey(master))
}
