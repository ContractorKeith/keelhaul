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
    #[error(
        "wrong secret: no key slot matches ({unreadable} of {total} slots unreadable: {objects})"
    )]
    WrongSecretUnreadable {
        unreadable: usize,
        total: usize,
        objects: String,
    },
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
    pub(super) master: MasterKey,
}

impl<B: Backend> Repository<B> {
    /// Config wins the create-once race before any slot is stored.
    /// A crash before the slot write leaves an unusable empty repository; delete
    /// that directory before retrying init. Existing repositories are never reset.
    pub fn init(backend: B, secret: &Secret) -> Result<Self, Error> {
        secret.validate_new().map_err(slot::Error::from)?;
        for kind in Kind::ALL {
            if !backend.list(kind)?.is_empty() || !backend.list_foreign(kind)?.is_empty() {
                return Err(Error::Refused("repository is not empty; to discard an incomplete init, delete the directory and retry".into()));
            }
        }
        let master = MasterKey::generate()?;
        let config = Config::new()?;
        let repo = Self {
            backend,
            config,
            master,
        };
        let bytes = serde_json::to_vec(&repo.config)?;
        let encrypted = envelope::seal(&repo.master.subkeys().encryption, Label::Config, &bytes)?;
        repo.backend
            .write(&ObjectName::new("config")?, &encrypted)?;
        repo.add_key(secret)?;
        Ok(repo)
    }

    pub fn open(backend: B, secret: &Secret) -> Result<Self, Error> {
        let name = ObjectName::new("config")?;
        let bytes = backend.read(&name).map_err(|e| match e {
            BackendError::NotFound(_) => damage("config", "missing", "config is missing"),
            other => Error::Backend(other),
        })?;
        let master = Self::unlock(&backend, secret)?;
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

    /// Unlock independently of config so audits can collect config damage too.
    pub(crate) fn unlock(backend: &B, secret: &Secret) -> Result<MasterKey, Error> {
        let mut entries = backend.list(Kind::Keys)?;
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        let total = entries.len();
        let mut unreadable = Vec::new();
        let mut master = None;
        for entry in entries {
            let bytes = backend.read(&entry.name)?;
            // Invalid or nonmatching slots do not prevent a later valid slot unlocking.
            let Ok(slot) = KeySlot::parse(&bytes) else {
                unreadable.push(entry.name.to_string());
                continue;
            };
            if let Ok(key) = slot::unwrap(&slot, secret) {
                master = Some(key);
                break;
            }
        }
        master.ok_or_else(|| {
            if unreadable.is_empty() {
                Error::WrongSecret
            } else {
                Error::WrongSecretUnreadable {
                    unreadable: unreadable.len(),
                    total,
                    objects: unreadable.join(", "),
                }
            }
        })
    }

    pub(crate) fn subkeys(&self) -> crate::crypto::keys::Subkeys {
        self.master.subkeys()
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
        let slots = self.list_keys()?;
        if !slots.iter().any(|(found, _)| found == id) {
            return Err(Error::Refused(format!("key slot {id} not found")));
        }
        if slots.len() <= 1 {
            return Err(Error::Refused("cannot remove the last key slot".into()));
        }
        self.backend
            .delete(&ObjectName::new(&format!("keys/{id}"))?)?;
        Ok(())
    }
}

pub(super) fn damage(object: &str, problem: &'static str, detail: &str) -> Error {
    Error::Damage {
        object: object.into(),
        problem,
        detail: detail.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::memory::MemoryBackend;

    #[test]
    fn opens_alternate_pack_target_and_refuses_zero() {
        for target in [32 * 1024 * 1024, 0] {
            let secret = Secret::KeyFile([5; 32]);
            let mut repo = Repository::init(MemoryBackend::new(), &secret).unwrap();
            repo.config.pack_target = target;
            let stored = envelope::seal(
                &repo.master.subkeys().encryption,
                Label::Config,
                &serde_json::to_vec(&repo.config).unwrap(),
            )
            .unwrap();
            repo.backend.plant_raw("config", &stored).unwrap();
            match Repository::open(repo.backend, &secret) {
                Ok(opened) => assert_eq!(opened.config.pack_target, 32 * 1024 * 1024),
                Err(error) => {
                    assert_eq!(target, 0);
                    assert!(matches!(
                        error,
                        Error::Damage {
                            problem: "bad-format",
                            ..
                        }
                    ));
                }
            }
        }
    }
}
