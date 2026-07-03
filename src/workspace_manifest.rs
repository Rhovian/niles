use std::{
    fs,
    io::{self, BufRead, IsTerminal, Write},
};

use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};

use crate::{
    config::{
        agents,
        spec::{CommandConfig, TaskSpec, TaskStep},
    },
    schema::{self, ArtifactKind},
};

const MANIFEST_RELATIVE_PATH: &str = ".niles/manifest.yaml";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct WorkspaceManifest {
    pub manager: String,
    pub planner: String,
    pub implementer: String,
    pub reviewer: String,
    pub validation_command: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceManifestDefaults {
    pub manager: String,
    pub planner: String,
    pub implementer: String,
    pub reviewer: String,
    pub validation_command: String,
}

impl Default for WorkspaceManifestDefaults {
    fn default() -> Self {
        Self {
            manager: "claude".to_owned(),
            planner: "claude".to_owned(),
            implementer: "codex".to_owned(),
            reviewer: "claude".to_owned(),
            validation_command: "test".to_owned(),
        }
    }
}

pub fn manifest_path(root: &Utf8Path) -> Utf8PathBuf {
    root.join(MANIFEST_RELATIVE_PATH)
}

pub fn load(root: &Utf8Path) -> Result<Option<WorkspaceManifest>> {
    let path = manifest_path(root);
    schema::read_optional_yaml(&path, ArtifactKind::WorkspaceManifest)
}

pub fn load_required(root: &Utf8Path) -> Result<WorkspaceManifest> {
    load(root)?.with_context(|| {
        format!(
            "workspace manifest {} does not exist; run `niles` in an interactive tmux session to configure workspace roles",
            manifest_path(root)
        )
    })
}

pub fn save(root: &Utf8Path, manifest: &WorkspaceManifest) -> Result<()> {
    let path = manifest_path(root);
    let parent = path
        .parent()
        .with_context(|| format!("workspace manifest path has no parent: {path}"))?;
    fs::create_dir_all(parent).with_context(|| format!("failed to create {parent}"))?;
    schema::write_yaml(&path, manifest)
}

pub fn ensure_interactive(
    root: &Utf8Path,
    defaults: &WorkspaceManifestDefaults,
) -> Result<WorkspaceManifest> {
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let mut output = io::stdout();
    ensure_interactive_with_io(root, defaults, stdin.is_terminal(), &mut input, &mut output)
}

pub fn task_uses_role_bindings(spec: &TaskSpec) -> bool {
    spec.steps
        .iter()
        .any(|step| matches!(step, TaskStep::Role { .. }))
}

pub fn resolve_task_roles(mut spec: TaskSpec, manifest: &WorkspaceManifest) -> Result<TaskSpec> {
    let mut validation_command = None;

    for step in &mut spec.steps {
        let TaskStep::Role { role, task } = step else {
            continue;
        };

        let role = role.clone();
        let task = task.clone();
        *step = match role.as_str() {
            "planner" => TaskStep::Agent {
                agent: manifest.planner.clone(),
                task: task.with_context(|| "planner role step requires task text")?,
                role: Some(role),
            },
            "implementer" => TaskStep::Agent {
                agent: manifest.implementer.clone(),
                task: task.with_context(|| "implementer role step requires task text")?,
                role: Some(role),
            },
            "reviewer" => TaskStep::Agent {
                agent: manifest.reviewer.clone(),
                task: task.with_context(|| "reviewer role step requires task text")?,
                role: Some(role),
            },
            "validation" => {
                if task.is_some() {
                    bail!("validation role step must not include task text");
                }
                let command = manifest.validation_command.clone();
                validation_command = Some(command.clone());
                TaskStep::Command {
                    command,
                    role: Some(role),
                }
            }
            _ => bail!("unknown workspace role `{role}` in task step"),
        };
    }

    if let Some(command) = validation_command {
        spec.commands
            .entry(command.clone())
            .or_insert_with(|| default_command_config(&command));
    }

    Ok(spec)
}

pub fn default_command_config(command: &str) -> CommandConfig {
    CommandConfig::Full {
        run: if command == "test" {
            "cargo test".to_owned()
        } else {
            command.to_owned()
        },
    }
}

fn ensure_interactive_with_io<R: BufRead, W: Write>(
    root: &Utf8Path,
    defaults: &WorkspaceManifestDefaults,
    interactive: bool,
    input: &mut R,
    output: &mut W,
) -> Result<WorkspaceManifest> {
    let path = manifest_path(root);
    let mut recreating = false;
    let existing = match load(root) {
        Ok(existing) => existing,
        Err(err) if interactive && path.exists() => {
            writeln!(output, "Niles workspace manifest could not be read: {path}")?;
            writeln!(output, "{err}")?;
            if prompt_yes_no(input, output, "Recreate workspace manifest?", false)? {
                recreating = true;
                None
            } else {
                return Err(err);
            }
        }
        Err(err) => return Err(err),
    };
    if !interactive {
        if existing.is_some() {
            bail!(
                "workspace manifest {path} exists but stdin is not interactive; start or attach a tmux session and run `niles` interactively to choose the manager agent"
            );
        } else {
            bail!(
                "workspace manifest {path} does not exist and stdin is not interactive; start or attach a tmux session and run `niles` interactively to configure workspace roles"
            );
        }
    }

    if let Some(mut manifest) = existing {
        writeln!(output, "Niles workspace manifest: {path}")?;
        writeln!(
            output,
            "Choose the foreground manager agent. Press Enter to accept the default."
        )?;
        let manager = prompt_agent_value(input, output, "Manager agent", &manifest.manager)?;
        let manager_changed = manager != manifest.manager;
        manifest.manager = manager;
        if manager_changed {
            save(root, &manifest)?;
            writeln!(output, "manifest: {path} (updated manager)")?;
        }

        if prompt_yes_no(input, output, "Change any manifest roles?", false)? {
            writeln!(
                output,
                "Choose persistent agents for this workspace. Press Enter to accept a default."
            )?;
            manifest = prompt_manifest_values(input, output, &manifest)?;
            save(root, &manifest)?;
            writeln!(output, "manifest: {path} (updated roles)")?;
        }

        return Ok(manifest);
    }

    if recreating {
        writeln!(output, "Recreating Niles workspace manifest: {path}")?;
    } else {
        writeln!(output, "Niles workspace manifest not found: {path}")?;
    }
    writeln!(
        output,
        "Choose persistent agents for this workspace. Press Enter to accept a default."
    )?;
    let defaults = WorkspaceManifest {
        manager: defaults.manager.clone(),
        planner: defaults.planner.clone(),
        implementer: defaults.implementer.clone(),
        reviewer: defaults.reviewer.clone(),
        validation_command: defaults.validation_command.clone(),
    };
    let mut manifest = prompt_manifest_values(input, output, &defaults)?;
    save(root, &manifest)?;
    writeln!(output, "manifest: {path}")?;

    if prompt_yes_no(input, output, "Change any manifest roles?", false)? {
        writeln!(
            output,
            "Choose persistent agents for this workspace. Press Enter to accept a default."
        )?;
        manifest = prompt_manifest_values(input, output, &manifest)?;
        save(root, &manifest)?;
        writeln!(output, "manifest: {path} (updated roles)")?;
    }

    Ok(manifest)
}

fn prompt_manifest_values<R: BufRead, W: Write>(
    input: &mut R,
    output: &mut W,
    defaults: &WorkspaceManifest,
) -> Result<WorkspaceManifest> {
    Ok(WorkspaceManifest {
        manager: prompt_agent_value(input, output, "Manager agent", &defaults.manager)?,
        planner: prompt_agent_value(input, output, "Planner agent", &defaults.planner)?,
        implementer: prompt_agent_value(input, output, "Implementer agent", &defaults.implementer)?,
        reviewer: prompt_agent_value(input, output, "Reviewer agent", &defaults.reviewer)?,
        validation_command: prompt_value(
            input,
            output,
            "Default validation command",
            &defaults.validation_command,
        )?,
    })
}

fn prompt_yes_no<R: BufRead, W: Write>(
    input: &mut R,
    output: &mut W,
    label: &str,
    default: bool,
) -> Result<bool> {
    let default_label = if default { "Y/n" } else { "y/N" };
    loop {
        write!(output, "{label} [{default_label}]: ")?;
        output.flush()?;

        let mut line = String::new();
        let bytes = input
            .read_line(&mut line)
            .with_context(|| format!("failed to read {label}"))?;
        if bytes == 0 {
            bail!("stdin closed before workspace manifest was configured");
        }

        match line.trim().to_ascii_lowercase().as_str() {
            "" => return Ok(default),
            "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            _ => writeln!(output, "Please answer y or n.")?,
        }
    }
}

fn prompt_agent_value<R: BufRead, W: Write>(
    input: &mut R,
    output: &mut W,
    label: &str,
    default: &str,
) -> Result<String> {
    loop {
        let value = prompt_value(input, output, label, default)?;
        match agents::parse_spec(&value) {
            Ok(_) => return Ok(value),
            Err(err) => writeln!(output, "Invalid agent spec: {err}")?,
        }
    }
}

fn prompt_value<R: BufRead, W: Write>(
    input: &mut R,
    output: &mut W,
    label: &str,
    default: &str,
) -> Result<String> {
    loop {
        write!(output, "{label} [{default}]: ")?;
        output.flush()?;

        let mut line = String::new();
        let bytes = input
            .read_line(&mut line)
            .with_context(|| format!("failed to read {label}"))?;
        if bytes == 0 {
            bail!("stdin closed before workspace manifest was configured");
        }

        let value = line.trim();
        let value = if value.is_empty() { default } else { value };
        if !value.trim().is_empty() {
            return Ok(value.to_owned());
        }

        writeln!(output, "{label} cannot be empty")?;
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    use std::{
        io::Cursor,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn skips_role_changes_by_default() {
        let root = temp_test_path("existing-no-role-update");
        fs::create_dir_all(root.join(".niles")).unwrap();
        let original = WorkspaceManifest {
            manager: "codex".to_owned(),
            planner: "plan-old".to_owned(),
            implementer: "impl-old".to_owned(),
            reviewer: "review-old".to_owned(),
            validation_command: "lint".to_owned(),
        };
        save(&root, &original).unwrap();

        let mut input = Cursor::new(b"\n\n".to_vec());
        let mut output = Vec::new();
        let manifest = ensure_interactive_with_io(
            &root,
            &WorkspaceManifestDefaults::default(),
            true,
            &mut input,
            &mut output,
        )
        .unwrap();

        assert_eq!(manifest, original);
        let persisted = load(&root).unwrap().unwrap();
        assert_eq!(persisted, manifest);
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("Manager agent [codex]: "));
        assert!(output.contains("Change any manifest roles? [y/N]: "));
        assert!(!output.contains("(updated manager)"));
        assert!(!output.contains("(updated roles)"));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn prompts_for_manager_and_persists_manager_change() {
        let root = temp_test_path("existing-prompt");
        fs::create_dir_all(root.join(".niles")).unwrap();
        fs::write(
            manifest_path(&root),
            r#"
manager: codex
planner: claude
implementer: codex
reviewer: claude
validation_command: lint
niles_schema: 2
"#,
        )
        .unwrap();

        let mut input = Cursor::new(b"claude\n\n".to_vec());
        let mut output = Vec::new();
        let manifest = ensure_interactive_with_io(
            &root,
            &WorkspaceManifestDefaults::default(),
            true,
            &mut input,
            &mut output,
        )
        .unwrap();

        assert_eq!(manifest.manager, "claude");
        assert_eq!(manifest.validation_command, "lint");
        let persisted = load(&root).unwrap().unwrap();
        assert_eq!(persisted.manager, "claude");
        assert_eq!(persisted.planner, "claude");
        assert_eq!(persisted.implementer, "codex");
        assert_eq!(persisted.reviewer, "claude");
        assert_eq!(persisted.validation_command, "lint");
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("Niles workspace manifest: "));
        assert!(output.contains("Manager agent [codex]: "));
        assert!(output.contains("(updated manager)"));
        assert!(output.contains("Change any manifest roles? [y/N]: "));
        assert!(!output.contains("(updated roles)"));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn parseable_legacy_manifest_loads_and_stamps_on_next_write() {
        let root = temp_test_path("legacy-load");
        fs::create_dir_all(root.join(".niles")).unwrap();
        fs::write(
            manifest_path(&root),
            r#"
manager: codex
planner: claude
implementer: codex
reviewer: claude
validation_command: lint
"#,
        )
        .unwrap();

        let mut input = Cursor::new(b"claude\n\n".to_vec());
        let mut output = Vec::new();
        let manifest = ensure_interactive_with_io(
            &root,
            &WorkspaceManifestDefaults::default(),
            true,
            &mut input,
            &mut output,
        )
        .unwrap();

        assert_eq!(manifest.manager, "claude");
        let persisted = fs::read_to_string(manifest_path(&root)).unwrap();
        assert!(persisted.contains("niles_schema: 2"));
        assert!(
            String::from_utf8(output)
                .unwrap()
                .contains("(updated manager)")
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_invalid_agent_specs_before_persisting_manifest_values() {
        let root = temp_test_path("invalid-agent-prompt");
        fs::create_dir_all(root.join(".niles")).unwrap();
        let original = WorkspaceManifest {
            manager: "codex".to_owned(),
            planner: "claude".to_owned(),
            implementer: "codex".to_owned(),
            reviewer: "claude".to_owned(),
            validation_command: "lint".to_owned(),
        };
        save(&root, &original).unwrap();

        let mut input = Cursor::new(
            b"claude:opus:turbo\nclaude:haiku:low\ny\n\nclaude:nope:low\nclaude:haiku:max\ncodex\nclaude\nlint\n"
                .to_vec(),
        );
        let mut output = Vec::new();
        let manifest = ensure_interactive_with_io(
            &root,
            &WorkspaceManifestDefaults::default(),
            true,
            &mut input,
            &mut output,
        )
        .unwrap();

        assert_eq!(manifest.manager, "claude:haiku:low");
        assert_eq!(manifest.planner, "claude:haiku:max");
        assert_eq!(manifest.implementer, "codex");
        assert_eq!(manifest.reviewer, "claude");
        assert_eq!(manifest.validation_command, "lint");
        let persisted = load(&root).unwrap().unwrap();
        assert_eq!(persisted, manifest);

        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("Invalid agent spec: unsupported claude effort `turbo`"));
        assert!(output.contains("Invalid agent spec: unsupported claude model `nope`"));
        assert!(output.contains("Manager agent [codex]: "));
        assert!(output.contains("Planner agent [claude]: "));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn can_reconfigure_workspace_manifest_roles_after_manager_pick() {
        let root = temp_test_path("existing-role-update");
        fs::create_dir_all(root.join(".niles")).unwrap();
        fs::write(
            manifest_path(&root),
            r#"
manager: codex
planner: plan-old
implementer: impl-old
reviewer: review-old
validation_command: lint
niles_schema: 2
"#,
        )
        .unwrap();

        let mut input =
            Cursor::new(b"claude\ny\nclaude\nplanbot\ncodebot\nreviewbot\ncheck\n".to_vec());
        let mut output = Vec::new();
        let manifest = ensure_interactive_with_io(
            &root,
            &WorkspaceManifestDefaults::default(),
            true,
            &mut input,
            &mut output,
        )
        .unwrap();

        assert_eq!(
            manifest,
            WorkspaceManifest {
                manager: "claude".to_owned(),
                planner: "planbot".to_owned(),
                implementer: "codebot".to_owned(),
                reviewer: "reviewbot".to_owned(),
                validation_command: "check".to_owned(),
            }
        );

        let persisted = load(&root).unwrap().unwrap();
        assert_eq!(persisted, manifest);
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("Change any manifest roles? [y/N]: "));
        assert!(output.contains("Manager agent [codex]: "));
        assert!(output.contains("Manager agent [claude]: "));
        assert!(output.contains("Planner agent [plan-old]: "));
        assert!(output.contains("Implementer agent [impl-old]: "));
        assert!(output.contains("Reviewer agent [review-old]: "));
        assert!(output.contains("Default validation command [lint]: "));
        assert!(output.contains("(updated roles)"));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn skewed_manifest_can_be_recreated_interactively() {
        let root = temp_test_path("skewed-recreate");
        fs::create_dir_all(root.join(".niles")).unwrap();
        fs::write(manifest_path(&root), "manager: codex\n").unwrap();

        let mut input = Cursor::new(b"y\ncodex\nclaude\ncodex\nclaude\ncheck\n\n".to_vec());
        let mut output = Vec::new();
        let manifest = ensure_interactive_with_io(
            &root,
            &WorkspaceManifestDefaults::default(),
            true,
            &mut input,
            &mut output,
        )
        .unwrap();

        assert_eq!(manifest.validation_command, "check");
        let persisted = fs::read_to_string(manifest_path(&root)).unwrap();
        assert!(persisted.contains("niles_schema: 2"));
        assert!(persisted.contains("validation_command: check"));
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("could not be read"));
        assert!(output.contains("Recreate workspace manifest? [y/N]: "));
        assert!(output.contains("Recreating Niles workspace manifest"));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn skewed_manifest_remediation_names_delete_and_rerun() {
        let root = temp_test_path("skewed-remediation");
        fs::create_dir_all(root.join(".niles")).unwrap();
        fs::write(manifest_path(&root), "manager: codex\n").unwrap();

        let err = load(&root).unwrap_err().to_string();

        assert!(err.contains("workspace manifest"));
        assert!(err.contains("schema 1"));
        assert!(err.contains("delete .niles/manifest.yaml and rerun `niles`"));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn existing_workspace_manifest_errors_when_stdin_is_not_interactive() {
        let root = temp_test_path("existing-noninteractive");
        fs::create_dir_all(root.join(".niles")).unwrap();
        fs::write(
            manifest_path(&root),
            r#"
manager: codex
planner: claude
implementer: codex
reviewer: claude
validation_command: lint
niles_schema: 2
"#,
        )
        .unwrap();

        let mut input = Cursor::new(Vec::<u8>::new());
        let mut output = Vec::new();
        let err = ensure_interactive_with_io(
            &root,
            &WorkspaceManifestDefaults::default(),
            false,
            &mut input,
            &mut output,
        )
        .unwrap_err();

        assert!(err.to_string().contains("stdin is not interactive"));
        assert!(err.to_string().contains("choose the manager agent"));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn creates_workspace_manifest_from_interactive_answers() {
        let root = temp_test_path("create");
        let mut input = Cursor::new(b"codex\nclaude\ncodex\nclaude\ncheck\n\n".to_vec());
        let mut output = Vec::new();

        let manifest = ensure_interactive_with_io(
            &root,
            &WorkspaceManifestDefaults::default(),
            true,
            &mut input,
            &mut output,
        )
        .unwrap();

        assert_eq!(
            manifest,
            WorkspaceManifest {
                manager: "codex".to_owned(),
                planner: "claude".to_owned(),
                implementer: "codex".to_owned(),
                reviewer: "claude".to_owned(),
                validation_command: "check".to_owned(),
            }
        );

        let persisted = load(&root).unwrap().unwrap();
        assert_eq!(persisted, manifest);
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("Niles workspace manifest not found"));
        assert!(output.contains("Manager agent [claude]: "));
        assert!(output.contains("Change any manifest roles? [y/N]: "));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_manifest_is_error_when_stdin_is_not_interactive() {
        let root = temp_test_path("noninteractive");
        let mut input = Cursor::new(Vec::<u8>::new());
        let mut output = Vec::new();

        let err = ensure_interactive_with_io(
            &root,
            &WorkspaceManifestDefaults::default(),
            false,
            &mut input,
            &mut output,
        )
        .unwrap_err();

        assert!(err.to_string().contains("stdin is not interactive"));
        assert!(!manifest_path(&root).exists());
    }

    #[test]
    fn resolves_role_steps_from_workspace_manifest() {
        let manifest = WorkspaceManifest {
            manager: "claude".to_owned(),
            planner: "planbot".to_owned(),
            implementer: "codebot".to_owned(),
            reviewer: "reviewbot".to_owned(),
            validation_command: "check".to_owned(),
        };
        let spec = TaskSpec {
            goal: "ship".to_owned(),
            workspace: None,
            agents: Default::default(),
            steps: vec![
                TaskStep::Role {
                    role: "planner".to_owned(),
                    task: Some("plan".to_owned()),
                },
                TaskStep::Role {
                    role: "validation".to_owned(),
                    task: None,
                },
            ],
            commands: Default::default(),
        };

        let spec = resolve_task_roles(spec, &manifest).unwrap();

        assert!(matches!(
            &spec.steps[0],
            TaskStep::Agent { agent, role, .. }
                if agent == "planbot" && role.as_deref() == Some("planner")
        ));
        assert!(matches!(
            &spec.steps[1],
            TaskStep::Command { command, role }
                if command == "check" && role.as_deref() == Some("validation")
        ));
        assert!(spec.commands.contains_key("check"));
    }

    fn temp_test_path(label: &str) -> Utf8PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        Utf8PathBuf::from_path_buf(std::env::temp_dir().join(format!(
            "niles-workspace-manifest-{label}-{}-{nanos}",
            std::process::id()
        )))
        .unwrap()
    }
}
