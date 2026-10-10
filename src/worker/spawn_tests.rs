use clap::Parser;

use super::*;
use crate::{
    cli::{Cli, CommandName},
    test_support::temp_test_path,
    workspace_manifest::{WorkspaceManifest, manifest_path, save},
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
    resolve_agent(project, assignment.role, agent, &config)
}

fn manifest() -> WorkspaceManifest {
    WorkspaceManifest {
        lead: "leadbot".into(),
        worker: "codex:gpt-5.5:xhigh".to_owned().into(),
        reviewer: "claude:opus:high".to_owned().into(),
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
        ("design", "claude"),
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
            && err.contains("claude:opus:high"),
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
fn scalar_bindings_keep_colons_in_model_names_and_pin_their_effort() {
    let models =
        agents::roster::parse("pi:\n  nvidia/nemotron:free:\n    efforts: [low, high]\n").unwrap();
    let parse = |agent| agents::AgentSpec::parse(agent, &models).unwrap();
    for (scalar, allowed, refused) in [
        (
            "pi:nvidia/nemotron:free",
            "pi:nvidia/nemotron:free:low",
            None,
        ),
        (
            "pi:nvidia/nemotron:free:high",
            "pi:nvidia/nemotron:free:high",
            Some("pi:nvidia/nemotron:free:low"),
        ),
    ] {
        let binding = crate::workspace_manifest::RoleBinding::from(scalar.to_owned());
        assert_eq!(binding.default_agent(&models).unwrap(), scalar);
        assert!(binding.allows(&parse(allowed), &models).unwrap());
        assert!(refused.is_none_or(|agent| !binding.allows(&parse(agent), &models).unwrap()));
    }
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

fn assignment(role: WorkerRole, design: Option<&str>, mechanical: Option<&str>) -> Assignment {
    Assignment {
        role,
        design: design.map(str::to_owned),
        mechanical: mechanical.map(str::to_owned),
    }
}

#[test]
fn assignment_flags_refuse_invalid_combinations_and_reasons() {
    use WorkerRole::{Design, Research, Reviewer, Security, Worker};
    let root = temp_test_path("assignment-flags");
    let flags = "--design or --mechanical";
    for (role, design, mechanical, message) in [
        (Worker, None, None, flags),
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
    for role in [Reviewer, Security, Design, Research] {
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
