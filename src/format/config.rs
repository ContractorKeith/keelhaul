use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    version: u32,
    pub id: String,
    created: String,
    pub chunker: Chunker,
    pub pack_target: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Chunker {
    alg: String,
    pub min: u64,
    pub avg: u64,
    pub max: u64,
}

impl Config {
    pub fn new() -> std::io::Result<Self> {
        Ok(Self {
            version: 1,
            id: hex::encode(crate::crypto::keys::random::<32>()?),
            created: super::timestamp(),
            chunker: Chunker {
                alg: "fastcdc-gear-keyed-v1".into(),
                min: 524288,
                avg: 1048576,
                max: 8388608,
            },
            pack_target: 16777216,
        })
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let config: Self = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        if config.version != 1 {
            return Err(format!("unknown config version {}", config.version));
        }
        if config.chunker.alg != "fastcdc-gear-keyed-v1" {
            return Err(format!("unknown chunker algorithm {}", config.chunker.alg));
        }
        if !super::valid_id(&config.id) {
            return Err("invalid repository id".into());
        }
        super::validate_time(&config.created)?;
        if config.chunker.min != 524288
            || config.chunker.avg != 1048576
            || config.chunker.max != 8388608
            || config.pack_target != 16777216
        {
            return Err("invalid v1 chunker sizes or pack target".into());
        }
        Ok(config)
    }
}
