mod commands;
mod repo;

use clap::{Parser, Subcommand};
pub use commands::run;
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(name = "keelhaul", version, about)]
pub struct Cli {
    /// Repository path, sftp:[user@]host:/path, or s3:https://host/bucket[/prefix].
    /// S3 bucket is the first path segment; endpoint path prefixes are unsupported.
    #[arg(short = 'r', long, env = "KEELHAUL_REPO", global = true)]
    repo: Option<String>,
    #[arg(
        long,
        env = "KEELHAUL_PASSWORD_FILE",
        hide_env_values = true,
        global = true
    )]
    password_file: Option<PathBuf>,
    #[arg(
        long,
        env = "KEELHAUL_PASSWORD_COMMAND",
        help = "Read password from sh -c COMMAND (stdout at most 1 MiB); command arguments are visible in ps",
        hide_env_values = true,
        global = true
    )]
    password_command: Option<String>,
    #[arg(long, env = "KEELHAUL_KEY_FILE", hide_env_values = true, global = true)]
    key_file: Option<PathBuf>,
    #[arg(long, global = true)]
    pub json: bool,
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
pub(super) enum Command {
    Version,
    Init,
    Key {
        #[command(subcommand)]
        command: KeyCommand,
    },
}

#[derive(Debug, Subcommand)]
pub(super) enum KeyCommand {
    List,
    Add {
        #[arg(long, conflicts_with = "new_password_file")]
        new_key_file: Option<PathBuf>,
        #[arg(long, env = "KEELHAUL_NEW_PASSWORD_FILE", hide_env_values = true)]
        new_password_file: Option<PathBuf>,
    },
    Remove {
        id: String,
    },
}
