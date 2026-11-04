#![forbid(unsafe_code)]
#![cfg_attr(not(test), warn(clippy::unwrap_used))]

use clap::{Parser, Subcommand};
use keelhaul::{
    exit::{self, Failure},
    output,
};
use std::{io, path::PathBuf, process::ExitCode};

#[derive(Debug, Parser)]
#[command(name = "keelhaul", version, about)]
struct Cli {
    #[arg(short = 'r', long, env = "KEELHAUL_REPO", global = true)]
    repo: Option<String>,
    #[arg(long, env = "KEELHAUL_PASSWORD_FILE", global = true)]
    password_file: Option<PathBuf>,
    #[arg(long, env = "KEELHAUL_PASSWORD_COMMAND", global = true)]
    password_command: Option<String>,
    #[arg(long, env = "KEELHAUL_KEY_FILE", global = true)]
    key_file: Option<PathBuf>,
    #[arg(long, global = true)]
    json: bool,
    #[arg(short, long, global = true)]
    quiet: bool,
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    verbose: u8,
    #[arg(long, default_value_t = 4, global = true, value_parser = clap::value_parser!(u32).range(1..))]
    jobs: u32,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Version,
    Init,
    Key {
        #[command(subcommand)]
        command: KeyCommand,
    },
}

#[derive(Debug, Subcommand)]
enum KeyCommand {
    List,
    Add {
        #[arg(long)]
        new_key_file: Option<PathBuf>,
    },
    Remove {
        id: String,
    },
}

fn run(cli: Cli) -> anyhow::Result<()> {
    match cli.command {
        Command::Version => output::version(io::stdout().lock(), cli.json)?,
        command => {
            use keelhaul::{backend::local::LocalBackend, repo::Repository, secret};
            let sources = usize::from(cli.password_file.is_some())
                + usize::from(cli.password_command.is_some())
                + usize::from(cli.key_file.is_some());
            if sources > 1 {
                return Err(secret::Error::ConflictingSources.into());
            }
            let path = cli
                .repo
                .ok_or_else(|| anyhow::anyhow!("--repo is required"))?;
            if path.starts_with("s3:") || path.starts_with("sftp:") {
                anyhow::bail!("this backend is not implemented yet");
            }
            let secret = secret::load(
                cli.password_file.as_deref(),
                cli.password_command.as_deref(),
                cli.key_file.as_deref(),
            )?;
            let backend = LocalBackend::new(path)?;
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
                            keelhaul::ops::key::render_list(io::stdout().lock(), &repo, cli.json)?;
                            return Ok(());
                        }
                        KeyCommand::Add { new_key_file } => {
                            let new_secret = match new_key_file {
                                Some(path) => secret::new_key_file(&path)?,
                                None => secret::new_password()?,
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

fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            let result = if error.use_stderr() {
                Err(Failure::Usage)
            } else {
                Ok(())
            };
            let _ = error.print();
            return ExitCode::from(exit::code(result));
        }
    };
    let result = run(cli).map_err(|error| {
        eprintln!("keelhaul: {error:#}");
        if error
            .downcast_ref::<keelhaul::secret::Error>()
            .is_some_and(|e| matches!(e, keelhaul::secret::Error::ConflictingSources))
        {
            Failure::Usage
        } else if error
            .downcast_ref::<keelhaul::repo::Error>()
            .is_some_and(|e| matches!(e, keelhaul::repo::Error::Damage { .. }))
        {
            Failure::Damage
        } else {
            Failure::Error
        }
    });
    ExitCode::from(exit::code(result))
}
