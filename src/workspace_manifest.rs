use std::{
    collections::BTreeMap,
    fmt, fs,
    io::{self, BufRead, IsTerminal, Write},
};

use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};

use crate::{
    agent_picker,
    config::spec::{AgentConfig, CommandConfig, TaskSpec, TaskStep, load_project_config_from},
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
    #[serde(default = "initial_flow")]
    pub flow: Vec<WorkspaceFlowRole>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceManifestDefaults {
    pub manager: String,
    pub planner: String,
    pub implementer: String,
    pub reviewer: String,
    pub validation_command: String,
    pub flow: Vec<WorkspaceFlowRole>,
}

impl Default for WorkspaceManifestDefaults {
    fn default() -> Self {
        Self {
            manager: "claude".to_owned(),
            planner: "claude".to_owned(),
            implementer: "codex".to_owned(),
            reviewer: "claude".to_owned(),
            validation_command: "test".to_owned(),
            flow: initial_flow(),
        }
    }
}

impl WorkspaceManifestDefaults {
    fn to_manifest(&self) -> WorkspaceManifest {
        WorkspaceManifest {
            manager: self.manager.clone(),
            planner: self.planner.clone(),
            implementer: self.implementer.clone(),
            reviewer: self.reviewer.clone(),
            validation_command: self.validation_command.clone(),
            flow: self.flow.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceFlowRole {
    Planner,
    Implementer,
    Validation,
    Reviewer,
}

impl WorkspaceFlowRole {
    fn parse(value: &str) -> Result<Self> {
        match value {
            "planner" => Ok(Self::Planner),
            "implementer" => Ok(Self::Implementer),
            "validation" => Ok(Self::Validation),
            "reviewer" => Ok(Self::Reviewer),
            _ => bail!("unknown workspace role `{value}`"),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Planner => "planner",
            Self::Implementer => "implementer",
            Self::Validation => "validation",
            Self::Reviewer => "reviewer",
        }
    }
}

impl fmt::Display for WorkspaceFlowRole {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

pub fn initial_flow() -> Vec<WorkspaceFlowRole> {
    vec![
        WorkspaceFlowRole::Planner,
        WorkspaceFlowRole::Implementer,
        WorkspaceFlowRole::Validation,
        WorkspaceFlowRole::Reviewer,
    ]
}

pub fn flow_summary(flow: &[WorkspaceFlowRole]) -> String {
    if flow.is_empty() {
        return "<empty>".to_owned();
    }

    flow.iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(" -> ")
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

        let flow_role = WorkspaceFlowRole::parse(role)?;
        let role = role.clone();
        let task = task.clone();
        *step = match flow_role {
            WorkspaceFlowRole::Planner => TaskStep::Agent {
                agent: manifest.planner.clone(),
                task: task.with_context(|| "planner role step requires task text")?,
                role: Some(role),
            },
            WorkspaceFlowRole::Implementer => TaskStep::Agent {
                agent: manifest.implementer.clone(),
                task: task.with_context(|| "implementer role step requires task text")?,
                role: Some(role),
            },
            WorkspaceFlowRole::Reviewer => TaskStep::Agent {
                agent: manifest.reviewer.clone(),
                task: task.with_context(|| "reviewer role step requires task text")?,
                role: Some(role),
            },
            WorkspaceFlowRole::Validation => {
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
    let agent_configs = load_project_config_from(root)?.agents;
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
        let manager = agent_picker::prompt_agent_value(
            root,
            "Manager agent",
            &manifest.manager,
            &agent_configs,
        )?;
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
            manifest = prompt_manifest_values(root, input, output, &manifest, &agent_configs)?;
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
    let defaults = defaults.to_manifest();
    let mut manifest = prompt_manifest_values(root, input, output, &defaults, &agent_configs)?;
    save(root, &manifest)?;
    writeln!(output, "manifest: {path}")?;

    if prompt_yes_no(input, output, "Change any manifest roles?", false)? {
        writeln!(
            output,
            "Choose persistent agents for this workspace. Press Enter to accept a default."
        )?;
        manifest = prompt_manifest_values(root, input, output, &manifest, &agent_configs)?;
        save(root, &manifest)?;
        writeln!(output, "manifest: {path} (updated roles)")?;
    }

    Ok(manifest)
}

fn prompt_manifest_values<R: BufRead, W: Write>(
    root: &Utf8Path,
    input: &mut R,
    output: &mut W,
    defaults: &WorkspaceManifest,
    agent_configs: &BTreeMap<String, AgentConfig>,
) -> Result<WorkspaceManifest> {
    Ok(WorkspaceManifest {
        manager: agent_picker::prompt_agent_value(
            root,
            "Manager agent",
            &defaults.manager,
            agent_configs,
        )?,
        planner: agent_picker::prompt_agent_value(
            root,
            "Planner agent",
            &defaults.planner,
            agent_configs,
        )?,
        implementer: agent_picker::prompt_agent_value(
            root,
            "Implementer agent",
            &defaults.implementer,
            agent_configs,
        )?,
        reviewer: agent_picker::prompt_agent_value(
            root,
            "Reviewer agent",
            &defaults.reviewer,
            agent_configs,
        )?,
        validation_command: prompt_value(
            input,
            output,
            "Default validation command",
            &defaults.validation_command,
        )?,
        flow: defaults.flow.clone(),
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
    fn manifest_defaults_carry_flow() {
        let manifest = WorkspaceManifestDefaults::default().to_manifest();

        assert_eq!(
            flow_summary(&manifest.flow),
            "planner -> implementer -> validation -> reviewer"
        );
    }

    #[test]
    fn manifest_without_flow_loads_initial_flow() {
        let root = temp_test_path("manifest-flow");
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

        let manifest = load(&root).unwrap().unwrap();

        assert_eq!(
            flow_summary(&manifest.flow),
            "planner -> implementer -> validation -> reviewer"
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn current_manifest_loads_flow() {
        let root = temp_test_path("manifest-current-flow");
        fs::create_dir_all(root.join(".niles")).unwrap();
        fs::write(
            manifest_path(&root),
            r#"
manager: codex
planner: claude
implementer: codex
reviewer: claude
validation_command: lint
flow:
  - planner
  - reviewer
niles_schema: 2
"#,
        )
        .unwrap();

        let manifest = load(&root).unwrap().unwrap();

        assert_eq!(flow_summary(&manifest.flow), "planner -> reviewer");

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn saving_manifest_writes_flow() {
        let root = temp_test_path("manifest-save-flow");
        let manifest = WorkspaceManifest {
            manager: "claude".to_owned(),
            planner: "planbot".to_owned(),
            implementer: "codebot".to_owned(),
            reviewer: "reviewbot".to_owned(),
            validation_command: "check".to_owned(),
            flow: vec![WorkspaceFlowRole::Implementer],
        };

        save(&root, &manifest).unwrap();
        let body = fs::read_to_string(manifest_path(&root)).unwrap();

        assert!(body.contains("flow:\n- implementer"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn resolves_role_steps_from_workspace_manifest() {
        let manifest = WorkspaceManifest {
            manager: "claude".to_owned(),
            planner: "planbot".to_owned(),
            implementer: "codebot".to_owned(),
            reviewer: "reviewbot".to_owned(),
            validation_command: "check".to_owned(),
            flow: initial_flow(),
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
