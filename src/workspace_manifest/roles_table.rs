use std::{collections::BTreeMap, io::Write};

use anyhow::Result;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::{agents, config::spec::AgentConfig};

use super::WorkspaceManifest;

const MAX_TABLE_CELL_WIDTH: usize = 64;
const MAX_TABLE_NOTE_WIDTH: usize = 96;
const MISSING_SPEC_PART: &str = "-";
const TRUNCATION_MARKER: &str = "...";
const VALIDATION_ROLE: &str = "validation";

pub(super) fn print_manifest_roles<W: Write>(
    output: &mut W,
    manifest: &WorkspaceManifest,
    agent_configs: &BTreeMap<String, AgentConfig>,
) -> Result<()> {
    let rows = [
        manifest_role_row("manager", &manifest.manager, agent_configs),
        manifest_role_row("planner", &manifest.planner, agent_configs),
        manifest_role_row("worker", &manifest.worker, agent_configs),
        manifest_role_row("reviewer", &manifest.reviewer, agent_configs),
    ];
    let widths = rows.iter().fold(
        TableWidths {
            role: display_width(VALIDATION_ROLE),
            family: 0,
            model: 0,
        },
        |widths, row| TableWidths {
            role: widths.role.max(display_width(row.role)),
            family: widths.family.max(display_width(&row.family)),
            model: widths.model.max(display_width(&row.model)),
        },
    );

    for row in &rows {
        write_padded(output, row.role, widths.role)?;
        write!(output, "  ")?;
        write_padded(output, &row.family, widths.family)?;
        write!(output, "  ")?;
        write_padded(output, &row.model, widths.model)?;
        write!(output, "  {}", row.effort)?;
        if let Some(note) = &row.note {
            write!(output, "  {note}")?;
        }
        writeln!(output)?;
    }
    write_padded(output, VALIDATION_ROLE, widths.role)?;
    writeln!(output, "  {}", display_note(&manifest.validation_command))?;

    Ok(())
}

fn manifest_role_row(
    role: &'static str,
    value: &str,
    agent_configs: &BTreeMap<String, AgentConfig>,
) -> ManifestRoleRow {
    let spec = match agents::parse_spec(value) {
        Ok(spec) => spec,
        Err(err) => return invalid_manifest_role_row(role, value, &err.to_string()),
    };

    if let Err(err) = agents::canonical_manifest_agent(&spec, agent_configs) {
        return invalid_manifest_role_row(role, value, &err.to_string());
    }

    ManifestRoleRow {
        role,
        family: display_cell(spec.family()),
        model: display_spec_part(spec.model()),
        effort: display_spec_part(spec.effort()),
        note: None,
    }
}

fn display_spec_part(value: Option<&str>) -> String {
    match value {
        Some(value) => display_cell(value),
        None => MISSING_SPEC_PART.to_owned(),
    }
}

fn invalid_manifest_role_row(role: &'static str, value: &str, error: &str) -> ManifestRoleRow {
    ManifestRoleRow {
        role,
        family: display_cell(value),
        model: MISSING_SPEC_PART.to_owned(),
        effort: MISSING_SPEC_PART.to_owned(),
        note: Some(display_note(&format!("invalid: {error}"))),
    }
}

fn write_padded<W: Write>(output: &mut W, value: &str, width: usize) -> Result<()> {
    write!(output, "{value}")?;
    for _ in 0..width.saturating_sub(display_width(value)) {
        write!(output, " ")?;
    }
    Ok(())
}

fn display_cell(value: &str) -> String {
    display_value(value, MAX_TABLE_CELL_WIDTH)
}

pub(super) fn display_note(value: &str) -> String {
    display_value(value, MAX_TABLE_NOTE_WIDTH)
}

fn display_value(value: &str, max_width: usize) -> String {
    let escaped = escape_control_chars(value);
    truncate_display(&escaped, max_width)
}

fn escape_control_chars(value: &str) -> String {
    let mut escaped = String::new();
    for character in value.chars() {
        if character.is_control() {
            escaped.extend(character.escape_debug());
        } else {
            escaped.push(character);
        }
    }
    escaped
}

fn truncate_display(value: &str, max_width: usize) -> String {
    if display_width(value) <= max_width {
        return value.to_owned();
    }

    let marker_width = display_width(TRUNCATION_MARKER);
    let mut truncated = String::new();
    let mut width = 0;
    for character in value.chars() {
        let character_width = UnicodeWidthChar::width(character).map_or(0, |width| width);
        if width + character_width + marker_width > max_width {
            break;
        }
        truncated.push(character);
        width += character_width;
    }
    truncated.push_str(TRUNCATION_MARKER);
    truncated
}

fn display_width(value: &str) -> usize {
    UnicodeWidthStr::width(value)
}

struct ManifestRoleRow {
    role: &'static str,
    family: String,
    model: String,
    effort: String,
    note: Option<String>,
}

struct TableWidths {
    role: usize,
    family: usize,
    model: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::config::spec::PromptMode;

    #[test]
    fn manifest_roles_render_invalid_bindings_without_failing() -> Result<()> {
        let manifest = WorkspaceManifest {
            manager: "codex::xhigh".to_owned(),
            planner: "ghost".to_owned(),
            worker: "codex".to_owned(),
            reviewer: "claude:haiku".to_owned(),
            validation_command: "cargo clippy --all-targets -- -D warnings".to_owned(),
            flow: super::super::initial_flow(),
        };
        let mut output = Vec::new();

        print_manifest_roles(&mut output, &manifest, &BTreeMap::new())?;

        assert_eq!(
            String::from_utf8(output)?,
            "\
manager     codex::xhigh  -      -  invalid: invalid agent spec `codex::xhigh`; family, model, and effort cannot be empty
planner     ghost         -      -  invalid: unknown agent `ghost`; configure it in niles.yaml or choose a builtin agent
worker      codex         -      -
reviewer    claude        haiku  -
validation  cargo clippy --all-targets -- -D warnings
"
        );
        Ok(())
    }

    #[test]
    fn manifest_roles_escape_control_characters() -> Result<()> {
        let planner = "\u{1b}[1A\u{1b}[2Kmanager     claude  opus  max";
        let worker = "codex\nreviewer  forged  opus  max";
        let mut agent_configs = BTreeMap::new();
        agent_configs.insert(planner.to_owned(), agent_config());
        agent_configs.insert(worker.to_owned(), agent_config());
        let manifest = WorkspaceManifest {
            manager: "codex:gpt-5.5:xhigh".to_owned(),
            planner: planner.to_owned(),
            worker: worker.to_owned(),
            reviewer: "claude:opus:max".to_owned(),
            validation_command: "cargo test\u{1b}]0;pwned\u{7}\r\nvalidation forged".to_owned(),
            flow: super::super::initial_flow(),
        };
        let mut output = Vec::new();

        print_manifest_roles(&mut output, &manifest, &agent_configs)?;

        let output = String::from_utf8(output)?;
        assert!(output.contains("\\u{1b}[1A\\u{1b}[2Kmanager"));
        assert!(output.contains("codex\\nreviewer  forged"));
        assert!(output.contains("cargo test\\u{1b}]0;pwned\\u{7}\\r\\nvalidation forged"));
        assert_no_controls_inside_rows(&output);
        Ok(())
    }

    #[test]
    fn manifest_roles_do_not_create_rows_from_value_newlines() -> Result<()> {
        let worker = "codex\nreviewer  forged  opus  max";
        let mut agent_configs = BTreeMap::new();
        agent_configs.insert(worker.to_owned(), agent_config());
        let manifest = WorkspaceManifest {
            manager: "codex".to_owned(),
            planner: "claude".to_owned(),
            worker: worker.to_owned(),
            reviewer: "claude".to_owned(),
            validation_command: "cargo test\nmanager forged".to_owned(),
            flow: super::super::initial_flow(),
        };
        let mut output = Vec::new();

        print_manifest_roles(&mut output, &manifest, &agent_configs)?;

        let output = String::from_utf8(output)?;
        assert_eq!(output.lines().count(), 5);
        assert!(output.contains("codex\\nreviewer  forged"));
        assert!(output.contains("cargo test\\nmanager forged"));
        Ok(())
    }

    #[test]
    fn manifest_roles_truncate_overlong_values_without_panicking() -> Result<()> {
        let long_family = "x".repeat(70_000);
        let mut agent_configs = BTreeMap::new();
        agent_configs.insert(long_family.clone(), agent_config());
        let manifest = WorkspaceManifest {
            manager: "codex".to_owned(),
            planner: long_family,
            worker: "codex".to_owned(),
            reviewer: "claude".to_owned(),
            validation_command: "cargo test".to_owned(),
            flow: super::super::initial_flow(),
        };
        let mut output = Vec::new();

        print_manifest_roles(&mut output, &manifest, &agent_configs)?;

        let output = String::from_utf8(output)?;
        assert!(output.len() < 1_000);
        assert!(output.contains(TRUNCATION_MARKER));
        Ok(())
    }

    fn agent_config() -> AgentConfig {
        AgentConfig {
            binary: None,
            args: Vec::new(),
            prompt: PromptMode::Arg,
        }
    }

    fn assert_no_controls_inside_rows(output: &str) {
        assert_eq!(output.lines().count(), 5);
        for line in output.lines() {
            assert!(
                !line.chars().any(char::is_control),
                "line contains a control character: {line:?}"
            );
        }
    }
}
