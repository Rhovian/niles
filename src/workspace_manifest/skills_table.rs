use std::io::Write;

use anyhow::Result;

use crate::{config::spec::ProjectConfig, skills::SkillName};

use super::{Binding, WorkspaceManifest};

/// Manifest values are arbitrary strings from a file on disk. Capping every
/// cell keeps a runaway value from wrecking the layout, and keeps the padding
/// width well under the `u16` a runtime format width is stored in — past
/// `u16::MAX` the format call panics.
const MAX_CELL: usize = 40;
const MAX_REASON: usize = 68;
/// A model or effort the binding leaves to the builtin.
pub(crate) const MISSING: &str = "-";

pub(crate) fn print_manifest_skills<W: Write>(
    output: &mut W,
    manifest: &WorkspaceManifest,
    config: &ProjectConfig,
) -> Result<()> {
    let rows = manifest_skills(manifest, config);
    let skill_width = rows
        .iter()
        .map(|row| cells(&cell(row.skill.as_str())))
        .fold(0, usize::max);
    let family_width = rows
        .iter()
        .map(|row| cells(&row.family))
        .fold(0, usize::max);
    let model_width = rows.iter().map(|row| cells(&row.model)).fold(0, usize::max);

    for row in &rows {
        writeln!(
            output,
            "{:<skill_width$}  {:<family_width$}  {:<model_width$}  {}",
            cell(row.skill.as_str()),
            row.family,
            row.model,
            row.effort,
        )?;
        if let Some(reason) = &row.invalid_reason {
            writeln!(output, "{:<skill_width$}  reason: {reason}", "")?;
        }
    }

    Ok(())
}

pub(crate) fn manifest_skills(
    manifest: &WorkspaceManifest,
    config: &ProjectConfig,
) -> Vec<SkillRow> {
    manifest
        .ordered_bindings()
        .map(|(skill, binding)| binding_row(skill, binding, config))
        .collect()
}

pub(crate) struct SkillRow {
    pub skill: SkillName,
    pub family: String,
    pub model: String,
    pub effort: String,
    pub invalid_reason: Option<String>,
}

fn binding_row(skill: &SkillName, binding: &Binding, config: &ProjectConfig) -> SkillRow {
    match super::types::exact_spec(&binding.standard.primary, &config.models) {
        Ok(spec) => SkillRow {
            skill: skill.clone(),
            family: cell(spec.family()),
            model: spec.model().map_or_else(|| MISSING.to_owned(), cell),
            effort: spec.effort().map_or_else(|| MISSING.to_owned(), cell),
            invalid_reason: None,
        },
        Err(err) => SkillRow {
            skill: skill.clone(),
            family: cell(&binding.standard.primary),
            model: MISSING.to_owned(),
            effort: MISSING.to_owned(),
            invalid_reason: Some(clamp(&err.to_string(), MAX_REASON)),
        },
    }
}

fn cell(value: &str) -> String {
    clamp(value, MAX_CELL)
}

/// Render a manifest value as one inert line. Control characters are dropped
/// rather than escaped: a bare newline forges a whole extra table row, and an
/// ESC sequence rewrites rows already printed.
pub(crate) fn clamp(value: &str, max: usize) -> String {
    let kept: String = value
        .chars()
        .filter(|c| !c.is_control())
        .take(max)
        .collect();
    if kept.len() != value.len() {
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
    use crate::agents;

    fn manifest(reviewer: &str) -> WorkspaceManifest {
        let mut manifest = WorkspaceManifest::default();
        manifest
            .bindings
            .insert("reviewer".parse().unwrap(), reviewer.to_owned().into());
        manifest
    }

    fn render(manifest: &WorkspaceManifest) -> String {
        let mut output = Vec::new();
        let config = ProjectConfig {
            models: agents::ModelRoster::builtin().unwrap(),
        };
        print_manifest_skills(&mut output, manifest, &config).unwrap();
        String::from_utf8(output).unwrap()
    }

    #[test]
    fn renders_the_table_a_user_actually_sees() {
        let rendered = render(&manifest("claude:opus:max"));
        let rows = rendered
            .lines()
            .map(|line| line.split_whitespace().collect::<Vec<_>>())
            .collect::<Vec<_>>();
        assert_eq!(rows[0], ["lead", "claude", "sonnet", "high"]);
        assert_eq!(rows[1], ["design", "claude", "sonnet", "high"]);
        assert_eq!(rows[2], ["research", "codex", "gpt-6.1-sol", "medium"]);
        assert_eq!(rows[3], ["reviewer", "claude", "opus", "max"]);
        assert_eq!(rows[5], ["worker", "codex", "gpt-6.1-sol", "medium"]);
    }

    #[test]
    fn an_unpinned_model_or_effort_is_not_invented() {
        // The display names inexact input rather than inventing a launch choice.
        let rendered = render(&manifest("codex"));

        // Cells, not spacing: padding shifts whenever another binding gets wider.
        let row = rendered
            .lines()
            .find(|line| line.starts_with("reviewer"))
            .expect("reviewer row missing from rendered manifest");
        assert_eq!(
            row.split_whitespace().collect::<Vec<_>>(),
            ["reviewer", "codex", "-", "-"],
            "{rendered}"
        );
        assert!(rendered.contains("must name an exact"));
    }

    #[test]
    fn an_invalid_binding_says_why_instead_of_aborting() {
        let rendered = render(&manifest("ghost"));

        assert!(rendered.contains("reason: "), "{rendered}");
        assert!(rendered.contains("ghost"), "{rendered}");
    }

    #[test]
    fn a_newline_cannot_forge_an_extra_row() {
        let rendered = render(&manifest("claude\nreviewer  totally  fake  max"));

        // Continuation lines are indented, so a skill row is one that starts at column zero.
        let skill_rows = rendered
            .lines()
            .filter(|line| !line.starts_with(' '))
            .count();
        assert_eq!(skill_rows, 6, "{rendered}");
        assert!(!rendered.contains("\nreviewer  totally"), "{rendered}");
    }

    #[test]
    fn escape_sequences_cannot_rewrite_rows_already_printed() {
        let rendered = render(&manifest(
            "\u{1b}[1A\u{1b}[2Klead  claude  opus  max\u{1b}]0;pwned\u{7}",
        ));

        assert!(!rendered.contains('\u{1b}'), "{rendered}");
        assert!(!rendered.contains('\u{7}'), "{rendered}");
    }

    #[test]
    fn an_enormous_value_is_clamped_rather_than_panicking() {
        // A runtime format width is a u16; past 65535 the format call panics.
        let mut manifest = manifest(&"q".repeat(70_000));
        manifest.bindings.insert(
            "s".repeat(70_000).try_into().unwrap(),
            "claude:sonnet:high".to_owned().into(),
        );
        let rendered = render(&manifest);
        assert_eq!(
            rendered
                .lines()
                .filter(|line| !line.starts_with(' '))
                .count(),
            manifest.bindings.len()
        );

        assert!(
            rendered.lines().all(|line| line.chars().count() < 200),
            "longest line: {}",
            rendered.lines().map(|l| l.chars().count()).max().unwrap()
        );
    }

    #[test]
    fn a_long_model_name_widens_every_row_together() {
        let rendered = render(&manifest("hermes:deepseek/deepseek-v4.1-flash:high"));

        assert!(
            rendered.contains("deepseek/deepseek-v4.1-flash"),
            "{rendered}"
        );
        let effort_columns: Vec<Option<usize>> = rendered
            .lines()
            .take(5)
            .map(|line| line.rfind("  "))
            .collect();
        assert!(
            effort_columns.windows(2).all(|pair| pair[0] == pair[1]),
            "{rendered}"
        );
    }
}
