use crate::{
    backend::{Backend, Kind, ObjectName},
    repo::{
        Error, Repository,
        lock::{Clock, Guard},
    },
};
use std::{
    collections::HashSet,
    time::{Duration, Instant},
};
pub(super) enum Target {
    Object(ObjectName),
    Foreign(String),
}
pub(super) fn fence<B: Backend>(
    repo: &Repository<B>,
    guard: &Guard<'_, B>,
    snapshots: &HashSet<String>,
) -> Result<(), Error> {
    guard.self_check()?;
    if repo
        .backend
        .list(Kind::Snapshots)?
        .iter()
        .any(|entry| !snapshots.contains(entry.name.as_str()))
    {
        return Err(Error::Refused(
            "a backup ran during prune; rerun prune".into(),
        ));
    }
    Ok(())
}
pub(super) fn run<B: Backend>(
    repo: &Repository<B>,
    guard: &Guard<'_, B>,
    snapshots: &HashSet<String>,
    clock: &dyn Clock,
    targets: impl IntoIterator<Item = Target>,
) -> Result<(), Error> {
    let mut checked = clock.now();
    let mut elapsed = Instant::now();
    let mut count = 0;
    for target in targets {
        if count == 256
            || elapsed.elapsed() >= Duration::from_secs(10)
            || clock.now().duration_since(checked).unwrap_or_default() >= Duration::from_secs(10)
        {
            fence(repo, guard, snapshots)?;
            checked = clock.now();
            elapsed = Instant::now();
            count = 0;
        }
        match target {
            Target::Object(name) => repo.backend.delete(&name)?,
            Target::Foreign(raw) => repo.backend.delete_foreign(&raw)?,
        }
        count += 1;
    }
    Ok(())
}
