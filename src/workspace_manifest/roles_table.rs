use std::{collections::BTreeMap, io::Write};

use anyhow::Result;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::{agents, config::spec::AgentConfig};

use super::WorkspaceManifest;

const MAX_FAMILY_WIDTH: usize = 22;
const MAX_MODEL_WIDTH: usize = 28;
const MAX_TABLE_NOTE_WIDTH: usize = 68;
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
        family: display_family(spec.family()),
        model: display_spec_part(spec.model()),
        effort: display_spec_part(spec.effort()),
        note: None,
    }
}

fn display_spec_part(value: Option<&str>) -> String {
    match value {
        Some(value) => display_model(value),
        None => MISSING_SPEC_PART.to_owned(),
    }
}

fn invalid_manifest_role_row(role: &'static str, value: &str, _error: &str) -> ManifestRoleRow {
    ManifestRoleRow {
        role,
        family: display_family(value),
        model: MISSING_SPEC_PART.to_owned(),
        effort: MISSING_SPEC_PART.to_owned(),
        note: Some("invalid".to_owned()),
    }
}

fn write_padded<W: Write>(output: &mut W, value: &str, width: usize) -> Result<()> {
    write!(output, "{value}")?;
    for _ in 0..width.saturating_sub(display_width(value)) {
        write!(output, " ")?;
    }
    Ok(())
}

fn display_family(value: &str) -> String {
    display_value(value, MAX_FAMILY_WIDTH, MAX_FAMILY_WIDTH)
}

fn display_model(value: &str) -> String {
    display_value(value, MAX_MODEL_WIDTH, MAX_MODEL_WIDTH)
}

pub(super) fn display_note(value: &str) -> String {
    display_value(value, MAX_TABLE_NOTE_WIDTH, MAX_TABLE_NOTE_WIDTH)
}

fn display_value(value: &str, max_width: usize, max_chars: usize) -> String {
    let escaped = escape_inert_chars(value);
    truncate_display(&escaped, max_width, max_chars)
}

fn escape_inert_chars(value: &str) -> String {
    let mut escaped = String::new();
    for character in value.chars() {
        if character.is_control() {
            escaped.extend(character.escape_debug());
        } else if is_unicode_format_or_separator(character) {
            escaped.extend(character.escape_unicode());
        } else {
            escaped.push(character);
        }
    }
    escaped
}

fn is_unicode_format_or_separator(character: char) -> bool {
    matches!(
        character,
        '\u{00ad}'
            | '\u{0600}'..='\u{0605}'
            | '\u{061c}'
            | '\u{06dd}'
            | '\u{070f}'
            | '\u{0890}'..='\u{0891}'
            | '\u{08e2}'
            | '\u{180e}'
            | '\u{200b}'..='\u{200f}'
            | '\u{2028}'..='\u{202e}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{206f}'
            | '\u{feff}'
            | '\u{fff9}'..='\u{fffb}'
            | '\u{110bd}'
            | '\u{110cd}'
            | '\u{13430}'..='\u{1343f}'
            | '\u{1bca0}'..='\u{1bca3}'
            | '\u{1d173}'..='\u{1d17a}'
            | '\u{e0001}'
            | '\u{e0020}'..='\u{e007f}'
    )
}

fn truncate_display(value: &str, max_width: usize, max_chars: usize) -> String {
    if display_width(value) <= max_width && display_chars(value) <= max_chars {
        return value.to_owned();
    }

    let marker_width = display_width(TRUNCATION_MARKER);
    let marker_chars = display_chars(TRUNCATION_MARKER);
    let mut truncated = String::new();
    let mut width = 0;
    for (chars, character) in value.chars().enumerate() {
        #[expect(
            clippy::disallowed_methods,
            reason = "characters without terminal width occupy zero display columns in this table"
        )]
        let character_width: usize = UnicodeWidthChar::width(character).unwrap_or_default();
        if width + character_width + marker_width > max_width
            || chars + 1 + marker_chars > max_chars
        {
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

fn display_chars(value: &str) -> usize {
    value.chars().count()
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
manager     codex::xhigh  -      -  invalid
planner     ghost         -      -  invalid
worker      codex         -      -
reviewer    claude        haiku  -
validation  cargo clippy --all-targets -- -D warnings
"
        );
        Ok(())
    }

    #[test]
    fn manifest_roles_escape_control_characters() -> Result<()> {
        let planner = "\u{202e}\u{1b}[1A\u{1b}[2Kmanager     claude  opus  max";
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
        assert!(output.contains("\\u{202e}\\u{1b}"));
        assert!(output.contains("codex\\nreviewer"));
        assert!(output.contains("cargo test\\u{1b}]0;pwned\\u{7}\\r\\nvalidation forged"));
        assert_no_inert_characters_inside_rows(&output);
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
        assert!(output.contains("codex\\nreviewer"));
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

    #[test]
    fn manifest_roles_bound_zero_width_amplification() -> Result<()> {
        let zero_width_family = "\u{200b}".repeat(50_000);
        let combining_family = "\u{0301}".repeat(50_000);
        let mut agent_configs = BTreeMap::new();
        agent_configs.insert(zero_width_family.clone(), agent_config());
        agent_configs.insert(combining_family.clone(), agent_config());
        let manifest = WorkspaceManifest {
            manager: "codex".to_owned(),
            planner: zero_width_family,
            worker: combining_family,
            reviewer: "claude".to_owned(),
            validation_command: "\u{feff}".repeat(50_000),
            flow: super::super::initial_flow(),
        };
        let mut output = Vec::new();

        print_manifest_roles(&mut output, &manifest, &agent_configs)?;

        let output = String::from_utf8(output)?;
        assert!(output.len() < 1_000);
        assert!(output.contains(TRUNCATION_MARKER));
        assert!(!output.contains('\u{200b}'));
        assert!(!output.contains('\u{feff}'));
        assert_no_inert_characters_inside_rows(&output);
        Ok(())
    }

    #[test]
    fn invalid_rows_fit_a_normal_pane() -> Result<()> {
        let long_invalid = format!("{}manager     claude  opus  max", "q".repeat(120));
        let manifest = WorkspaceManifest {
            manager: "codex:claude-haiku-4-5-20251001:medium".to_owned(),
            planner: long_invalid,
            worker: "codex".to_owned(),
            reviewer: "claude".to_owned(),
            validation_command: "cargo test".to_owned(),
            flow: super::super::initial_flow(),
        };
        let mut output = Vec::new();

        print_manifest_roles(&mut output, &manifest, &BTreeMap::new())?;

        let output = String::from_utf8(output)?;
        for line in output.lines() {
            assert!(display_width(line) <= 80, "wide row: {line}");
        }
        Ok(())
    }

    fn agent_config() -> AgentConfig {
        AgentConfig {
            binary: None,
            args: Vec::new(),
            prompt: PromptMode::Arg,
        }
    }

    fn assert_no_inert_characters_inside_rows(output: &str) {
        assert_eq!(output.lines().count(), 5);
        for line in output.lines() {
            assert!(
                !line.chars().any(char::is_control),
                "line contains a control character: {line:?}"
            );
            assert!(
                !line.chars().any(is_unicode_format_or_separator),
                "line contains a format/separator character: {line:?}"
            );
        }
    }
}
