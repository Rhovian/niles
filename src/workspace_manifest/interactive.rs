use std::{
    collections::BTreeMap,
    io::{self, BufRead, IsTerminal, Write},
};

use anyhow::{Context, Result, bail};
use camino::Utf8Path;

use crate::{
    agents::{self, picker},
    config::spec::{AgentConfig, load_project_config_from},
};

use super::{WorkspaceManifest, load, manifest_path, save};

pub fn ensure_interactive(
    root: &Utf8Path,
    defaults: &WorkspaceManifest,
) -> Result<WorkspaceManifest> {
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let mut output = io::stdout();
    ensure_interactive_with_io(root, defaults, stdin.is_terminal(), &mut input, &mut output)
}

fn ensure_interactive_with_io<R: BufRead, W: Write>(
    root: &Utf8Path,
    defaults: &WorkspaceManifest,
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
        let manager =
            picker::prompt_agent_value(root, "Manager agent", &manifest.manager, &agent_configs)?;
        let manager_changed = manager != manifest.manager;
        manifest.manager = manager;
        if manager_changed {
            save(root, &manifest)?;
            writeln!(output, "manifest: {path} (updated manager)")?;
        }

        maybe_update_manifest_roles(root, input, output, &path, &mut manifest, &agent_configs)?;

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
    let manager =
        picker::prompt_agent_value(root, "Manager agent", &defaults.manager, &agent_configs)?;
    let manifest = prompt_manifest_values(root, input, output, manager, defaults, &agent_configs)?;
    save(root, &manifest)?;
    writeln!(output, "manifest: {path}")?;
    print_manifest_roles(output, &manifest)?;

    Ok(manifest)
}

fn maybe_update_manifest_roles<R: BufRead, W: Write>(
    root: &Utf8Path,
    input: &mut R,
    output: &mut W,
    path: &Utf8Path,
    manifest: &mut WorkspaceManifest,
    agent_configs: &BTreeMap<String, AgentConfig>,
) -> Result<()> {
    print_manifest_roles(output, manifest)?;
    if prompt_yes_no(input, output, "Change any manifest roles?", false)? {
        writeln!(
            output,
            "Choose persistent agents for this workspace. Press Enter to accept a default."
        )?;
        let manager = manifest.manager.clone();
        *manifest = prompt_manifest_values(root, input, output, manager, manifest, agent_configs)?;
        save(root, manifest)?;
        writeln!(output, "manifest: {path} (updated roles)")?;
        print_manifest_roles(output, manifest)?;
    }

    Ok(())
}

fn print_manifest_roles<W: Write>(output: &mut W, manifest: &WorkspaceManifest) -> Result<()> {
    let rows = [
        manifest_role_row("manager", &manifest.manager),
        manifest_role_row("planner", &manifest.planner),
        manifest_role_row("worker", &manifest.worker),
        manifest_role_row("reviewer", &manifest.reviewer),
    ];
    let role_width = rows
        .iter()
        .map(|row| row.role.len())
        .chain([VALIDATION_ROLE.len()])
        .max()
        .context("manifest roles table has no role labels")?;
    let family_width = rows
        .iter()
        .map(|row| row.family.len())
        .max()
        .context("manifest roles table has no family values")?;
    let model_width = rows
        .iter()
        .map(|row| row.model.len())
        .max()
        .context("manifest roles table has no model values")?;

    for row in rows {
        writeln!(
            output,
            "{:<role_width$}  {:<family_width$}  {:<model_width$}  {}",
            row.role, row.family, row.model, row.effort
        )?;
    }
    writeln!(
        output,
        "{:<role_width$}  {}",
        VALIDATION_ROLE, manifest.validation_command
    )?;

    Ok(())
}

fn manifest_role_row(role: &'static str, value: &str) -> ManifestRoleRow {
    match agents::parse_spec(value) {
        Ok(spec) => ManifestRoleRow {
            role,
            family: spec.family().to_owned(),
            model: display_spec_part(spec.model()),
            effort: display_spec_part(spec.effort()),
        },
        Err(_) => ManifestRoleRow {
            role,
            family: format!("{value:?}"),
            model: MISSING_SPEC_PART.to_owned(),
            effort: MISSING_SPEC_PART.to_owned(),
        },
    }
}

fn display_spec_part(value: Option<&str>) -> String {
    match value {
        Some(value) => value.to_owned(),
        None => MISSING_SPEC_PART.to_owned(),
    }
}

struct ManifestRoleRow {
    role: &'static str,
    family: String,
    model: String,
    effort: String,
}

const MISSING_SPEC_PART: &str = "-";
const VALIDATION_ROLE: &str = "validation";

fn prompt_manifest_values<R: BufRead, W: Write>(
    root: &Utf8Path,
    input: &mut R,
    output: &mut W,
    manager: String,
    defaults: &WorkspaceManifest,
    agent_configs: &BTreeMap<String, AgentConfig>,
) -> Result<WorkspaceManifest> {
    Ok(WorkspaceManifest {
        manager,
        planner: picker::prompt_agent_value(
            root,
            "Planner agent",
            &defaults.planner,
            agent_configs,
        )?,
        worker: picker::prompt_agent_value(root, "Worker agent", &defaults.worker, agent_configs)?,
        reviewer: picker::prompt_agent_value(
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

    use std::{fs, io::Cursor};

    use super::super::{io::manifest_path, test_support::temp_test_path};

    #[test]
    fn manifest_roles_are_printed_before_change_prompt() -> Result<()> {
        let root = temp_test_path("roles-table-before-prompt");
        let path = manifest_path(&root);
        let mut manifest = WorkspaceManifest {
            manager: "codex:gpt-5.5:xhigh".to_owned(),
            planner: "claude:opus:high".to_owned(),
            worker: "codex".to_owned(),
            reviewer: "claude:opus:max".to_owned(),
            validation_command: "cargo test".to_owned(),
            flow: super::super::initial_flow(),
        };
        let mut input = Cursor::new(b"n\n".to_vec());
        let mut output = Vec::new();

        maybe_update_manifest_roles(
            &root,
            &mut input,
            &mut output,
            &path,
            &mut manifest,
            &BTreeMap::new(),
        )?;

        assert_eq!(
            String::from_utf8(output)?,
            "\
manager     codex   gpt-5.5  xhigh
planner     claude  opus     high
worker      codex   -        -
reviewer    claude  opus     max
validation  cargo test
Change any manifest roles? [y/N]: "
        );
        Ok(())
    }

    #[test]
    fn manifest_roles_quote_unparseable_specs_without_failing() -> Result<()> {
        let manifest = WorkspaceManifest {
            manager: "codex::xhigh".to_owned(),
            planner: "custom:model:high".to_owned(),
            worker: "codex".to_owned(),
            reviewer: "claude:haiku".to_owned(),
            validation_command: "cargo clippy --all-targets -- -D warnings".to_owned(),
            flow: super::super::initial_flow(),
        };
        let mut output = Vec::new();

        print_manifest_roles(&mut output, &manifest)?;

        assert_eq!(
            String::from_utf8(output)?,
            "\
manager     \"codex::xhigh\"       -      -
planner     \"custom:model:high\"  -      -
worker      codex                -      -
reviewer    claude               haiku  -
validation  cargo clippy --all-targets -- -D warnings
"
        );
        Ok(())
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
worker: codex
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
            &WorkspaceManifest::default(),
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
            &WorkspaceManifest::default(),
            false,
            &mut input,
            &mut output,
        )
        .unwrap_err();

        assert!(err.to_string().contains("stdin is not interactive"));
        assert!(!manifest_path(&root).exists());
    }
}
