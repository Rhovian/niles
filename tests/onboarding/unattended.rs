use super::{environment, manifest, support};
use crate::common::{atomic_write, *};
use std::fs;
use std::sync::LazyLock;

static FIXTURE: LazyLock<serde_json::Value> =
    LazyLock::new(|| serde_saphyr::from_str(setup_manifest_body()).unwrap());
static TIERS: LazyLock<String> =
    LazyLock::new(|| serde_saphyr::to_string(&json_field(&FIXTURE, "/tiers")).unwrap());
pub(super) fn args() -> Vec<&'static str> {
    vec![
        "setup",
        "--entry-skill",
        "solo",
        "--entry-agent",
        json_field(&FIXTURE, "/entry_agent").as_str().unwrap(),
        "--tiers",
        &TIERS,
        "--start",
        "installed",
    ]
}

fn fetch_fixture(env: &TestEnv, name: &str) {
    let dir = env.home.join(".niles/skills").join(name);
    let body = fs::read_to_string(dir.join("SKILL.md")).unwrap();
    fs::remove_dir_all(dir).unwrap();
    support::import_fixture(env, name, &body);
}

#[test]
fn explicit_setup_fetches_missing_skills_preserves_choices_and_starts_detached() {
    let env = environment();
    fetch_fixture(&env, "solo");
    let mut args = args();
    args.extend(["--starter-skills", "solo,cost-analysis"]);
    let output = env.run(&args);
    assert_success_contains("explicit setup", &output, "entry session: installed");
    let body = fs::read_to_string(manifest(&env)).unwrap();
    let value: serde_json::Value = serde_saphyr::from_str(&body).unwrap();
    assert_eq!(json_field(&value, "/entry"), "solo");
    assert_eq!(
        json_field(&value, "/entry_agent"),
        json_field(&FIXTURE, "/entry_agent")
    );
    assert_eq!(json_field(&value, "/tiers"), json_field(&FIXTURE, "/tiers"));
    assert!(env.home.join(".niles/skills/solo/SKILL.ref").exists());
    assert_eq!(
        fs::read_link(env.home.join(".niles/projects/installed")).unwrap(),
        env.root
    );
    assert!(env.tmux_log().contains("new-session -d -s installed"));
    assert!(env.tmux_log().contains("start --keep-skills"));
    fs::remove_file(env.home.join("git.log")).unwrap();
    fs::remove_dir_all(env.home.join(".niles/skills/solo")).unwrap();
    let output = env.run(&args);
    assert_failure_contains("overwrite", &output, "manifest already exists");
    assert!(!env.home.join("git.log").exists());
}

#[test]
fn explicit_setup_rejects_invalid_input_before_writes_or_fetching() {
    let env = environment();
    fetch_fixture(&env, "design");
    for (flag, value, message) in [
        ("--entry-skill", "sloo", "unknown skill sloo"),
        ("--starter-skills", "design,workre", "fetchable:"),
        ("--entry-agent", "claude:opus:medium", "not listed"),
        ("--tiers", "{}", "missing field `standard`"),
        ("--start", "../bad", "project name must use"),
    ] {
        let mut args = args();
        args.extend(["--starter-skills", "design"]);
        if let Some([_, target]) = args
            .split_first_mut()
            .unwrap()
            .1
            .chunks_exact_mut(2)
            .find(|pair| pair.first() == Some(&flag))
        {
            *target = value;
        } else {
            args.extend([flag, value]);
        }
        assert_failure_contains(flag, &env.run(&args), message);
        assert!(!manifest(&env).exists());
    }
    assert!(!env.home.join("git.log").exists());
    for (missing, selection, message) in [
        (None, Some("other"), "does not accept"),
        (Some("tmux"), None, "CLI not found on PATH: tmux"),
        (Some("codex"), None, "CLI not found on PATH: codex"),
    ] {
        let env = environment();
        if let Some(binary) = missing {
            fs::remove_file(env.bin.join(binary)).unwrap();
        }
        let output = env
            .niles(&env.root, &args())
            .env("PATH", &env.bin)
            .envs(selection.map(|value| ("NILES_SETUP", value)))
            .output()
            .unwrap();
        assert_failure_contains("environment", &output, message);
        assert!(!manifest(&env).exists());
    }
}

#[test]
fn explicit_setup_respects_registration_by_name_and_directory() {
    for (existing_name, same_directory, expected) in [
        ("installed", true, None),
        ("old", true, Some("directory is already registered as old")),
        (
            "installed",
            false,
            Some("name is already registered elsewhere"),
        ),
    ] {
        let env = environment();
        let other = env.root.join("other");
        fs::create_dir(&other).unwrap();
        register_project(
            &env,
            existing_name,
            if same_directory { &env.root } else { &other },
        );
        let output = env.run(&args());
        if let Some(message) = expected {
            assert_failure_contains("registration conflict", &output, message);
            assert!(!manifest(&env).exists());
        } else {
            assert_success_contains("existing registration", &output, "entry session: installed");
        }
    }
}

#[test]
fn explicit_setup_refuses_manifest_created_during_skill_fetch() {
    let env = environment();
    fetch_fixture(&env, "solo");
    let git = env.bin.join("git");
    let original = fs::read_to_string(&git).unwrap();
    let hook = "cp \"$NILES_TEST_WINNER\" \"$NILES_TEST_MANIFEST\"";
    let script = original.replacen('\n', &format!("\n{hook}\n"), 1);
    write_executable(&git, &script);
    fs::create_dir_all(manifest(&env).parent().unwrap()).unwrap();
    let winner = env.root.join("winner.yaml");
    atomic_write(&winner, setup_manifest_body()).unwrap();
    let output = env
        .niles(&env.root, &args())
        .env("NILES_TEST_WINNER", winner)
        .env("NILES_TEST_MANIFEST", manifest(&env))
        .output()
        .unwrap();
    assert_failure_contains("concurrent manifest", &output, "manifest already exists");
}
