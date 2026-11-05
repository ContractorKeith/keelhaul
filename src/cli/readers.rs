//! Shared snapshot rendering with an explicit read-lock choice.
use crate::{backend::Backend, ops, repo::Repository};
pub(super) fn snapshots<B: Backend>(
    repo: &Repository<B>,
    host: Option<String>,
    tag: Option<String>,
    path: Option<String>,
    json: bool,
    no_lock: bool,
) -> anyhow::Result<()> {
    let path = path
        .as_deref()
        .map(|p| ops::paths::normalize_root(std::path::Path::new(p)))
        .transpose()?;
    let scanned = if no_lock {
        repo.scan_snapshots()?
    } else {
        crate::repo::lock::with(&repo.backend, repo.subkeys(), false, |_| {
            repo.scan_snapshots()
        })?
    };
    let snapshots = scanned.entries;
    let ids = snapshots
        .iter()
        .map(|(id, _)| id.clone())
        .collect::<Vec<_>>();
    let selected = snapshots
        .into_iter()
        .filter(|(_, s)| {
            host.as_ref().is_none_or(|h| h == &s.host)
                && tag.as_ref().is_none_or(|t| s.tags.contains(t))
                && path
                    .as_ref()
                    .is_none_or(|p| s.paths.iter().any(|saved| std::path::Path::new(saved) == p))
        })
        .collect::<Vec<_>>();
    if json {
        let values = selected
            .into_iter()
            .map(|(id, s)| {
                let mut value = serde_json::to_value(s)?;
                value["id"] = id.into();
                Ok(value)
            })
            .collect::<Result<Vec<_>, serde_json::Error>>()?;
        println!("{}", serde_json::to_string(&values)?);
    } else {
        println!("ID        TIME                            HOST  PATHS");
        for (id, s) in selected {
            println!(
                "{}  {}  {}  {}",
                super::backup::short_id(&id, &ids),
                s.time,
                s.host,
                s.paths.join(", ")
            );
        }
    }
    if scanned.damaged {
        super::backup::completed_damage()?;
    }
    Ok(())
}
