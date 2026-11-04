use super::{Cli, Command, KeyCommand};
use crate::output;
use std::io;

pub fn run(cli: Cli) -> anyhow::Result<()> {
    match cli.command {
        Command::Version => output::version(io::stdout().lock(), cli.json)?,
        command => {
            use crate::{repo::Repository, secret};
            let path = cli
                .repo
                .ok_or_else(|| anyhow::anyhow!("--repo is required"))?;
            let initializing = matches!(command, Command::Init);
            let backend = super::repo::select(&path, initializing, cli.verbose > 0)?;
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
                Command::Key { command } => {
                    let repo = Repository::open(backend, &secret)?;
                    match command {
                        KeyCommand::List => {
                            crate::ops::key::render_list(io::stdout().lock(), &repo, cli.json)?;
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
