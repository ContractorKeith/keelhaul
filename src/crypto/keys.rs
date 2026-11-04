use std::io;

pub struct MasterKey(pub(crate) [u8; 32]);

pub struct Subkeys {
    pub encryption: [u8; 32],
    pub chunk_id: [u8; 32],
    pub gear: [u8; 32],
    pub key_slot_check: [u8; 32],
}

pub fn random<const N: usize>() -> io::Result<[u8; N]> {
    let mut bytes = [0; N];
    getrandom::fill(&mut bytes).map_err(io::Error::other)?;
    Ok(bytes)
}

impl MasterKey {
    pub fn generate() -> io::Result<Self> {
        Ok(Self(random()?))
    }

    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn subkeys(&self) -> Subkeys {
        Subkeys {
            encryption: blake3::derive_key("keelhaul 2026-11 v1 encryption", &self.0),
            chunk_id: blake3::derive_key("keelhaul 2026-11 v1 chunk id", &self.0),
            gear: blake3::derive_key("keelhaul 2026-11 v1 gear", &self.0),
            key_slot_check: blake3::derive_key("keelhaul 2026-11 v1 key slot check", &self.0),
        }
    }
}
