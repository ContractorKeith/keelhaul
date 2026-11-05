//! Cooperative repository locks. The scoped refresher cannot outlive its backend.
mod payload;
use super::{Error, audit};
use crate::{
    backend::{Backend, BackendError, Kind, ObjectName},
    crypto::{
        envelope::{self, Label},
        keys::Subkeys,
    },
};
pub use payload::{Clock, Lock, STALE, SystemClock};
use std::{
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};

struct Held {
    name: ObjectName,
    payload: Lock,
    retired: Vec<ObjectName>,
}
pub struct Guard<'a, B: Backend> {
    backend: &'a B,
    keys: Subkeys,
    clock: &'a dyn Clock,
    held: Mutex<Held>,
    refresh_failed: AtomicBool,
}
impl<'a, B: Backend> Guard<'a, B> {
    pub fn acquire(
        backend: &'a B,
        keys: Subkeys,
        exclusive: bool,
        clock: &'a dyn Clock,
    ) -> Result<Self, Error> {
        Self::acquire_inner(backend, keys, exclusive, clock, false).map(|(guard, _)| guard)
    }
    fn acquire_inner(
        backend: &'a B,
        keys: Subkeys,
        exclusive: bool,
        clock: &'a dyn Clock,
        audit: bool,
    ) -> Result<(Self, Vec<crate::damage::Damage>), Error> {
        let payload = Lock::new(exclusive, clock.now())?;
        let name = write(backend, &keys, &payload)?;
        let guard = Self {
            backend,
            keys,
            clock,
            refresh_failed: AtomicBool::new(false),
            held: Mutex::new(Held {
                name,
                payload,
                retired: Vec::new(),
            }),
        };
        let own = guard.held.lock().map_err(poison)?.name.clone();
        let mut damage = Vec::new();
        for (name, lock) in scan(backend, &guard.keys, |error| {
            if audit {
                damage.push(crate::damage::Damage::from_error(error)?);
                Ok(())
            } else {
                Err(error)
            }
        })? {
            if name != own && !lock.dead() && (exclusive || lock.exclusive) {
                return Err(Error::Locked(format!(
                    "repository locked by {} pid {} (age {}s); if that process is gone, run `keelhaul unlock`",
                    lock.host,
                    lock.pid,
                    lock.age(clock.now())?.as_secs()
                )));
            }
        }
        Ok((guard, damage))
    }
    pub fn self_check(&self) -> Result<(), Error> {
        if self.refresh_failed.load(Ordering::Acquire) {
            return Err(Error::Locked(
                "lock refresh failed; operation aborted".into(),
            ));
        }
        let held = self.held.lock().map_err(poison)?;
        if held.payload.age(self.clock.now())? > STALE
            || !self
                .backend
                .list(Kind::Locks)?
                .iter()
                .any(|e| e.name == held.name)
        {
            return Err(Error::Locked(
                "own lock disappeared or refresh is older than 30 minutes; operation aborted"
                    .into(),
            ));
        }
        Ok(())
    }
    pub fn refresh(&self) -> Result<(), Error> {
        let result = self.refresh_inner();
        if result.is_err() {
            self.refresh_failed.store(true, Ordering::Release);
        }
        result
    }
    fn refresh_inner(&self) -> Result<(), Error> {
        let mut held = self.held.lock().map_err(poison)?;
        // A stalled process must not resurrect a lock after another operation ignored it.
        if held.payload.age(self.clock.now())? > STALE
            || !self
                .backend
                .list(Kind::Locks)?
                .iter()
                .any(|e| e.name == held.name)
        {
            return Err(Error::Locked("cannot refresh a lost or stale lock".into()));
        }
        let mut payload = held.payload.clone();
        payload.refreshed = payload::timestamp(self.clock.now())?;
        let name = write(self.backend, &self.keys, &payload)?;
        let old = std::mem::replace(&mut held.name, name);
        held.payload = payload;
        held.retired.push(old.clone());
        self.backend.delete(&old)?;
        held.retired.retain(|name| name != &old);
        Ok(())
    }
    pub fn run<T>(&self, operation: impl FnOnce(&Self) -> Result<T, Error>) -> Result<T, Error> {
        self.run_interval(Duration::from_secs(300), operation)
    }
    fn run_interval<T>(
        &self,
        interval: Duration,
        operation: impl FnOnce(&Self) -> Result<T, Error>,
    ) -> Result<T, Error> {
        std::thread::scope(|scope| {
            let (send, recv) = mpsc::channel::<()>();
            // Dropping the sender also wakes the thread when the operation unwinds.
            let worker = scope.spawn(move || {
                while matches!(
                    recv.recv_timeout(interval),
                    Err(mpsc::RecvTimeoutError::Timeout)
                ) {
                    if let Err(error) = self.refresh() {
                        crate::warning!("warning: lock refresh failed: {error}");
                    }
                }
            });
            let result = operation(self);
            drop(send);
            worker
                .join()
                .map_err(|_| Error::Refused("lock refresher panicked".into()))?;
            result
        })
    }
}
impl<B> Drop for Guard<'_, B>
where
    B: Backend,
{
    fn drop(&mut self) {
        let held = self
            .held
            .get_mut()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        {
            for name in std::iter::once(&held.name).chain(&held.retired) {
                if let Err(error) = self.backend.delete(name) {
                    crate::warning!("warning: cannot release lock {name}: {error}");
                }
            }
        }
    }
}
fn poison<T>(_: std::sync::PoisonError<T>) -> Error {
    Error::Refused("lock state poisoned".into())
}
pub fn with<B: Backend, T>(
    backend: &B,
    keys: Subkeys,
    exclusive: bool,
    operation: impl FnOnce(&Guard<'_, B>) -> Result<T, Error>,
) -> Result<T, Error> {
    let guard = Guard::acquire(backend, keys, exclusive, &SystemClock)?;
    guard.run(operation)
}
fn write<B: Backend>(backend: &B, keys: &Subkeys, payload: &Lock) -> Result<ObjectName, Error> {
    let bytes = envelope::seal(&keys.encryption, Label::Lock, &serde_json::to_vec(payload)?)?;
    let name = ObjectName::new(&format!("locks/{}", blake3::hash(&bytes).to_hex()))?;
    if let Err(error) = backend.write(&name, &bytes) {
        let _ = backend.delete(&name);
        return Err(error.context(Some(&name), None).into());
    }
    Ok(name)
}
/// Audits still report malformed locks, while honoring every readable live lock.
/// Mutating operations always refuse malformed locks instead.
pub(crate) fn with_audit<B: Backend>(
    backend: &B,
    keys: Subkeys,
    operation: impl FnOnce(Vec<crate::damage::Damage>) -> Result<crate::damage::Report, Error>,
) -> Result<crate::damage::Report, Error> {
    let (guard, damage) = Guard::acquire_inner(backend, keys, false, &SystemClock, true)?;
    guard.run(|_| operation(damage))
}
pub fn list<B: Backend>(backend: &B, keys: &Subkeys) -> Result<Vec<(ObjectName, Lock)>, Error> {
    scan(backend, keys, Err)
}
fn scan<B: Backend>(
    backend: &B,
    keys: &Subkeys,
    mut damaged: impl FnMut(Error) -> Result<(), Error>,
) -> Result<Vec<(ObjectName, Lock)>, Error> {
    let mut locks = Vec::new();
    for entry in backend.list(Kind::Locks)? {
        let bytes = match audit::object(backend, keys, &entry.name, Label::Lock) {
            // Refresh/release may remove an object between list and read.
            Err(Error::Damage {
                problem: "missing", ..
            })
            | Err(Error::Backend(BackendError::NotFound(_))) => continue,
            Err(error @ Error::Damage { .. }) => {
                damaged(error)?;
                continue;
            }
            other => other?,
        };
        let lock = match Lock::parse(&bytes) {
            Ok(lock) => lock,
            Err(detail) => {
                damaged(super::open::damage(
                    entry.name.as_str(),
                    "bad-format",
                    &detail,
                ))?;
                continue;
            }
        };
        locks.push((entry.name, lock));
    }
    Ok(locks)
}
pub fn unlock<B: Backend>(
    backend: &B,
    keys: &Subkeys,
    clock: &dyn Clock,
) -> Result<Vec<String>, Error> {
    let mut removed = Vec::new();
    for (name, lock) in list(backend, keys)? {
        if lock.dead() || lock.stale(clock.now())? {
            backend.delete(&name)?;
            removed.push(name.to_string());
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests;
