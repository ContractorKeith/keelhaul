//! Secret flags take precedence over the entire group of environment sources.
use super::Cli;
use std::path::PathBuf;
pub(super) fn resolve(cli: &mut Cli) {
    if cli.password_file.is_some() || cli.password_command.is_some() || cli.key_file.is_some() {
        return;
    }
    cli.password_file = std::env::var_os("KEELHAUL_PASSWORD_FILE").map(PathBuf::from);
    cli.password_command = std::env::var("KEELHAUL_PASSWORD_COMMAND").ok();
    cli.key_file = std::env::var_os("KEELHAUL_KEY_FILE").map(PathBuf::from);
}
