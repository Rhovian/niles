use super::*;
use crate::cli::Cli;
use crate::store::atomic_write;
use crate::test_support::{assert_err_contains as assert_error, temp_test_path};
use clap::Parser;

#[test]
fn skill_is_required_and_task_basis_flags_conflict() {
    for args in [
        "task",
        "--skill worker --tier frontier --agent claude task",
        "--skill worker --from a --mechanical rename task",
    ] {
        let argv = ["niles", "spawn", "job"]
            .into_iter()
            .chain(args.split_whitespace());
        assert!(Cli::try_parse_from(argv).is_err());
    }
}

fn assignment(from: Option<&str>, mechanical: Option<&str>) -> Assignment {
    Assignment {
        menu: Vec::new(),
        with: Vec::new(),
        tier: agents::roster::Tier::Standard,
        skill: "custom".parse().unwrap(),
        from: from.map(str::to_owned),
        mechanical: mechanical.map(str::to_owned),
    }
}

#[test]
fn from_accepts_any_skill_but_requires_live_latest_done() {
    let root = temp_test_path("assignment-from");
    let request = || assignment(Some("source"), None);
    assert_error(
        request().resolve(&root, |_| Ok(None)),
        "no such live worker",
    );
    fs::create_dir_all(&root).unwrap();
    for status in ["", "done: ready\nworking: amending\n"] {
        atomic_write(wake::status_log_path(&root), status).unwrap();
        assert_error(
            request().resolve(&root, |_| Ok(Some(root.clone()))),
            "latest status is not done:",
        );
    }
    atomic_write(wake::status_log_path(&root), "done: ready\n").unwrap();
    assert!(
        matches!(request().resolve(&root, |_| Ok(Some(root.clone()))).unwrap(), TaskBasis::FromReport(path) if path == report_path(&root))
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn mechanical_any_skill_requires_a_lead_and_an_inert_reason() {
    let root = temp_test_path("assignment-mechanical");
    assert_error(
        assignment(None, Some("rename")).resolve(&root, |_| Ok(None)),
        "requires a lead session",
    );
    assert!(matches!(
        assignment(None, None).resolve(&root, |_| Ok(None)).unwrap(),
        TaskBasis::None
    ));
    for reason in ["", "   ", "line\nforged", "tab\t", "escape\u{1b}"] {
        assert_error(
            assignment(None, Some(reason)).resolve(&root, |_| Ok(None)),
            "--mechanical reason",
        );
    }
}
