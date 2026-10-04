use std::{fmt::Write, fs};

use anyhow::{Context, Result};
use clap::ValueEnum;

use super::{registry, rows};
use crate::{
    config::spec::PROJECT_CONFIG_FILES,
    session, tmux,
    worker::usage::{self, SessionUsage},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum Panel {
    Config,
    Telemetry,
    Help,
}

impl Panel {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Config => "config",
            Self::Telemetry => "telemetry",
            Self::Help => "help",
        }
    }
}

pub(crate) fn render(panel: Panel) -> Result<()> {
    let mut text = String::new();
    let entries = registry::entries()?;
    match panel {
        Panel::Help => text.push_str(help_text(entries.is_empty())),
        Panel::Config => {
            for entry in entries {
                text.push_str(&config(&entry)?);
            }
        }
        Panel::Telemetry => {
            for entry in entries {
                let session = entry.name.session()?;
                let lead = if tmux::project_session(&session)?.as_deref()
                    == Some(entry.path.as_str())
                    && tmux::lead_running(&session)?
                {
                    session::latest_lead(&entry.path)?
                } else {
                    None
                };
                let sessions = usage::collect(&entry.path, lead)?;
                if !sessions.is_empty() {
                    writeln!(text, "{}  {}", entry.name.as_str(), entry.path)?;
                    text.push_str(&telemetry(&sessions)?);
                }
            }
        }
    }
    print!("{text}");
    Ok(())
}

const FIRST_RUN: &str = "niles

Coordinates coding agents from different model families in tmux.

Get started
  r    register a project directory, in the explorer on the left
  ↵    start its lead and show it here

Key bindings for the home view: https://github.com/Rhovian/niles/blob/main/docs/setup.md
";

const KEYS: &str = "Choose a project on the left.

  ↵      open                 → ←    expand / collapse
  [ ]    previous / next project
  ; '    previous / next window
  r      register a project   q      quiet a worker
  c      close a worker or project

  ⣾ running   ⚠ waiting

With the bindings in docs/setup.md, M-[ M-] M-; M-' do the same from any pane.
";

fn help_text(first_run: bool) -> &'static str {
    if first_run { FIRST_RUN } else { KEYS }
}

fn config(entry: &registry::Entry) -> Result<String> {
    let mut text = format!("{}  {}\n", entry.name.as_str(), entry.path);
    for file in std::iter::once(".niles/manifest.yaml").chain(PROJECT_CONFIG_FILES) {
        let path = entry.path.join(file);
        if !path.try_exists()? {
            if file == ".niles/manifest.yaml" {
                writeln!(text, "{file} missing")?;
            }
            continue;
        }
        let body = fs::read_to_string(&path).with_context(|| format!("failed to read {path}"))?;
        writeln!(text, "{file}\n{}", body.trim_end_matches('\n'))?;
    }
    text.push('\n');
    Ok(text)
}

fn telemetry(sessions: &[SessionUsage]) -> Result<String> {
    let mut text = String::new();
    let mut tokens = 0;
    let mut cost_total = None;
    for session in sessions {
        write!(
            text,
            "{}  {}  {}  ",
            session.id, session.role, session.agent
        )?;
        match &session.usage {
            Some(usage) => {
                let total = usage.total_tokens();
                tokens += total;
                write!(text, "{} tokens", rows::abbreviate(total))?;
                if let Some(cost) = usage.estimated_cost_usd {
                    cost_total = Some(cost_total.map_or(cost, |total| total + cost));
                    write!(text, "  ${cost:.4}")?;
                }
            }
            None => text.push_str("tokens unknown"),
        }
        text.push('\n');
    }
    write!(text, "total  {} tokens", rows::abbreviate(tokens))?;
    if let Some(cost) = cost_total {
        write!(text, "  ${cost:.4}")?;
    }
    text.push_str("\n\n");
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{projects::registry::ProjectName, test_support::temp_test_path};

    #[test]
    fn help_text_depends_on_whether_registry_is_empty() {
        assert_eq!(help_text(true), FIRST_RUN);
        assert_eq!(help_text(false), KEYS);
    }

    #[test]
    fn config_prints_files_and_missing_manifests() {
        let path = temp_test_path("panel-config");
        let entry = registry::Entry {
            name: ProjectName::parse("api").unwrap(),
            path,
        };
        let heading = format!("api  {}\n", entry.path);
        assert_eq!(
            config(&entry).unwrap(),
            format!("{heading}.niles/manifest.yaml missing\n\n")
        );
        fs::create_dir_all(entry.path.join(".niles")).unwrap();
        for file in [".niles/manifest.yaml", "niles.yaml", ".niles.yaml"] {
            fs::write(entry.path.join(file), "agents: {}\n").unwrap();
        }
        assert_eq!(
            config(&entry).unwrap(),
            format!(
                "{heading}.niles/manifest.yaml\nagents: {{}}\nniles.yaml\nagents: {{}}\n.niles.yaml\nagents: {{}}\n\n"
            )
        );
        fs::remove_dir_all(&entry.path).unwrap();
    }

    #[test]
    fn telemetry_prints_known_and_unknown_usage_and_totals() {
        let sessions = [
            SessionUsage {
                id: "lead".into(),
                role: "lead",
                agent: "claude".into(),
                usage: Some(crate::telemetry::Usage {
                    input_tokens: 10_000,
                    output_tokens: 2_000,
                    cache_read_tokens: 0,
                    cache_write_tokens: None,
                    reasoning_tokens: None,
                    last_turn_at: None,
                    state: None,
                    estimated_cost_usd: Some(0.03),
                }),
            },
            SessionUsage {
                id: "parse".into(),
                role: "worker",
                agent: "codex".into(),
                usage: None,
            },
        ];
        assert_eq!(
            telemetry(&sessions).unwrap(),
            "lead  lead  claude  12k tokens  $0.0300\nparse  worker  codex  tokens unknown\ntotal  12k tokens  $0.0300\n\n"
        );
        assert_eq!(
            telemetry(&sessions[1..]).unwrap(),
            "parse  worker  codex  tokens unknown\ntotal  0 tokens\n\n"
        );
    }
}
