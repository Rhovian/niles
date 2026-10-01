use std::io::{self, BufRead, IsTerminal, Write};

use anyhow::{Context, Result, bail};
use camino::Utf8Path;

use crate::{
    agents::picker,
    config::spec::{ProjectConfig, load_project_config_from},
};

use super::{WorkspaceManifest, load, manifest_path, roles_table::print_manifest_roles, save};

pub fn ensure_interactive(root: &Utf8Path) -> Result<WorkspaceManifest> {
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let mut output = io::stdout();
    ensure_interactive_with_io(root, stdin.is_terminal(), &mut input, &mut output)
}

fn ensure_interactive_with_io<R: BufRead, W: Write>(
    root: &Utf8Path,
    interactive: bool,
    input: &mut R,
    output: &mut W,
) -> Result<WorkspaceManifest> {
    let config = load_project_config_from(root)?;
    let path = manifest_path(root);
    let defaults = match load(root) {
        Ok(Some(manifest)) => {
            print_manifest_roles(output, &manifest, &config)?;
            if !interactive || !prompt_yes_no(input, output, "Change any manifest roles?", false)? {
                return Ok(manifest);
            }
            manifest
        }
        Ok(None) => {
            if !interactive {
                bail!(
                    "workspace manifest {path} does not exist; run `niles` from an interactive terminal"
                );
            }
            writeln!(output, "Niles workspace manifest not found: {path}")?;
            WorkspaceManifest::default()
        }
        Err(err) => {
            if !interactive {
                return Err(err);
            }
            writeln!(output, "Niles workspace manifest could not be read: {path}")?;
            writeln!(output, "{err}")?;
            WorkspaceManifest::default()
        }
    };
    writeln!(
        output,
        "Choose persistent agents for this workspace. Press Enter to accept a default."
    )?;
    let manifest = prompt_manifest_values(&defaults, &config)?;
    save(root, &manifest)?;
    writeln!(output, "manifest: {path}")?;

    Ok(manifest)
}

fn prompt_manifest_values(
    defaults: &WorkspaceManifest,
    config: &ProjectConfig,
) -> Result<WorkspaceManifest> {
    let pick = |label, default| picker::prompt_agent_value(label, default, config);
    Ok(WorkspaceManifest {
        lead: pick("Lead agent", &defaults.lead)?,
        worker: pick("Worker agent", &defaults.worker)?,
        reviewer: picker::prompt_reviewer_value("Reviewer agent", &defaults.reviewer, config)?,
        security: pick("Security agent", &defaults.security)?,
        // Hand-edited settings are not prompted for, so changing roles must preserve them.
        ..defaults.clone()
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

    use crate::test_support::temp_test_path;

    fn existing_manifest(root: &Utf8Path) -> Result<WorkspaceManifest> {
        let manifest = WorkspaceManifest {
            lead: "codex:gpt-5.5:xhigh".to_owned(),
            worker: "codex".to_owned(),
            reviewer: crate::workspace_manifest::ReviewerBinding::Agent(
                "claude:opus:max".to_owned(),
            ),
            security: "claude:opus:max".to_owned(),
            worker_planning: [("codex".to_owned(), "Plan carefully.".to_owned())].into(),
            ..WorkspaceManifest::default()
        };
        save(root, &manifest)?;
        Ok(manifest)
    }

    const ROLES_TABLE: &str = "\
lead      codex   gpt-5.5  xhigh
worker    codex   -        -
reviewer  claude  opus     max
security  claude  opus     max
";

    #[test]
    fn existing_manifest_prints_roles_without_terminal() -> Result<()> {
        let root = temp_test_path("existing-noninteractive");
        let manifest = existing_manifest(&root)?;
        let mut input = Cursor::new(Vec::<u8>::new());
        let mut output = Vec::new();

        let result = ensure_interactive_with_io(&root, false, &mut input, &mut output)?;

        assert_eq!(result, manifest);
        assert_eq!(String::from_utf8(output)?, ROLES_TABLE);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn existing_manifest_no_keeps_roles_without_saving() -> Result<()> {
        let root = temp_test_path("existing-no");
        let manifest = existing_manifest(&root)?;
        let path = manifest_path(&root);
        fs::write(
            &path,
            format!("{}# keep this comment\n", fs::read_to_string(&path)?),
        )?;
        let before = fs::read(manifest_path(&root))?;
        let mut input = Cursor::new(b"n\n".to_vec());
        let mut output = Vec::new();

        let result = ensure_interactive_with_io(&root, true, &mut input, &mut output)?;

        assert_eq!(result, manifest);
        assert_eq!(fs::read(manifest_path(&root))?, before);
        assert_eq!(
            String::from_utf8(output)?,
            format!("{ROLES_TABLE}Change any manifest roles? [y/N]: ")
        );
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn missing_manifest_is_error_when_stdin_is_not_interactive() {
        let root = temp_test_path("noninteractive");
        let mut input = Cursor::new(Vec::<u8>::new());
        let mut output = Vec::new();

        let err = ensure_interactive_with_io(&root, false, &mut input, &mut output).unwrap_err();

        assert!(
            err.to_string()
                .contains("run `niles` from an interactive terminal")
        );
        assert!(err.to_string().contains(manifest_path(&root).as_str()));
        assert!(output.is_empty());
        assert!(!manifest_path(&root).exists());
    }

    #[test]
    fn unreadable_manifest_returns_load_error_without_terminal() -> Result<()> {
        let root = temp_test_path("unreadable-noninteractive");
        fs::create_dir_all(root.join(".niles"))?;
        fs::write(manifest_path(&root), "lead: [\n")?;
        let expected = load(&root).unwrap_err().to_string();
        let mut input = Cursor::new(Vec::<u8>::new());
        let mut output = Vec::new();

        let err = ensure_interactive_with_io(&root, false, &mut input, &mut output).unwrap_err();

        assert_eq!(err.to_string(), expected);
        assert!(output.is_empty());
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
