// This scope includes the legacy backup renderer without changing its source.
macro_rules! println {
    ($($arg:tt)*) => { crate::output::line(format_args!($($arg)*))? };
}
mod audit;
mod backup;
mod commands;
mod repo;
mod secrets;

use clap::{Parser, Subcommand};
pub use commands::run;
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(
    name = "keelhaul",
    version,
    about,
    after_help = "SSH environment:
  KEELHAUL_SSH_ARGS     Shell-style extra ssh arguments. Defaults: ServerAliveInterval=15, ServerAliveCountMax=4 (dead links exit after about 60 seconds), BatchMode=yes (no password prompts). Explicit -o arguments come first and override these defaults.
  KEELHAUL_SSH_COMMAND  Executable to use instead of ssh."
)]
pub struct Cli {
    /// Repository path, sftp:[user@]host:/path, or s3:https://host/bucket[/prefix].
    /// S3 bucket is the first path segment; endpoint path prefixes are unsupported.
    /// S3: KEELHAUL_S3_TIMEOUT_SECS bounds each attempt (default 20);
    /// KEELHAUL_S3_DEADLINE_SECS bounds an operation across retries (default 120). Both accept 1–3600 seconds.
    #[arg(short = 'r', long, env = "KEELHAUL_REPO", global = true)]
    repo: Option<String>,
    #[arg(long, hide_env_values = true, global = true)]
    password_file: Option<PathBuf>,
    #[arg(
        long,
        help = "Read password from sh -c COMMAND (stdout at most 1 MiB); command arguments are visible in ps",
        hide_env_values = true,
        global = true
    )]
    password_command: Option<String>,
    #[arg(long, hide_env_values = true, global = true)]
    key_file: Option<PathBuf>,
    #[arg(long, global = true)]
    pub json: bool,
    #[arg(short, long, global = true)]
    quiet: bool,
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    verbose: u8,
    /// Workers (1–256). File descriptors: about 2 per job plus repository/SSH overhead; lower jobs or raise ulimit -n if exhausted.
    /// Memory: jobs × (8 MiB + 2 × pack_target), plus index and tree metadata.
    #[arg(long, default_value_t = 4, global = true, value_parser = clap::value_parser!(u32).range(1..=256))]
    jobs: u32,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
pub(super) enum Command {
    #[command(about = "Show the installed version")]
    Version,
    #[command(about = "Check repository structure and references")]
    Check,
    #[command(about = "Read and verify every stored byte")]
    Verify {
        #[arg(long)]
        subset: Option<crate::ops::verify::Subset>,
    },
    #[command(about = "Back up files and directories")]
    Backup {
        #[arg(required = true)]
        paths: Vec<PathBuf>,
        #[arg(long)]
        tag: Vec<String>,
        #[arg(long)]
        host: Option<String>,
        #[arg(long)]
        parent: Option<String>,
        #[arg(long)]
        force_rehash: bool,
        /// Verify stored chunks before deduplicating and replace damaged copies; implies --force-rehash.
        #[arg(long)]
        heal: bool,
        #[arg(long)]
        exclude: Vec<String>,
        #[arg(long)]
        exclude_file: Option<PathBuf>,
        #[arg(long)]
        one_file_system: bool,
    },
    #[command(about = "List snapshots")]
    Snapshots {
        #[arg(long)]
        host: Option<String>,
        #[arg(long)]
        tag: Option<String>,
        #[arg(long)]
        path: Option<String>,
    },
    #[command(
        about = "List a snapshot; SNAP is a unique hex prefix (at least 8 characters) or latest (newest for this host; --host H selects another host)"
    )]
    Ls {
        snap: String,
        path: Option<PathBuf>,
        #[arg(long)]
        host: Option<String>,
    },
    #[command(
        about = "Restore beneath TARGET with original absolute paths relative to it; SNAP is a unique hex prefix (at least 8 characters) or latest (newest for this host; --host H selects another host)"
    )]
    Restore {
        snap: String,
        #[arg(long)]
        target: PathBuf,
        #[arg(long)]
        include: Vec<PathBuf>,
        #[arg(long)]
        host: Option<String>,
    },
    #[command(about = "Remove snapshots by ID or retention policy")]
    Forget(crate::ops::forget::Options),
    #[command(about = "Reclaim unreferenced data and repack sparse packs")]
    Prune {
        #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u8).range(0..=100))]
        max_unused: u8,
        #[arg(long)]
        dry_run: bool,
    },
    #[command(about = "Remove stale or dead repository locks")]
    Unlock,
    /// Rebuild cached indexes from every pack header.
    RebuildIndex {
        #[arg(long)]
        dry_run: bool,
    },
    #[command(about = "Create an encrypted repository")]
    Init,
    #[command(about = "Manage repository key slots")]
    Key {
        #[command(subcommand)]
        command: KeyCommand,
    },
}

#[derive(Debug, Subcommand)]
pub(super) enum KeyCommand {
    #[command(about = "List repository key slots")]
    List,
    #[command(about = "Add a password or key-file slot")]
    Add {
        #[arg(long, conflicts_with = "new_password_file")]
        new_key_file: Option<PathBuf>,
        #[arg(long, env = "KEELHAUL_NEW_PASSWORD_FILE", hide_env_values = true)]
        new_password_file: Option<PathBuf>,
    },
    #[command(about = "Remove a key slot")]
    Remove { id: String },
}
