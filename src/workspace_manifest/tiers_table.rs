use std::io::Write;

use anyhow::Result;

use crate::{agents::roster::Tier, config::spec::ProjectConfig};

use super::{TierBinding, WorkspaceManifest};

/// Manifest values are arbitrary strings from a file on disk. Capping every
/// cell keeps a runaway value from wrecking the layout, and keeps the padding
/// width well under the `u16` a runtime format width is stored in — past
/// `u16::MAX` the format call panics.
const MAX_CELL: usize = 40;
const MAX_REASON: usize = 68;
/// An absent model or effort in the display.
pub(crate) const MISSING: &str = "-";

pub(crate) fn print_manifest_tiers<W: Write>(
    output: &mut W,
    manifest: &WorkspaceManifest,
    config: &ProjectConfig,
) -> Result<()> {
    let rows = manifest_tiers(manifest, config);
    let tier_width = rows
        .iter()
        .map(|row| cells(&cell(row.tier.as_str())))
        .fold(0, usize::max);
    let family_width = rows
        .iter()
        .map(|row| cells(&row.family))
        .fold(0, usize::max);
    let model_width = rows.iter().map(|row| cells(&row.model)).fold(0, usize::max);

    for row in &rows {
        writeln!(
            output,
            "{:<tier_width$}  {:<family_width$}  {:<model_width$}  {}",
            cell(row.tier.as_str()),
            row.family,
            row.model,
            row.effort,
        )?;
        if let Some(reason) = &row.invalid_reason {
            writeln!(output, "{:<tier_width$}  reason: {reason}", "")?;
        }
    }

    Ok(())
}

pub(crate) fn manifest_tiers(manifest: &WorkspaceManifest, config: &ProjectConfig) -> Vec<TierRow> {
    manifest
        .tiers
        .iter()
        .map(|(tier, binding)| tier_row(tier, binding, config))
        .collect()
}

pub(crate) struct TierRow {
    pub tier: Tier,
    pub family: String,
    pub model: String,
    pub effort: String,
    pub secondaries: String,
    pub invalid_reason: Option<String>,
}

fn tier_row(tier: Tier, binding: &TierBinding, config: &ProjectConfig) -> TierRow {
    match super::types::exact_spec(&binding.primary, &config.models) {
        Ok(spec) => TierRow {
            tier,
            family: cell(spec.family()),
            model: spec.model().map_or_else(|| MISSING.to_owned(), cell),
            effort: spec.effort().map_or_else(|| MISSING.to_owned(), cell),
            secondaries: binding.secondaries.join(", "),
            invalid_reason: None,
        },
        Err(err) => TierRow {
            tier,
            family: cell(&binding.primary),
            model: MISSING.to_owned(),
            effort: MISSING.to_owned(),
            secondaries: binding.secondaries.join(", "),
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

    fn manifest(primary: &str) -> WorkspaceManifest {
        let mut manifest = crate::test_support::manifest();
        manifest.tiers.standard.primary = primary.to_owned();
        manifest
    }

    fn render(manifest: &WorkspaceManifest) -> String {
        let mut output = Vec::new();
        let config = ProjectConfig {
            models: agents::ModelRoster::builtin().unwrap(),
        };
        print_manifest_tiers(&mut output, manifest, &config).unwrap();
        String::from_utf8(output).unwrap()
    }

    #[test]
    fn renders_the_table_a_user_actually_sees() {
        let rendered = render(&manifest("claude:opus:max"));
        let rows = rendered
            .lines()
            .map(|line| line.split_whitespace().collect::<Vec<_>>())
            .collect::<Vec<_>>();
        assert_eq!(rows[0], ["standard", "claude", "opus", "max"]);
        assert_eq!(rows[1], ["frontier", "claude", "opus", "high"]);
        assert_eq!(rows[2], ["economy", "codex", "gpt-6-luna", "medium"]);
        assert_eq!(rows[3], ["mechanical", "codex", "gpt-6-luna", "low"]);
    }

    #[test]
    fn an_unpinned_model_or_effort_is_not_invented() {
        // The display names inexact input rather than inventing a launch choice.
        let rendered = render(&manifest("codex"));

        // Cells, not spacing: padding shifts whenever another tier gets wider.
        let row = rendered
            .lines()
            .find(|line| line.starts_with("standard"))
            .expect("standard row missing from rendered manifest");
        assert_eq!(
            row.split_whitespace().collect::<Vec<_>>(),
            ["standard", "codex", "-", "-"],
            "{rendered}"
        );
        assert!(rendered.contains("must name an exact"));
    }

    #[test]
    fn an_invalid_tier_says_why_instead_of_aborting() {
        let rendered = render(&manifest("ghost"));

        assert!(rendered.contains("reason: "), "{rendered}");
        assert!(rendered.contains("ghost"), "{rendered}");
    }

    #[test]
    fn a_newline_cannot_forge_an_extra_row() {
        let rendered = render(&manifest("claude\nreviewer  totally  fake  max"));

        // Continuation lines are indented, so a tier row is one that starts at column zero.
        let tier_rows = rendered
            .lines()
            .filter(|line| !line.starts_with(' '))
            .count();
        assert_eq!(tier_rows, 4, "{rendered}");
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
        let manifest = manifest(&"q".repeat(70_000));
        let rendered = render(&manifest);
        assert_eq!(
            rendered
                .lines()
                .filter(|line| !line.starts_with(' '))
                .count(),
            manifest.tiers.iter().count()
        );

        assert!(
            rendered.lines().all(|line| line.chars().count() < 200),
            "longest line: {}",
            rendered.lines().map(|l| l.chars().count()).max().unwrap()
        );
    }

    #[test]
    fn a_long_model_name_widens_every_row_together() {
        let rendered = render(&manifest("pi:deepseek/deepseek-v4.1-flash:high"));

        assert!(
            rendered.contains("deepseek/deepseek-v4.1-flash"),
            "{rendered}"
        );
        let effort_columns: Vec<Option<usize>> =
            rendered.lines().map(|line| line.rfind("  ")).collect();
        assert_eq!(effort_columns.len(), 4);
        assert!(
            effort_columns.windows(2).all(|pair| pair[0] == pair[1]),
            "{rendered}"
        );
    }
}
