use base64::{Engine, prelude::BASE64_STANDARD};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SlotKind {
    Password,
    Keyfile,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Kdf {
    pub(crate) alg: String,
    pub(crate) m_kib: u32,
    pub(crate) t: u32,
    pub(crate) p: u32,
    pub(crate) salt: String,
}

impl Kdf {
    pub fn new(m_kib: u32, t: u32, p: u32, salt: &[u8]) -> Result<Self, String> {
        let kdf = Self {
            alg: "argon2id".into(),
            m_kib,
            t,
            p,
            salt: BASE64_STANDARD.encode(salt),
        };
        kdf.validate()?;
        Ok(kdf)
    }

    pub(crate) fn validate(&self) -> Result<Vec<u8>, String> {
        if self.alg != "argon2id"
            || !(8192..=4194304).contains(&self.m_kib)
            || !(1..=10).contains(&self.t)
            || !(1..=16).contains(&self.p)
        {
            return Err("invalid Argon2id algorithm or bounds".into());
        }
        let salt = BASE64_STANDARD
            .decode(&self.salt)
            .map_err(|e| e.to_string())?;
        if salt.len() < 16 {
            return Err("salt shorter than 16 bytes".into());
        }
        Ok(salt)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct KeySlot {
    pub(crate) version: u32,
    pub kind: SlotKind,
    pub created: String,
    pub host: String,
    pub username: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) kdf: Option<Kdf>,
    pub(crate) wrapped: String,
}

impl KeySlot {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let slot: Self = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        if slot.version != 1 {
            return Err(format!("unknown key slot version {}", slot.version));
        }
        super::validate_time(&slot.created)?;
        match (slot.kind, &slot.kdf) {
            (SlotKind::Password, Some(kdf)) => {
                kdf.validate()?;
            }
            (SlotKind::Keyfile, None) => {}
            _ => return Err("key slot kind and KDF disagree".into()),
        }
        let wrapped = BASE64_STANDARD
            .decode(&slot.wrapped)
            .map_err(|e| e.to_string())?;
        if wrapped.len() != 72 {
            return Err("invalid wrapped master key length".into());
        }
        Ok(slot)
    }
}
