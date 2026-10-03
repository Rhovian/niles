use clap::Parser;

use super::*;
use crate::{
    cli::{Cli, CommandName},
    test_support::temp_test_path,
    workspace_manifest::{WorkspaceManifest, manifest_path, save},
};

fn resolve_from_cli(project: &Utf8Path, args: &[&str]) -> Result<String> {
    let cli = Cli::try_parse_from(args).unwrap();
    let Some(CommandName::Spawn { role, agent, .. }) = cli.command else {
        panic!("expected spawn");
    };
    resolve_agent(project, role, agent, &load_project_config_from(project)?)
}

fn manifest() -> WorkspaceManifest {
    WorkspaceManifest {
        lead: "leadbot".into(),
        worker: "codex:gpt-5.5:xhigh".to_owned().into(),
        reviewer: crate::workspace_manifest::ReviewerBinding::Agent(
            "claude:opus:high".to_owned().into(),
        ),
        security: "auditbot".to_owned().into(),
        ..WorkspaceManifest::default()
    }
}

#[test]
fn omitted_agent_uses_each_roles_manifest_binding() {
    let root = temp_test_path("spawn-role-agents");
    let manifest = manifest();
    save(&root, &manifest).unwrap();
    for (role, expected) in [
        ("worker", "codex:gpt-5.5:xhigh"),
        ("reviewer", "claude:opus:high"),
        ("security", "auditbot"),
    ] {
        let agent =
            resolve_from_cli(&root, &["niles", "spawn", "job", "--role", role, "task"]).unwrap();
        assert_eq!(agent, expected);
    }
    assert_eq!(
        resolve_from_cli(&root, &["niles", "spawn", "job", "task"]).unwrap(),
        "codex:gpt-5.5:xhigh"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn explicit_agent_requires_membership_when_manifest_exists() {
    let root = temp_test_path("spawn-explicit-agent");
    let args = [
        "niles", "spawn", "job", "--role", "reviewer", "--agent", "custom", "task",
    ];
    assert_eq!(resolve_from_cli(&root, &args).unwrap(), "custom");
    save(&root, &manifest()).unwrap();
    let err = resolve_from_cli(&root, &args).unwrap_err().to_string();
    assert!(
        err.contains("reviewer")
            && err.contains(manifest_path(&root).as_str())
            && err.contains("claude:opus [high]"),
        "{err}"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn groups_choose_default_effort_or_report_unsupported_effort() {
    let models = agents::ModelRoster::builtin().unwrap();
    let mut binding =
        crate::workspace_manifest::RoleBinding(vec![crate::workspace_manifest::AgentGroup {
            when: Some("standard".into()),
            models: vec!["claude:haiku".into()],
            efforts: Some(vec!["medium".into()]),
        }]);
    assert_eq!(binding.default_agent(&models).unwrap(), "claude:haiku");
    binding.0[0].models = vec!["codex:gpt-6-sol".into()];
    assert_eq!(
        binding.default_agent(&models).unwrap(),
        "codex:gpt-6-sol:medium"
    );
    binding.0[0].efforts = Some(vec!["veryhigh".into()]);
    let err = binding.default_agent(&models).unwrap_err().to_string();
    assert!(
        err.contains("gpt-6-sol") && err.contains("veryhigh"),
        "{err}"
    );
    binding.0[0].models = vec!["codex:gpt-6-sol:high".into()];
    let err = binding.default_agent(&models).unwrap_err().to_string();
    assert!(err.contains("must not carry an effort"), "{err}");
}

#[test]
fn a_lead_reviewer_cannot_be_spawned_without_an_explicit_agent() {
    let root = temp_test_path("spawn-lead-reviewer");
    let mut manifest = manifest();
    manifest.reviewer = crate::workspace_manifest::ReviewerBinding::Lead;
    save(&root, &manifest).unwrap();
    let args = ["niles", "spawn", "job", "--role", "reviewer", "task"];
    let err = resolve_from_cli(&root, &args).unwrap_err().to_string();
    assert!(
        err.contains(manifest_path(&root).as_str()) && err.contains("--agent"),
        "{err}"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn omitted_agent_requires_a_manifest() {
    let root = temp_test_path("spawn-no-manifest");
    let config = load_project_config_from(&root).unwrap();
    let err = resolve_agent(&root, WorkerRole::Reviewer, None, &config)
        .unwrap_err()
        .to_string();
    assert!(err.contains(manifest_path(&root).as_str()), "{err}");
    assert!(err.contains("role 'reviewer'"), "{err}");
    assert!(err.contains("does not exist"), "{err}");
}

#[test]
fn omitted_agent_requires_the_roles_entry() {
    let root = temp_test_path("spawn-missing-role");
    let config = load_project_config_from(&root).unwrap();
    for role in [
        WorkerRole::Worker,
        WorkerRole::Reviewer,
        WorkerRole::Security,
    ] {
        save(&root, &manifest()).unwrap();
        let path = manifest_path(&root);
        let body = fs::read_to_string(&path).unwrap();
        let body = body
            .lines()
            .filter(|line| !line.starts_with(&format!("{}:", role.as_str())))
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(&path, body).unwrap();
        let err = format!(
            "{:#}",
            resolve_agent(&root, role, None, &config).unwrap_err()
        );
        assert!(err.contains(path.as_str()), "{err}");
        assert!(err.contains(&format!("role '{}'", role.as_str())), "{err}");
        assert!(err.contains("missing field"), "{err}");
    }
    fs::remove_dir_all(root).unwrap();
}
