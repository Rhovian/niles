use std::{collections::BTreeMap, io::Write};

use anyhow::Result;

use crate::{agents, config::spec::AgentConfig};

use super::WorkspaceManifest;

/// Manifest values are arbitrary strings from a file on disk. Capping every
/// cell keeps a runaway value from wrecking the layout, and keeps the padding
/// width well under the `u16` a runtime format width is stored in — past
/// `u16::MAX` the format call panics.
const MAX_CELL: usize = 40;
const MAX_REASON: usize = 68;
const MISSING: &str = "-";
const VALIDATION_ROLE: &str = "validation";

pub(super) fn print_manifest_roles<W: Write>(
    output: &mut W,
    manifest: &WorkspaceManifest,
    agent_configs: &BTreeMap<String, AgentConfig>,
) -> Result<()> {
    let rows = [
        role_row("manager", &manifest.manager, agent_configs),
        role_row("planner", &manifest.planner, agent_configs),
        role_row("worker", &manifest.worker, agent_configs),
        role_row("reviewer", &manifest.reviewer, agent_configs),
    ];

    let role_width = rows
        .iter()
        .map(|row| row.role.len())
        .fold(VALIDATION_ROLE.len(), usize::max);
    let family_width = rows
        .iter()
        .map(|row| cells(&row.family))
        .fold(0, usize::max);
    let model_width = rows.iter().map(|row| cells(&row.model)).fold(0, usize::max);

    for row in &rows {
        writeln!(
            output,
            "{:<role_width$}  {:<family_width$}  {:<model_width$}  {}",
            row.role, row.family, row.model, row.effort,
        )?;
        if let Some(reason) = &row.invalid_reason {
            writeln!(output, "{:<role_width$}  reason: {reason}", "")?;
        }
    }

    writeln!(
        output,
        "{:<role_width$}  {}",
        VALIDATION_ROLE,
        display_note(&manifest.validation_command)
    )?;

    Ok(())
}

struct RoleRow {
    role: &'static str,
    family: String,
    model: String,
    effort: String,
    invalid_reason: Option<String>,
}

fn role_row(
    role: &'static str,
    value: &str,
    agent_configs: &BTreeMap<String, AgentConfig>,
) -> RoleRow {
    // Validate through the same contract the picker writes with, so a binding
    // niles would refuse to launch is flagged here, where it can be fixed.
    let spec = agents::parse_spec(value)
        .and_then(|spec| agents::canonical_manifest_agent(&spec, agent_configs).map(|_| spec));

    match spec {
        Ok(spec) => RoleRow {
            role,
            family: cell(spec.family()),
            model: spec.model().map_or_else(|| MISSING.to_owned(), cell),
            effort: spec.effort().map_or_else(|| MISSING.to_owned(), cell),
            invalid_reason: None,
        },
        Err(err) => RoleRow {
            role,
            family: cell(value),
            model: MISSING.to_owned(),
            effort: MISSING.to_owned(),
            invalid_reason: Some(clamp(&err.to_string(), MAX_REASON)),
        },
    }
}

fn cell(value: &str) -> String {
    clamp(value, MAX_CELL)
}

pub(super) fn display_note(value: &str) -> String {
    clamp(value, MAX_REASON)
}

/// Render a manifest value as one inert line. Control characters are dropped
/// rather than escaped: a bare newline forges a whole extra table row, and an
/// ESC sequence rewrites rows already printed.
fn clamp(value: &str, max: usize) -> String {
    let kept: String = value
        .chars()
        .filter(|c| !c.is_control())
        .take(max)
        .collect();
    let dropped_anything = cells(&kept) < value.chars().filter(|c| !c.is_control()).count()
        || value.chars().any(char::is_control);
    if dropped_anything {
        format!("{kept}…")
    } else {
        kept
    }
}

/// Character count, not bytes — `{:<width$}` pads by characters too, so the
/// two agree and a non-ASCII agent name still lines up.
fn cells(value: &str) -> usize {
    value.chars().count()
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::workspace_manifest::initial_flow;

    fn manifest(planner: &str, validation: &str) -> WorkspaceManifest {
        WorkspaceManifest {
            manager: "codex:gpt-5.5:xhigh".to_owned(),
            planner: planner.to_owned(),
            worker: "codex".to_owned(),
            reviewer: "claude:opus:max".to_owned(),
            validation_command: validation.to_owned(),
            flow: initial_flow(),
        }
    }

    fn render(manifest: &WorkspaceManifest) -> String {
        let mut output = Vec::new();
        print_manifest_roles(&mut output, manifest, &BTreeMap::new()).unwrap();
        String::from_utf8(output).unwrap()
    }

    #[test]
    fn renders_the_table_a_user_actually_sees() {
        assert_eq!(
            render(&manifest("claude:opus:high", "cargo test")),
            "\
manager     codex   gpt-5.5  xhigh
planner     claude  opus     high
worker      codex   -        -
reviewer    claude  opus     max
validation  cargo test
"
        );
    }

    #[test]
    fn an_unpinned_model_or_effort_is_not_invented() {
        // A bare `codex` must not be resolved into whatever it would launch as.
        let rendered = render(&manifest("codex", "cargo test"));

        assert!(
            rendered.contains("planner     codex   -        -"),
            "{rendered}"
        );
    }

    #[test]
    fn an_invalid_binding_says_why_instead_of_aborting() {
        let rendered = render(&manifest("ghost", "cargo test"));

        assert!(rendered.contains("reason: "), "{rendered}");
        assert!(rendered.contains("ghost"), "{rendered}");
    }

    #[test]
    fn a_newline_cannot_forge_an_extra_row() {
        let rendered = render(&manifest(
            "claude",
            "cargo test\nreviewer  totally  fake  max",
        ));

        assert_eq!(rendered.lines().count(), 5, "{rendered}");
        assert!(!rendered.contains("\nreviewer  totally"), "{rendered}");
    }

    #[test]
    fn escape_sequences_cannot_rewrite_rows_already_printed() {
        let rendered = render(&manifest(
            "\u{1b}[1A\u{1b}[2Kmanager  claude  opus  max",
            "cargo test\u{1b}]0;pwned\u{7}",
        ));

        assert!(!rendered.contains('\u{1b}'), "{rendered}");
        assert!(!rendered.contains('\u{7}'), "{rendered}");
    }

    #[test]
    fn an_enormous_value_is_clamped_rather_than_panicking() {
        // A runtime format width is a u16; past 65535 the format call panics.
        let rendered = render(&manifest(&"q".repeat(70_000), &"v".repeat(70_000)));

        assert!(
            rendered.lines().all(|line| line.chars().count() < 200),
            "longest line: {}",
            rendered.lines().map(|l| l.chars().count()).max().unwrap()
        );
    }

    #[test]
    fn a_long_model_name_widens_every_row_together() {
        let rendered = render(&manifest(
            "claude:claude-haiku-4-5-20251001:medium",
            "cargo test",
        ));

        assert!(rendered.contains("claude-haiku-4-5-20251001"), "{rendered}");
        let effort_columns: Vec<Option<usize>> = rendered
            .lines()
            .take(4)
            .map(|line| line.rfind("  "))
            .collect();
        assert!(
            effort_columns.windows(2).all(|pair| pair[0] == pair[1]),
            "{rendered}"
        );
    }
}
