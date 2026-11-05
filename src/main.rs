#![forbid(unsafe_code)]
#![cfg_attr(not(test), warn(clippy::unwrap_used))]

use clap::Parser;
use keelhaul::{
    cli::{self, Cli},
    exit::{self, Failure},
};
use std::process::ExitCode;

fn main() -> ExitCode {
    // time's local-zone lookup must precede SSH readers and backup workers.
    keelhaul::ops::forget::local_offset();
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
                let _ = keelhaul::output::line(format_args!("{}", error.to_string().trim_end()));
            }
            return ExitCode::from(exit::code(result));
        }
    };
    let json = cli.json;
    let result = match cli::run(cli) {
        Ok(()) => Ok(()),
        Err(error) if keelhaul::output::broken_pipe(error.as_ref()) => Ok(()),
        Err(error) => {
            let failure = exit::failure(error.as_ref());
            exit::report(&error.to_string(), failure, json);
            Err(failure)
        }
    };
    ExitCode::from(exit::code(result))
}
