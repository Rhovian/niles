#![allow(clippy::unwrap_used, clippy::expect_used)]
mod common;
use common::*;
use std::{fs, path::PathBuf};

fn manifest(agent: &str) -> String {
    format!(
        "lead: {{standard: {{primary: claude:sonnet:high, secondaries: []}}}}\nworker: {{standard: {{primary: {agent}, secondaries: []}}}}\n"
    )
}

struct Setups {
    env: TestEnv,
    global: PathBuf,
}
impl Setups {
    fn new(label: &str) -> Self {
        let env = TestEnv::with_manifest(label, "claude");
        let global = env.home.join(".niles/setups/tdd.yaml");
        let directory = env.root.join("linked-setups");
        fs::create_dir_all(&directory).unwrap();
        fs::create_dir_all(env.home.join(".niles")).unwrap();
        std::os::unix::fs::symlink(&directory, global.parent().unwrap()).unwrap();
        let target = env.root.join("global.yaml");
        fs::write(&target, manifest("codex:gpt-6.1-sol:medium")).unwrap();
        std::os::unix::fs::symlink(target, &global).unwrap();
        Self { env, global }
    }
}

#[test]
fn invalid_setup_names_fail_at_the_cli_boundary() {
    let env = TestEnv::new("setup-names");
    for name in ["a/b", ".", "../x", "global:a:b", "global:", "builtin:tdd"] {
        let output = env.run(&["models", "--setup", name]);
        assert!(!output.status.success(), "{name}");
        assert!(
            stderr_of(&output).contains("invalid setup"),
            "{name}: {}",
            stderr_of(&output)
        );
    }
}

#[test]
fn env_selection_reads_global_links_and_never_merges_the_default_manifest() {
    let setups = Setups::new("setup-env");
    let default = setups.env.run(&["skills"]);
    assert_command_success("default manifest", &default);
    assert!(
        stdout_of(&default)
            .lines()
            .any(|line| line.starts_with("skill: research "))
    );
    let output = setups
        .env
        .niles(&setups.env.root, &["skills"])
        .env("NILES_SETUP", "tdd")
        .output()
        .unwrap();
    assert_command_success("env setup through directory and file links", &output);
    assert!(
        !stdout_of(&output)
            .lines()
            .any(|line| line.starts_with("skill: research "))
    );
}

#[test]
fn flag_selection_overrides_a_malformed_unused_environment_value() {
    let setups = Setups::new("setup-flag");
    let path = setups.env.root.join(".niles/setups/tdd.yaml");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, manifest("claude:haiku")).unwrap();
    let output = setups
        .env
        .niles(&setups.env.root, &["models", "--setup", "workspace:tdd"])
        .env("NILES_SETUP", "invalid:selection")
        .output()
        .unwrap();
    assert_command_success("flag wins over env", &output);
    assert!(stdout_of(&output).contains("haiku"));
}

#[test]
fn entry_to_spawn_keeps_qualified_selection_provenance_and_metadata_after_shadowing() {
    for entry in ["lead", "solo"] {
        assert_entry_setup(entry);
    }
}

fn assert_entry_setup(entry: &str) {
    let setups = Setups::new("setup-entry");
    let env = &setups.env;
    fs::write(
        &setups.global,
        format!(
            "entry: {entry}\n{entry}: {{standard: {{primary: claude:sonnet:high, secondaries: []}}}}\nworker: {{standard: {{primary: codex:gpt-6.1-sol:medium, secondaries: []}}, skills: [guide], always: [guide]}}\n"
        ),
    )
    .unwrap();
    let guide = env.root.join(".niles/skills/guide/SKILL.md");
    fs::create_dir_all(guide.parent().unwrap()).unwrap();
    fs::write(
        guide,
        "---\nname: guide\ndescription: Use when testing setups.\n---\nSETUP GUIDE BODY",
    )
    .unwrap();
    let override_file = env.root.join("override.yaml");
    fs::write(&override_file, manifest("claude:haiku")).unwrap();
    write_executable(
        &env.bin.join("claude"),
        "#!/bin/sh\nprintf '%s' \"${NILES_SETUP-unset}\" > \"$NILES_TEST_CAPTURE\"\nmkdir -p \"$NILES_TEST_WORKSPACE/.niles/setups\"\ncp \"$NILES_TEST_OVERRIDE\" \"$NILES_TEST_WORKSPACE/.niles/setups/tdd.yaml\"\n\"$NILES_TEST_BINARY\" spawn selected --skill worker task\n",
    );
    let capture = env.root.join("selection");
    let output = env
        .niles(&env.root, &["lead", "--keep-skills", "--setup", "tdd"])
        .env("NILES_TEST_CAPTURE", &capture)
        .env("NILES_TEST_WORKSPACE", &env.root)
        .env("NILES_TEST_OVERRIDE", &override_file)
        .env("NILES_TEST_BINARY", env!("CARGO_BIN_EXE_niles"))
        .env_remove("TMUX_PANE")
        .output()
        .unwrap();
    assert_command_success("entry with setup", &output);
    assert_eq!(fs::read_to_string(capture).unwrap(), "global:tdd");
    let provenance = format!("setup: tdd (global {})", setups.global.display());
    assert!(stderr_of(&output).contains(&provenance));
    assert!(stdout_of(&output).contains(&provenance));
    let worker = read_json(state_dir(&env.root).join("worker/selected/meta.json"));
    assert_eq!(
        worker["setup"],
        serde_json::json!({"name":"tdd","origin":"global"})
    );
    assert!(worker["agent"].as_str().unwrap().starts_with("codex:"));
    assert_eq!(worker["menu"], serde_json::json!(["guide"]));
    assert_eq!(worker["always"], serde_json::json!(["guide"]));
    let worker_dir = state_dir(&env.root).join("worker/selected");
    assert!(
        fs::read_to_string(worker_dir.join("brief.md"))
            .unwrap()
            .contains("guide: Use when testing setups.")
    );
    assert!(
        fs::read_to_string(worker_dir.join("launch.sh"))
            .unwrap()
            .contains("SETUP GUIDE BODY")
    );
    assert!(
        !fs::read_to_string(state_dir(&env.root).join("worker/selected/launch.sh"))
            .unwrap()
            .contains("NILES_SETUP=")
    );
    let session = fs::read_dir(state_dir(&env.root).join("sessions"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path()
        .join("session.json");
    assert_eq!(read_json(&session)["setup"], worker["setup"]);
    assert_eq!(read_json(&session)["entry_skill"], entry);
    assert!(
        fs::read_to_string(session.parent().unwrap().join("lead.md"))
            .unwrap()
            .contains(&format!("manifest: {}", setups.global.display()))
    );
}
