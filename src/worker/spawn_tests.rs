use clap::Parser;

use super::*;
use crate::{
    cli::{Cli, CommandName},
    test_support::temp_test_path,
    workspace_manifest::{WorkspaceManifest, save},
};

fn resolve_from_cli(project: &Utf8Path, args: &[&str]) -> Result<String> {
    let cli = Cli::try_parse_from(args).unwrap();
    let Some(CommandName::Spawn {
        assignment, agent, ..
    }) = cli.command
    else {
        panic!("expected spawn");
    };
    let config = load_project_config_from(project)?;
    let path = workspace_manifest::manifest_path(project)?;
    let manifest = workspace_manifest::load(project)?;
    resolve_agent(
        &path,
        manifest.as_ref(),
        assignment.role,
        assignment.tier,
        agent,
        &config,
    )
}

#[test]
fn spawn_resolves_role_tier_primary_and_exact_secondary() {
    let root = temp_test_path("spawn-role-tiers");
    let manifest = WorkspaceManifest::default();
    save(&root, &manifest).unwrap();
    for role in crate::agents::picker::Role::ALL.into_iter().skip(1) {
        let expected = role.binding(&manifest).default_model();
        assert_eq!(
            resolve_from_cli(
                &root,
                &["niles", "spawn", "job", "--role", role.name(), "task"]
            )
            .unwrap(),
            expected
        );
    }
    assert_eq!(
        resolve_from_cli(
            &root,
            &["niles", "spawn", "job", "--tier", "economy", "task"]
        )
        .unwrap(),
        manifest.worker.economy.as_ref().unwrap().primary
    );
    let error = resolve_from_cli(
        &root,
        &[
            "niles", "spawn", "job", "--role", "design", "--tier", "economy", "task",
        ],
    )
    .unwrap_err();
    assert!(error.to_string().contains("does not list tier economy"));
    let secondary = &manifest.worker.standard.secondaries[0];
    assert_eq!(
        resolve_from_cli(
            &root,
            &["niles", "spawn", "job", "--agent", secondary, "task"]
        )
        .unwrap(),
        *secondary
    );
    for denied in ["claude:sonnet:medium", "claude:sonnet", "claude"] {
        assert!(
            resolve_from_cli(
                &root,
                &[
                    "niles", "spawn", "job", "--role", "design", "--agent", denied, "task"
                ]
            )
            .is_err()
        );
    }
    assert!(
        Cli::try_parse_from([
            "niles", "spawn", "job", "--tier", "frontier", "--agent", secondary, "task"
        ])
        .is_err()
    );
    crate::test_support::remove_workspace(&root).unwrap();
}

#[test]
fn omitted_agent_requires_a_manifest() {
    let root = temp_test_path("spawn-no-manifest");
    let error = resolve_from_cli(&root, &["niles", "spawn", "job", "task"]).unwrap_err();
    assert!(error.to_string().contains("does not exist"));
    assert_eq!(
        resolve_from_cli(
            &root,
            &["niles", "spawn", "job", "--agent", "claude", "task"]
        )
        .unwrap(),
        "claude"
    );
}

fn assignment(role: WorkerRole, design: Option<&str>, mechanical: Option<&str>) -> Assignment {
    Assignment {
        tier: agents::roster::Tier::Standard,
        role,
        design: design.map(str::to_owned),
        mechanical: mechanical.map(str::to_owned),
    }
}

#[test]
fn assignment_flags_refuse_invalid_combinations_and_reasons() {
    use WorkerRole::{Design, Research, Reviewer, Security, Worker};
    let root = temp_test_path("assignment-flags");
    for (role, design, mechanical, message) in [
        (Reviewer, None, Some("rename"), "--mechanical"),
        (Security, None, Some("rename"), "--mechanical"),
        (Design, Some("d"), None, "--design and --mechanical"),
        (Research, Some("d"), None, "--design and --mechanical"),
        (Design, None, Some("rename"), "--design and --mechanical"),
        (Research, None, Some("rename"), "--design and --mechanical"),
    ] {
        let err = assignment(role, design, mechanical)
            .resolve(&root, |_| panic!("invalid flags must not resolve a worker"))
            .unwrap_err();
        assert!(err.to_string().contains(message), "{err}");
    }
    for reason in ["", "   ", "line\nforged", "tab\t", "escape\u{1b}"] {
        let err = assignment(Worker, None, Some(reason))
            .resolve(&root, |_| Ok(None))
            .unwrap_err();
        assert!(err.to_string().contains("--mechanical reason"), "{err}");
    }
    for role in [Worker, Reviewer, Security, Design, Research] {
        assignment(role, None, None)
            .resolve(&root, |_| Ok(None))
            .unwrap();
    }
}

#[test]
fn design_requires_live_designer_latest_done() {
    use WorkerRole::{Design, Reviewer, Security, Worker};
    let root = temp_test_path("assignment-design");
    let request = |role| assignment(role, Some("designer"), None);
    let err = request(Worker).resolve(&root, |_| Ok(None)).unwrap_err();
    assert!(
        err.to_string()
            .contains("no such live design worker 'designer'")
    );
    fs::create_dir_all(&root).unwrap();
    let not_done = "latest status is not done:";
    for (role, status, message) in [
        (Reviewer, "done: ready\n", "not a design worker"),
        (Design, "", not_done),
        (Design, "done: ready\nworking: amending\n", not_done),
    ] {
        let meta = serde_json::json!({
            "id": "designer", "role": role, "agent": "claude", "created_at": Utc::now(),
            "project": root, "window": "@1", "brief": root.join("brief.md"),
            "launch": root.join("launch.sh")
        });
        store::write_json(&root.join("meta.json"), &meta).unwrap();
        fs::write(wake::status_log_path(&root), status).unwrap();
        let result = request(Worker).resolve(&root, |_| Ok(Some(root.clone())));
        let err = result.unwrap_err().to_string();
        assert!(err.contains("designer") && err.contains(message), "{err}");
    }
    fs::write(wake::status_log_path(&root), "done: ready\n").unwrap();
    for role in [Reviewer, Security] {
        request(role)
            .resolve(&root, |_| Ok(Some(root.clone())))
            .unwrap();
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn mechanical_requires_a_lead_session() {
    let root = temp_test_path("assignment-mechanical");
    let err = assignment(WorkerRole::Worker, None, Some("rename"))
        .resolve(&root, |_| Ok(None))
        .unwrap_err();
    assert!(err.to_string().contains("requires a lead session"), "{err}");
}
