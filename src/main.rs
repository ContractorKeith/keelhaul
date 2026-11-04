#![forbid(unsafe_code)]
#![cfg_attr(not(test), warn(clippy::unwrap_used))]

use clap::Parser;
use keelhaul::{
    cli::{self, Cli},
    exit::{self, Failure},
};
use std::process::ExitCode;

fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            let result = if error.use_stderr() {
                Err(Failure::Usage)
            } else {
                Ok(())
            };
            if error.use_stderr() {
                let json = std::env::args_os().any(|arg| arg == "--json");
                exit::report(&error.to_string(), Failure::Usage, json);
            } else {
                let _ = error.print();
            }
            return ExitCode::from(exit::code(result));
        }
    };
    let json = cli.json;
    let result = cli::run(cli).map_err(|error| {
        let failure = exit::failure(error.as_ref());
        exit::report(&format!("{error:#}"), failure, json);
        failure
    });
    ExitCode::from(exit::code(result))
}
