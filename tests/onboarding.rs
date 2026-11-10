#![allow(clippy::unwrap_used, clippy::expect_used, reason = "test assertions")]
use crate::common::{atomic_write, json_field};
mod common;
#[path = "onboarding/retry.rs"]
mod retry;
#[path = "onboarding/support.rs"]
mod support;
#[path = "onboarding/unattended.rs"]
mod unattended;
use common::register_project;
use std::fs;
use support::{environment, manifest, terminal};
const SOLO_DEFAULTS: &[(&str, &[u8])] = &[
    ("work?", b"\x1b[B\r"),
    ("TIERS", b"\r"),
    ("Optional", b"\r"),
    ("Warning:", b"\r"),
];
const CUSTOM_URL: &[(&str, &[u8])] = &[
    ("work?", b"\r"),
    ("lead's", b"\x1b[B\r"),
    ("URL", b"github.com/other/repository/skills/custom\r"),
    ("TIERS", b"\r"),
    ("starter", b"\r"),
    ("Warning:", b"\r"),
];

#[test]
fn first_run_registration_confirms_and_can_cancel() {
    for (answer, registers) in [(b"n".as_slice(), false), (b"\x1b", false), (b"y", true)] {
        let env = environment();
        let (output, _) = terminal(
            &env,
            &[],
            &[("directory:", b"\r"), ("]:", b"\r"), ("Register?", answer)],
            |_| {},
        );
        assert!(output.status.success(), "{output:?}");
        assert_eq!(env.home.join(".niles/projects").exists(), registers);
        if registers {
            assert!(env.tmux_log().contains(" start ;"));
        }
    }
}
#[test]
fn wizard_solo_accepts_defaults_without_optional_imports_or_editor() {
    let env = environment();
    for name in ["design", "reviewer", "worker", "security", "research"] {
        fs::remove_dir_all(env.home.join(".niles/skills").join(name)).unwrap();
    }
    let (output, text) = terminal(&env, &["start"], SOLO_DEFAULTS, |_| {});
    assert!(output.status.success(), "{output:?} {text}");
    let manifest = fs::read_to_string(manifest(&env)).unwrap();
    assert!(manifest.contains("entry: solo"), "{manifest}");
    assert!(!text.contains("lead's"));
    assert!(text.contains("docs/security.md"));
}
#[test]
fn missing_entry_fetch_failure_stops_before_manifest() {
    let env = environment();
    fs::remove_file(env.home.join(".niles/skills/solo/SKILL.md")).unwrap();
    let (output, _) = terminal(&env, &["start"], SOLO_DEFAULTS, |_| {});
    assert!(!output.status.success());
    assert!(!manifest(&env).exists());
    let output = env.run(&unattended::args());
    common::assert_failure_contains("explicit setup fetch failure", &output, "unexpected import");
    assert!(!manifest(&env).exists());
}

#[test]
fn existing_registry_does_not_start_first_run_registration() {
    let env = environment();
    register_project(&env, "existing", &env.root);
    let (output, text) = terminal(&env, &[], &[], |_| {});
    assert!(output.status.success(), "{output:?}");
    assert!(!text.contains("register"));
}

#[test]
fn taken_registration_name_can_be_replaced_after_confirmation() {
    let env = environment();
    let registry = env.home.join(".niles/projects");
    let other = env.root.join("other");
    fs::create_dir(&other).unwrap();
    let (output, text) = terminal(
        &env,
        &[],
        &[
            ("directory:", b"\r"),
            ("]:", b"taken\r"),
            ("Register?", b"y"),
            ("taken.", b"replacement\r"),
            ("Register?", b"y"),
        ],
        |step| {
            if step == 2 {
                register_project(&env, "taken", &other);
            }
        },
    );
    assert!(output.status.success(), "{output:?} {text}");
    assert_eq!(
        fs::read_link(registry.join("replacement")).unwrap(),
        env.root
    );
    assert_eq!(fs::read_link(registry.join("taken")).unwrap(), other);
}

#[test]
fn lead_saves_secondary_edits_and_independent_entry_agent() {
    let env = environment();
    let (output, text) = terminal(
        &env,
        &["start"],
        &[
            ("work?", b"\r"),
            ("lead's", b"\r"),
            ("TIERS", b"a"),
            ("Editing", b"\r\r\x1b[B\r\r"),
            ("Optional", b"\r"),
            ("Warning:", b"\r"),
        ],
        |_| {},
    );
    assert!(output.status.success(), "{output:?} {text}");
    let value: serde_json::Value =
        serde_saphyr::from_str(&fs::read_to_string(manifest(&env)).unwrap()).unwrap();
    assert_eq!(json_field(&value, "/entry"), "lead");
    assert_eq!(
        json_field(&value, "/tiers/standard/primary"),
        "codex:gpt-6.1-sol:medium"
    );
    assert_eq!(
        json_field(&value, "/entry_agent"),
        "codex:gpt-6.1-sol:medium"
    );
    assert_eq!(
        value
            .pointer("/tiers/standard/secondaries")
            .unwrap()
            .as_array()
            .unwrap()
            .last()
            .unwrap(),
        "codex:gpt-6.1-sol:high"
    );
}

#[test]
fn selected_starter_is_fetched_at_head_and_recorded_by_sha() {
    let env = environment();
    fs::remove_dir_all(env.home.join(".niles/skills/design")).unwrap();
    support::import_fixture(
        &env,
        "design",
        "---\nname: design\ndescription: Plans changes\n---\nPlan the work.\n",
    );
    let (output, text) = terminal(
        &env,
        &["start"],
        &[
            ("work?", b"\x1b[B\r"),
            ("TIERS", b"\r"),
            ("Optional", b"\x1b[B \r"),
            ("Warning:", b"\r"),
        ],
        |step| {
            if step == 3 {
                assert!(!env.home.join("git.log").exists());
            }
        },
    );
    assert!(output.status.success(), "{output:?} {text}");
    let sha = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    assert_eq!(
        fs::read_to_string(env.home.join(".niles/skills/design/SKILL.ref")).unwrap(),
        format!("github.com/Rhovian/niles/skills/design@{sha}\n")
    );
    let log = fs::read_to_string(env.home.join("git.log")).unwrap();
    assert!(
        log.contains("ls-remote -- https://github.com/Rhovian/niles.git HEAD "),
        "{log}"
    );
    assert!(
        log.contains(&format!(
            "fetch --depth 1 --filter=blob:none -- https://github.com/Rhovian/niles.git {sha}"
        )),
        "{log}"
    );
    assert!(
        env.home
            .join(format!(
                ".niles/cache/skills/github.com/Rhovian/niles/{sha}/skills/design/@skill/SKILL.md"
            ))
            .exists()
    );
}

#[test]
fn concurrent_manifest_is_kept() {
    let env = environment();
    let winner = common::setup_manifest_body();
    let (output, text) = terminal(&env, &["start"], SOLO_DEFAULTS, |step| {
        if step == 3 {
            fs::create_dir_all(manifest(&env).parent().unwrap()).unwrap();
            atomic_write(manifest(&env), winner).unwrap();
        }
    });
    assert!(output.status.success(), "{output:?} {text}");
    assert_eq!(fs::read_to_string(manifest(&env)).unwrap(), winner);
}

#[test]
fn lead_picker_always_offers_custom_and_reviews_default_entry() {
    for (removed, optional) in [("skills/design", b" \r".as_slice()), ("skills", b"\r")] {
        let env = environment();
        fs::remove_dir_all(env.home.join(".niles").join(removed)).unwrap();
        let (output, text) = terminal(
            &env,
            &["start"],
            &[
                ("work?", b"\r"),
                ("lead's", b"\r"),
                ("TIERS", b"\r"),
                ("Optional", optional),
                ("Warning:", b"q"),
            ],
            |_| {},
        );
        assert!(output.status.success(), "{output:?} {text}");
        assert!(text.contains("lead's"));
        assert!(text.contains("Custom"));
        let summary = text.rsplit_once("Warning:").unwrap().1;
        assert!(summary.contains("lead"), "{summary}");
        assert!(!manifest(&env).exists());
    }
}

#[test]
fn back_walk_preserves_edits_and_checks_and_skips_lead_for_solo() {
    let env = environment();
    fs::remove_dir_all(env.home.join(".niles/skills/design")).unwrap();
    support::import_fixture(
        &env,
        "design",
        "---\nname: design\ndescription: Design\n---\nPlan work.\n",
    );
    let (output, text) = terminal(
        &env,
        &["start"],
        &[
            ("work?", b"\r"),
            ("lead's", b"\r"),
            ("TIERS", b"a"),
            ("Editing", b"\r\r\x1b[B\r\r"),
            ("starter", b" \r"),
            ("Warning:", b"\x1b"),
            ("starter", b"\x1b"),
            ("TIERS", b"\x1b"),
            ("lead's", b"\x1b"),
            ("work?", b"\x1b[B\r"),
            ("TIERS", b"\x1b"),
            ("work?", b"\r"),
            ("TIERS", b"\r"),
            ("starter", b"\r"),
            ("Warning:", b"\r"),
        ],
        |_| {
            assert!(!manifest(&env).exists());
            assert!(!env.home.join("git.log").exists());
        },
    );
    assert!(output.status.success(), "{output:?} {text}");
    let value: serde_json::Value =
        serde_saphyr::from_str(&fs::read_to_string(manifest(&env)).unwrap()).unwrap();
    assert_eq!(json_field(&value, "/entry"), "solo");
    assert_eq!(
        value
            .pointer("/tiers/standard/secondaries")
            .unwrap()
            .as_array()
            .unwrap()
            .last()
            .unwrap(),
        "codex:gpt-6.1-sol:high"
    );
    assert!(env.home.join(".niles/skills/design/SKILL.ref").exists());
}

#[test]
fn custom_input_errors_stay_on_input_and_collisions_never_fetch() {
    let env = environment();
    let before = fs::read(env.home.join(".niles/skills/lead/SKILL.md")).unwrap();
    let mut collision = vec![0x7f; "gitlab.com/o/r/lead".len()];
    collision.extend_from_slice(b"github.com/o/r/lead\r");
    let (output, text) = terminal(
        &env,
        &["start"],
        &[
            ("work?", b"\r"),
            ("lead's", b"\x1b[B\r"),
            ("URL", b"gitlab.com/o/r/lead\r"),
            ("only", &collision),
            ("already", b"\x1b[21~"),
        ],
        |_| {},
    );
    assert!(output.status.success(), "{output:?} {text}");
    assert_eq!(
        fs::read(env.home.join(".niles/skills/lead/SKILL.md")).unwrap(),
        before
    );
    assert!(!manifest(&env).exists());
    assert!(!env.home.join("git.log").exists());
}

#[test]
fn custom_lead_imports_its_own_repository_only_after_confirmation() {
    for body in ["Coordinate the work.", ""] {
        let env = environment();
        support::import_fixture(
            &env,
            "custom",
            &format!("---\nname: custom\ndescription: Custom lead\n---\n{body}\n"),
        );
        let (output, text) = terminal(&env, &["start"], CUSTOM_URL, |_| {
            assert!(!env.home.join("git.log").exists());
        });
        assert_eq!(
            output.status.success(),
            !body.is_empty(),
            "{output:?} {text}"
        );
        assert_eq!(manifest(&env).exists(), !body.is_empty());
        assert!(text.contains("other/repository"), "{text}");
        let pointer = env.home.join(".niles/skills/custom/SKILL.ref");
        assert_eq!(pointer.exists(), !body.is_empty());
        if !body.is_empty() {
            assert_eq!(
                fs::read_to_string(pointer).unwrap(),
                "github.com/other/repository/skills/custom@aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n"
            );
        }
        let log = fs::read_to_string(env.home.join("git.log")).unwrap();
        assert!(log.contains("https://github.com/other/repository.git"));
        assert!(!log.contains("Rhovian/niles"));
        if !body.is_empty() {
            assert!(
                fs::read_to_string(manifest(&env))
                    .unwrap()
                    .contains("entry: custom")
            );
        }
    }
}

#[test]
fn custom_back_keeps_text_and_switching_to_solo_fetches_the_selected_source() {
    let env = environment();
    fs::remove_dir_all(env.home.join(".niles/skills/solo")).unwrap();
    support::import_fixture(
        &env,
        "solo",
        "---\nname: solo\ndescription: Solo\n---\nDo the work.\n",
    );
    let (output, text) = terminal(
        &env,
        &["start"],
        &[
            ("work?", b"\r"),
            ("lead's", b"\x1b[B\r"),
            ("URL", b"github.com/other/repository/skills/solo\x1b"),
            ("lead's", b"\r"),
            ("URL", b"\r"),
            ("TIERS", b"\x1b"),
            ("URL", b"\x1b"),
            ("lead's", b"\x1b"),
            ("work?", b"\x1b[B\r"),
            ("TIERS", b"\r"),
            ("starter", b"\r"),
            ("Warning:", b"\r"),
        ],
        |_| {
            assert!(!env.home.join("git.log").exists());
        },
    );
    assert!(output.status.success(), "{output:?} {text}");
    let log = fs::read_to_string(env.home.join("git.log")).unwrap();
    assert!(log.contains("https://github.com/Rhovian/niles.git"));
    assert!(!log.contains("other/repository"));
    let summary = text.rsplit_once("Warning:").unwrap().1;
    assert!(!summary.contains("Custom"));
    assert!(
        fs::read_to_string(manifest(&env))
            .unwrap()
            .contains("entry: solo")
    );
}
