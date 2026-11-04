//! M2 command rendering stays separate from backend selection and key commands.
use super::Command;
use crate::{backend::Backend, ops, repo::Repository};
pub(super) fn run<B: Backend>(
    repo: &Repository<B>,
    command: Command,
    json: bool,
    jobs: usize,
) -> anyhow::Result<()> {
    match command {
        Command::Backup {
            paths,
            tag,
            host,
            parent,
            force_rehash,
            mut exclude,
            exclude_file,
            one_file_system,
        } => {
            if let Some(file) = exclude_file {
                for line in std::fs::read_to_string(file)?.lines() {
                    if !line.trim().is_empty() && !line.starts_with('#') {
                        exclude.push(line.to_string());
                    }
                }
            }
            let options = ops::backup::Options {
                host,
                parent,
                force_rehash,
                excludes: exclude,
                one_file_system,
                jobs,
            };
            let report = ops::backup::backup_with_options(repo, &paths, tag, &options)?;
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
                    report.summary.files_new
                        + report.summary.files_changed
                        + report.summary.files_unchanged,
                    report.summary.data_blobs_added,
                    report.summary.files_skipped
                );
            }
        }
        Command::Snapshots { host, tag, path } => {
            let path = path
                .as_deref()
                .map(|p| ops::paths::normalize_root(std::path::Path::new(p)))
                .transpose()?;
            let scanned = crate::repo::lock::with(&repo.backend, repo.subkeys(), false, |_| {
                repo.scan_snapshots()
            })?;
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
                        && path.as_ref().is_none_or(|p| {
                            s.paths.iter().any(|saved| std::path::Path::new(saved) == p)
                        })
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
            if scanned.damaged {
                completed_damage()?;
            }
        }
        Command::Ls { snap, path, host } => {
            let entries = ops::listing::ls_for_host(repo, &snap, path.as_deref(), host.as_deref())?;
            if json {
                println!("{}", serde_json::to_string(&entries)?);
            } else {
                for entry in entries {
                    if entry.shared {
                        println!("{} [shared subtree]", entry.path);
                    } else {
                        println!("{}", entry.path);
                    }
                }
            }
        }
        Command::Restore {
            snap,
            target,
            include,
            host,
        } => {
            let report = match ops::restore::restore_for_host(
                repo,
                &snap,
                &target,
                &include,
                host.as_deref(),
            ) {
                Ok(report) => report,
                Err(error) => return super::audit::restore_failure(error, json),
            };
            if json {
                println!("{}", serde_json::to_string(&report)?);
            } else {
                println!("restored {} entries", report.restored);
                for d in &report.damaged {
                    println!("{d}");
                }
            }
            if !report.damaged.is_empty() {
                completed_damage()?;
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

pub(super) fn completed_damage() -> std::io::Result<()> {
    // Rendering is complete; avoid main emitting a second JSON document.
    use std::io::Write;
    std::io::stdout().flush()?;
    std::process::exit(crate::exit::code(Err(crate::exit::Failure::Damage)) as i32)
}
