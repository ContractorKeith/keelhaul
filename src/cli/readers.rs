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

// Only failed lock writes on readers suggest bypassing locking.
pub(super) fn lock_hint(error: anyhow::Error, reader: bool) -> anyhow::Error {
    if reader && error.chain().any(|cause| {
        matches!(cause.downcast_ref::<crate::backend::BackendError>(),
            Some(crate::backend::BackendError::Io { name: Some(name), source, .. })
            if name.as_str().starts_with("locks/") &&
                (source.kind() == std::io::ErrorKind::PermissionDenied || source.raw_os_error() == Some(30) || source.to_string().contains("HTTP 403")))
    }) {
        anyhow::anyhow!("{error}; the repository looks read-only; use --no-lock for snapshots, ls, restore, check and verify")
    } else {
        error
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_reader_lock_permission_failures_get_the_hint() {
        for (name, code, reader, expected) in [
            ("locks/", 13, true, true),
            ("locks/", 30, true, true),
            ("data/", 13, true, false),
            ("locks/", 13, false, false),
            ("locks/", 5, true, false),
        ] {
            let name = crate::backend::ObjectName::new(&format!("{name}{}", "ab".repeat(32))).ok();
            let error = crate::backend::BackendError::Io {
                name,
                path: None,
                source: std::io::Error::from_raw_os_error(code),
            };
            assert_eq!(
                lock_hint(error.into(), reader)
                    .to_string()
                    .contains("use --no-lock"),
                expected
            );
        }
    }
}
