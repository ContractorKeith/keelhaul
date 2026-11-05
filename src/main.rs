#![forbid(unsafe_code)]
#![cfg_attr(not(test), warn(clippy::unwrap_used))]

use clap::Parser;
use keelhaul::{
    cli::{self, Cli},
    exit::{self, Failure},
};
use std::process::ExitCode;

fn main() -> ExitCode {
    // exec with a cleaned environment instead of mutating process-global state.
    // This also covers backend-specific variables without duplicating their parsers.
    let empty = std::env::vars_os()
        .filter(|(name, value)| {
            name.to_str()
                .is_some_and(|name| name.starts_with("KEELHAUL_"))
                && value.is_empty()
        })
        .map(|(name, _)| name)
        .collect::<Vec<_>>();
    if !empty.is_empty() {
        use std::os::unix::process::CommandExt;
        let error = match std::env::current_exe() {
            Ok(executable) => {
                let mut command = std::process::Command::new(executable);
                command.args(std::env::args_os().skip(1));
                for name in empty {
                    command.env_remove(name);
                }
                command.exec()
            }
            Err(error) => error,
        };
        exit::report(
            &format!("cannot start with cleaned environment: {error}"),
            Failure::Error,
            false,
        );
        return ExitCode::from(1);
    }
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
