use clap::CommandFactory;
#[test]
fn every_subcommand_has_a_description() {
    fn check(command: clap::Command) {
        for subcommand in command.get_subcommands() {
            assert!(
                subcommand.get_about().is_some(),
                "{}",
                subcommand.get_name()
            );
            check(subcommand.clone());
        }
    }
    check(keelhaul::cli::Cli::command());
}
