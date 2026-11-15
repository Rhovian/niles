use crate::cli::Cli;
use clap::Parser;

#[test]
fn skill_is_required_and_tier_conflicts_with_agent() {
    for args in ["task", "--skill worker --tier frontier --agent claude task"] {
        let argv = ["niles", "spawn", "job"]
            .into_iter()
            .chain(args.split_whitespace());
        assert!(Cli::try_parse_from(argv).is_err());
    }
}
