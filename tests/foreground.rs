#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::*;
use std::fs;

#[test]
fn lead_delivers_primary_and_session_header_to_builtin_cli() {
    let env = TestEnv::new("niles-foreground-header");
    write_workspace_manifest(&env.root, "claude:haiku", "codex", "claude", "codex");
    write_executable(
        &env.bin.join("claude"),
        "#!/bin/sh\nprintf '%s\\0' \"$@\" > \"$NILES_TEST_ARGS\"\n",
    );
    let args_log = env.root.join("args");
    let output = env
        .niles(&env.root, &["lead", "--keep-roles"])
        .env("NILES_TEST_ARGS", &args_log)
        .env_remove("TMUX_PANE")
        .output()
        .unwrap();
    assert_command_success("lead", &output);
    let args = fs::read_to_string(args_log).unwrap();
    let args = args
        .strip_suffix('\0')
        .unwrap()
        .split('\0')
        .collect::<Vec<_>>();
    let turn = args.last().unwrap();
    let dir = turn
        .strip_prefix("# Niles lead session\nsession_dir: ")
        .unwrap()
        .strip_suffix("\n\nStart the Niles manager session.")
        .unwrap();
    let primary = fs::read_to_string(std::path::Path::new(dir).join("lead.md")).unwrap();
    assert!(
        args.windows(2)
            .any(|pair| pair == ["--append-system-prompt", primary.as_str()])
    );
    assert!(args.windows(2).any(|pair| pair == ["--model", "haiku"]));
}
