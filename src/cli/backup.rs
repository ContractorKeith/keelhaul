//! M2 command rendering stays separate from backend selection and key commands.
use super::Command;
use crate::{backend::Backend, ops, repo::Repository};
pub(super) fn run<B: Backend>(
    repo: &Repository<B>,
    command: Command,
    json: bool,
) -> anyhow::Result<()> {
    match command {
        Command::Backup { paths, tag } => {
            let report = ops::backup::backup(repo, &paths, tag)?;
            if json {
                println!("{}", serde_json::to_string(&report)?);
            } else {
                let ids = repo
                    .snapshots()?
                    .into_iter()
                    .map(|(id, _)| id)
                    .collect::<Vec<_>>();
                println!(
                    "snapshot {}: {} files, {} data blobs added, {} skipped",
                    short_id(&report.snapshot, &ids),
                    report.summary.files_new,
                    report.summary.data_blobs_added,
                    report.summary.files_skipped
                );
            }
        }
        Command::Snapshots { host, tag, path } => {
            let snapshots = repo.snapshots()?;
            let ids = snapshots
                .iter()
                .map(|(id, _)| id.clone())
                .collect::<Vec<_>>();
            let selected = snapshots
                .into_iter()
                .filter(|(_, s)| {
                    host.as_ref().is_none_or(|h| h == &s.host)
                        && tag.as_ref().is_none_or(|t| s.tags.contains(t))
                        && path.as_ref().is_none_or(|p| s.paths.contains(p))
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
                        short_id(&id, &ids),
                        s.time,
                        s.host,
                        s.paths.join(", ")
                    );
                }
            }
        }
        Command::Ls { snap, path } => {
            let entries = ops::listing::ls(repo, &snap, path.as_deref())?;
            if json {
                println!("{}", serde_json::to_string(&entries)?);
            } else {
                for entry in entries {
                    println!("{}", entry.path);
                }
            }
        }
        Command::Restore {
            snap,
            target,
            include,
        } => {
            let report = ops::restore::restore(repo, &snap, &target, &include)?;
            if json {
                println!("{}", serde_json::to_string(&report)?);
            } else {
                println!("restored {} entries", report.restored);
            }
            if !report.damaged.is_empty() {
                // The command already rendered its one JSON document and diagnostics.
                // Use the shared exit mapping without main printing a second document.
                use std::io::Write;
                std::io::stdout().flush()?;
                std::process::exit(crate::exit::code(Err(crate::exit::Failure::Damage)) as i32);
            }
        }
        _ => unreachable!("M2 dispatch only"),
    }
    Ok(())
}
fn short_id<'a>(id: &'a str, ids: &[String]) -> &'a str {
    let mut len = 8;
    while len < id.len()
        && ids
            .iter()
            .any(|other| other != id && other.starts_with(&id[..len]))
    {
        len += 1;
    }
    &id[..len]
}
