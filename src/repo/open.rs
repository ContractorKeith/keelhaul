use crate::{
    backend::{Backend, BackendError, Kind, ObjectName},
    crypto::{
        envelope::{self, Label},
        keys::{MasterKey, random},
        slot,
    },
    format::{config::Config, key_slot::KeySlot},
    secret::Secret,
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Backend(#[from] BackendError),
    #[error("wrong secret: no key slot matches")]
    WrongSecret,
    #[error("damage: {object}: {problem}: {detail}")]
    Damage {
        object: String,
        problem: &'static str,
        detail: String,
    },
    #[error("{0}")]
    Refused(String),
    #[error(transparent)]
    Slot(#[from] slot::Error),
    #[error(transparent)]
    Envelope(#[from] envelope::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub struct Repository<B> {
    pub backend: B,
    pub config: Config,
    master: MasterKey,
}

impl<B: Backend> Repository<B> {
    pub fn init(backend: B, secret: &Secret) -> Result<Self, Error> {
        for kind in Kind::ALL {
            if !backend.list(kind)?.is_empty() || !backend.list_foreign(kind)?.is_empty() {
                return Err(Error::Refused("repository is not empty".into()));
            }
        }
        let master = MasterKey::generate()?;
        let config = Config::new()?;
        let repo = Self {
            backend,
            config,
            master,
        };
        repo.add_key(secret)?;
        let bytes = serde_json::to_vec(&repo.config)?;
        let encrypted = envelope::seal(&repo.master.subkeys().encryption, Label::Config, &bytes)?;
        repo.backend
            .write(&ObjectName::new("config")?, &encrypted)?;
        Ok(repo)
    }

    pub fn open(backend: B, secret: &Secret) -> Result<Self, Error> {
        let mut entries = backend.list(Kind::Keys)?;
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        let mut master = None;
        for entry in entries {
            let bytes = backend.read(&entry.name)?;
            // Invalid or nonmatching slots do not prevent a later valid slot unlocking.
            let Ok(slot) = KeySlot::parse(&bytes) else {
                continue;
            };
            if let Ok(key) = slot::unwrap(&slot, secret) {
                master = Some(key);
                break;
            }
        }
        let master = master.ok_or(Error::WrongSecret)?;
        let name = ObjectName::new("config")?;
        let bytes = backend.read(&name).map_err(|e| match e {
            BackendError::NotFound(_) => damage("config", "missing", "config is missing"),
            other => Error::Backend(other),
        })?;
        let payload =
            envelope::open(&master.subkeys().encryption, Label::Config, &bytes).map_err(|e| {
                let problem = if matches!(e, envelope::Error::Undecryptable) {
                    "undecryptable"
                } else {
                    "bad-format"
                };
                damage("config", problem, &e.to_string())
            })?;
        let config = Config::parse(&payload).map_err(|e| damage("config", "bad-format", &e))?;
        Ok(Self {
            backend,
            config,
            master,
        })
    }

    pub fn add_key(&self, secret: &Secret) -> Result<String, Error> {
        let slot = slot::create(&self.master, secret)?;
        let id = hex::encode(random::<32>()?);
        self.backend.write(
            &ObjectName::new(&format!("keys/{id}"))?,
            &serde_json::to_vec(&slot)?,
        )?;
        Ok(id)
    }

    pub fn list_keys(&self) -> Result<Vec<(String, KeySlot)>, Error> {
        let mut entries = self.backend.list(Kind::Keys)?;
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        entries
            .into_iter()
            .map(|entry| {
                let bytes = self.backend.read(&entry.name)?;
                let slot = KeySlot::parse(&bytes)
                    .map_err(|e| damage(entry.name.as_str(), "bad-format", &e))?;
                Ok((entry.name.as_str()[5..].to_owned(), slot))
            })
            .collect()
    }

    pub fn remove_key(&self, id: &str) -> Result<(), Error> {
        // M5: exclusive lock
        let name = ObjectName::new(&format!("keys/{id}"))?;
        let slots = self.list_keys()?;
        if !slots.iter().any(|(found, _)| found == id) {
            return Err(Error::Refused(format!("key slot {id} not found")));
        }
        if slots.len() <= 1 {
            return Err(Error::Refused("cannot remove the last key slot".into()));
        }
        self.backend.delete(&name)?;
        Ok(())
    }
}

fn damage(object: &str, problem: &'static str, detail: &str) -> Error {
    Error::Damage {
        object: object.into(),
        problem,
        detail: detail.into(),
    }
}
