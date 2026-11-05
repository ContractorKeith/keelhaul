use super::{Cli, Command, KeyCommand};
use crate::output;

pub fn run(cli: Cli) -> anyhow::Result<()> {
    let read_only = matches!(
        cli.command,
        Command::Snapshots { .. }
            | Command::Ls { .. }
            | Command::Restore { .. }
            | Command::Check
            | Command::Verify { .. }
    );
    run_inner(cli).map_err(|error| super::readers::lock_hint(error, read_only))
}
fn run_inner(mut cli: Cli) -> anyhow::Result<()> {
    output::configure(cli.quiet, cli.json);
    if cli.no_lock {
        if !matches!(
            cli.command,
            Command::Snapshots { .. }
                | Command::Ls { .. }
                | Command::Restore { .. }
                | Command::Check
                | Command::Verify { .. }
        ) {
            return Err(crate::repo::Error::Usage(
                "--no-lock is only allowed for snapshots, ls, restore, check, and verify".into(),
            )
            .into());
        }
        crate::warning!("warning: --no-lock skips repository locking; exclude concurrent writers");
    }
    super::secrets::resolve(&mut cli);
    match cli.command {
        Command::Version => output::version(output::stdout(), cli.json)?,
        command => {
            use crate::{repo::Repository, secret};
            let path = cli
                .repo
                .ok_or_else(|| crate::repo::Error::Usage("--repo is required".into()))?;
            let initializing = matches!(command, Command::Init);
            let backend = super::repo::select(&path, initializing, cli.verbose > 0)?;
            #[cfg(feature = "fault-injection")]
            let backend: Box<dyn crate::backend::Backend> =
                Box::new(crate::backend::fault::FaultBackend::from_env(
                    backend,
                    if path.starts_with("s3:") || path.starts_with("sftp:") {
                        None
                    } else {
                        Some(std::path::PathBuf::from(&path))
                    },
                )?);
            let load = if initializing {
                secret::load_for_init
            } else {
                secret::load
            };
            let secret = load(
                cli.password_file.as_deref(),
                cli.password_command.as_deref(),
                cli.key_file.as_deref(),
            )?;
            let (value, text) = match command {
                Command::Init => {
                    let repo = Repository::init(backend, &secret)?;
                    (
                        serde_json::json!({"id": repo.config.id}),
                        format!("created repository {}", repo.config.id),
                    )
                }
                Command::Check | Command::Verify { .. } => {
                    let report = match command {
                        Command::Check if cli.no_lock => {
                            crate::ops::check::check_backend_without_lock(&backend, &secret)?
                        }
                        Command::Check => crate::ops::check::check_backend(&backend, &secret)?,
                        Command::Verify { subset } if cli.no_lock => {
                            crate::ops::verify::verify_backend_without_lock(
                                &backend, &secret, subset,
                            )?
                        }
                        Command::Verify { subset } => {
                            crate::ops::verify::verify_backend(&backend, &secret, subset)?
                        }
                        _ => unreachable!("audit dispatch"),
                    };
                    if cli.json {
                        println!("{}", serde_json::to_string(&report)?);
                    } else {
                        for damage in &report.damage {
                            output::damage(damage);
                        }
                        for note in &report.notes {
                            println!("{}: {}", note.object.escape_debug(), note.problem);
                        }
                        if report.damage.is_empty() {
                            println!(
                                "no damage: {} packs, {} snapshots checked",
                                backend.list(crate::backend::Kind::Data)?.len(),
                                backend.list(crate::backend::Kind::Snapshots)?.len()
                            );
                        }
                    }
                    if !report.damage.is_empty() {
                        super::backup::completed_damage()?;
                    }
                    return Ok(());
                }
                Command::Forget(ref options) => {
                    options.validate()?;
                    let repo = Repository::open(backend, &secret)?;
                    let report = crate::ops::forget::forget(&repo, options)?;
                    let mut text = report
                        .groups
                        .iter()
                        .flat_map(|g| {
                            g.keep
                                .iter()
                                .map(|s| format!("keep {} ({})", s.id, s.reasons.join(", ")))
                                .chain(g.remove.iter().map(|id| format!("remove {id}")))
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    if let Some(summary) = &report.prune {
                        if !text.is_empty() {
                            text.push('\n');
                        }
                        text.push_str(&format!(
                            "{} packs removed, {} repacked, {} bytes freed{}",
                            summary.packs_removed,
                            summary.packs_repacked,
                            summary.bytes_freed,
                            if options.dry_run { " (dry run)" } else { "" }
                        ));
                    }
                    (serde_json::to_value(report)?, text)
                }
                Command::Prune {
                    max_unused,
                    dry_run,
                } => {
                    let repo = Repository::open(backend, &secret)?;
                    let summary = crate::ops::prune::prune(&repo, max_unused, dry_run)?;
                    let mut text = format!(
                        "{} packs removed, {} repacked, {} bytes freed{}",
                        summary.packs_removed,
                        summary.packs_repacked,
                        summary.bytes_freed,
                        if dry_run { " (dry run)" } else { "" }
                    );
                    for warning in &summary.warnings {
                        text.push_str(&format!("\nwarning: {warning}"));
                    }
                    (serde_json::to_value(summary)?, text)
                }
                Command::RebuildIndex { dry_run } => {
                    let repo = Repository::open(backend, &secret)?;
                    let summary = crate::ops::rebuild_index::rebuild(&repo, dry_run)?;
                    let text = format!(
                        "{} packs indexed, {} indexes written, {} old indexes removed{}",
                        summary.packs,
                        summary.indexes_written,
                        summary.indexes_removed,
                        if dry_run { " (dry run)" } else { "" }
                    );
                    if cli.json {
                        println!("{}", serde_json::to_string(&summary)?);
                    } else {
                        println!("{text}");
                        for damage in &summary.damage {
                            output::damage(damage);
                        }
                    }
                    if !summary.damage.is_empty() {
                        super::backup::completed_damage()?;
                    }
                    return Ok(());
                }
                Command::Unlock => {
                    let repo = Repository::open(backend, &secret)?;
                    let removed = crate::repo::lock::unlock(
                        &repo.backend,
                        &repo.subkeys(),
                        &crate::repo::lock::SystemClock,
                    )?;
                    let text = format!("removed {} locks", removed.len());
                    (serde_json::json!({"removed": removed}), text)
                }
                Command::Key { command } => {
                    let repo = Repository::open(backend, &secret)?;
                    match command {
                        KeyCommand::List => {
                            crate::ops::key::render_list(output::stdout(), &repo, cli.json)?;
                            return Ok(());
                        }
                        KeyCommand::Add {
                            new_key_file,
                            new_password_file,
                        } => {
                            let new_secret = match new_key_file {
                                Some(path) => {
                                    if repo.backend.contains_destination(&path)? {
                                        return Err(secret::Error::KeyInRepository.into());
                                    }
                                    secret::new_key_file(&path)?
                                }
                                None => secret::new_password(new_password_file.as_deref())?,
                            };
                            let id = repo.add_key(&new_secret)?;
                            (
                                serde_json::json!({"id": id}),
                                format!("added key slot {id}"),
                            )
                        }
                        KeyCommand::Remove { id } => {
                            repo.remove_key(&id)?;
                            (
                                serde_json::json!({"removed": id}),
                                format!("removed key slot {id}"),
                            )
                        }
                    }
                }
                command @ (Command::Backup { .. }
                | Command::Snapshots { .. }
                | Command::Ls { .. }
                | Command::Restore { .. }) => {
                    let restoring = matches!(command, Command::Restore { .. });
                    let repo = match Repository::open(backend, &secret) {
                        Ok(repo) => repo,
                        Err(error) if restoring => {
                            return super::audit::restore_failure(error, cli.json);
                        }
                        Err(error) => return Err(error.into()),
                    };
                    if let Command::Restore {
                        snap,
                        target,
                        include,
                        skip_identical,
                        host,
                    } = command
                    {
                        super::audit::restore(
                            &repo,
                            &snap,
                            &target,
                            &include,
                            crate::ops::restore::Options {
                                host: host.as_deref(),
                                skip_identical,
                                no_lock: cli.no_lock,
                            },
                            cli.json,
                        )?;
                    } else if let Command::Ls { snap, path, host } = command {
                        super::audit::listing(
                            &repo,
                            &snap,
                            path.as_deref(),
                            host.as_deref(),
                            cli.json,
                            cli.no_lock,
                        )?;
                    } else if let Command::Snapshots { host, tag, path } = command {
                        super::readers::snapshots(&repo, host, tag, path, cli.json, cli.no_lock)?;
                    } else {
                        super::backup::run(&repo, command, cli.json, cli.jobs as usize)?;
                    }
                    return Ok(());
                }
                Command::Version => unreachable!("version handled above"),
            };
            if cli.json {
                println!("{}", serde_json::to_string(&value)?);
            } else {
                println!("{text}");
            }
        }
    }
    Ok(())
}
