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
}

fn run(cli: Cli) -> anyhow::Result<()> {
    match cli.command {
        Command::Version => output::version(io::stdout().lock(), cli.json)?,
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
        Failure::Error
    });
    ExitCode::from(exit::code(result))
}
