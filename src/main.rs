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
            let _ = error.print();
            return ExitCode::from(exit::code(result));
        }
    };
    let result = cli::run(cli).map_err(|error| {
        eprintln!("keelhaul: {error:#}");
        exit::failure(error.as_ref())
    });
    ExitCode::from(exit::code(result))
}
