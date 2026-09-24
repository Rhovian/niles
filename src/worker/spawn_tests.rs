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
    resolve_agent(project, role, agent)
}

fn manifest() -> WorkspaceManifest {
    WorkspaceManifest {
        lead: "leadbot".into(),
        worker: "codex:gpt-5.5:xhigh".into(),
        reviewer: "claude:opus:high".into(),
        security: "auditbot".into(),
        ..WorkspaceManifest::default()
    }
}

#[test]
fn omitted_agent_uses_each_roles_manifest_binding() {
    let root = temp_test_path("spawn-role-agents");
    let manifest = manifest();
    save(&root, &manifest).unwrap();
    for (role, expected) in [
        ("worker", manifest.worker),
        ("reviewer", manifest.reviewer),
        ("security", manifest.security),
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
fn explicit_agent_overrides_manifest_and_does_not_require_one() {
    let root = temp_test_path("spawn-explicit-agent");
    let args = [
        "niles", "spawn", "job", "--role", "reviewer", "--agent", "custom", "task",
    ];
    assert_eq!(resolve_from_cli(&root, &args).unwrap(), "custom");
    save(&root, &manifest()).unwrap();
    assert_eq!(resolve_from_cli(&root, &args).unwrap(), "custom");
    fs::write(manifest_path(&root), "invalid: [").unwrap();
    assert_eq!(resolve_from_cli(&root, &args).unwrap(), "custom");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn omitted_agent_requires_a_manifest() {
    let root = temp_test_path("spawn-no-manifest");
    let err = resolve_agent(&root, WorkerRole::Reviewer, None)
        .unwrap_err()
        .to_string();
    assert!(err.contains(manifest_path(&root).as_str()), "{err}");
    assert!(err.contains("role 'reviewer'"), "{err}");
    assert!(err.contains("does not exist"), "{err}");
}

#[test]
fn omitted_agent_requires_the_roles_entry() {
    let root = temp_test_path("spawn-missing-role");
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
        let err = format!("{:#}", resolve_agent(&root, role, None).unwrap_err());
        assert!(err.contains(path.as_str()), "{err}");
        assert!(err.contains(&format!("role '{}'", role.as_str())), "{err}");
        assert!(err.contains("missing field"), "{err}");
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn omitted_agent_rejects_an_empty_role_binding() {
    let root = temp_test_path("spawn-empty-role");
    let mut manifest = manifest();
    manifest.security = " ".into();
    save(&root, &manifest).unwrap();
    let err = resolve_agent(&root, WorkerRole::Security, None)
        .unwrap_err()
        .to_string();
    assert!(err.contains(manifest_path(&root).as_str()), "{err}");
    assert!(err.contains("role 'security'"), "{err}");
    fs::remove_dir_all(root).unwrap();
}
