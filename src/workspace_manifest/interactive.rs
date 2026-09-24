use std::{
    collections::BTreeMap,
    io::{self, BufRead, IsTerminal, Write},
};

use anyhow::{Context, Result, bail};
use camino::Utf8Path;

use crate::{
    agents::picker,
    config::spec::{AgentConfig, load_project_config_from},
};

use super::{WorkspaceManifest, load, manifest_path, roles_table::print_manifest_roles, save};

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
                "workspace manifest {path} exists but stdin is not interactive; start or attach a tmux session and run `niles` interactively to choose the lead agent"
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
            "Choose the foreground lead agent. Press Enter to accept the default."
        )?;
        let lead = picker::prompt_agent_value("Lead agent", &manifest.lead, &agent_configs)?;
        let lead_changed = lead != manifest.lead;
        manifest.lead = lead;
        if lead_changed {
            save(root, &manifest)?;
            writeln!(output, "manifest: {path} (updated lead)")?;
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
    let lead = picker::prompt_agent_value("Lead agent", &defaults.lead, &agent_configs)?;
    let manifest = prompt_manifest_values(lead, defaults, &agent_configs)?;
    save(root, &manifest)?;
    writeln!(output, "manifest: {path}")?;

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
    print_manifest_roles(output, manifest, agent_configs)?;
    if prompt_yes_no(input, output, "Change any manifest roles?", false)? {
        writeln!(
            output,
            "Choose persistent agents for this workspace. Press Enter to accept a default."
        )?;
        let lead = manifest.lead.clone();
        *manifest = prompt_manifest_values(lead, manifest, agent_configs)?;
        save(root, manifest)?;
        writeln!(output, "manifest: {path} (updated roles)")?;
        print_manifest_roles(output, manifest, agent_configs)?;
    }

    Ok(())
}

fn prompt_manifest_values(
    lead: String,
    defaults: &WorkspaceManifest,
    agent_configs: &BTreeMap<String, AgentConfig>,
) -> Result<WorkspaceManifest> {
    Ok(WorkspaceManifest {
        lead,
        worker: picker::prompt_agent_value("Worker agent", &defaults.worker, agent_configs)?,
        reviewer: picker::prompt_agent_value("Reviewer agent", &defaults.reviewer, agent_configs)?,
        security: picker::prompt_agent_value("Security agent", &defaults.security, agent_configs)?,
        // Not prompted for: the check-in cadence is a hand-edited line, and changing a role must
        // not quietly drop the one the workspace already carries.
        checkin: defaults.checkin.clone(),
        recheck: defaults.recheck.clone(),
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
            lead: "codex:gpt-5.5:xhigh".to_owned(),
            worker: "codex".to_owned(),
            reviewer: "claude:opus:max".to_owned(),
            security: "claude:opus:max".to_owned(),
            ..WorkspaceManifest::default()
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
lead      codex   gpt-5.5  xhigh
worker    codex   -        -
reviewer  claude  opus     max
security  claude  opus     max
Change any manifest roles? [y/N]: "
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
lead: codex
worker: codex
reviewer: claude
security: claude
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
        assert!(err.to_string().contains("choose the lead agent"));

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
