use crate::telemetry::SessionLink;
use anyhow::{Context, Result, bail};
use camino::Utf8Path;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentProfile {
    pub id: &'static str,
    pub binary: &'static str,
    pub tested_version: &'static str,
    pub foreground_args: &'static [&'static str],
    pub worker_args: &'static [&'static str],
    /// Switches a worker launch to a single non-interactive turn; appended after
    /// `worker_args` because codex's subcommand must follow its root flags.
    pub headless: &'static [&'static str],
    pub opening_turn: BriefDelivery,
    pub system_prompt: SystemPromptArg,
    tier_args: TierArgs,
    pub launch_env: &'static [(&'static str, &'static str)],
    pub composer: Option<&'static [&'static str]>,
}

/// How an agent receives its opening user turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BriefDelivery {
    /// The brief is one argument: the agent's opening turn.
    Arg,
    /// The brief behind a flag: `value` takes text, `path` takes a file.
    /// Passing a path preserves the brief verbatim and leaves the pane's TTY available.
    Flag {
        value: &'static str,
        path: &'static str,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemPromptArg {
    Flag(&'static str),
    Config {
        flag: &'static str,
        key: &'static str,
    },
    Env(&'static str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TierArgs {
    model_flag: &'static str,
    effort: EffortArg,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EffortArg {
    Flag(&'static str),
    Config {
        flag: &'static str,
        key: &'static str,
    },
}

const HERMES_QUERY: BriefDelivery = BriefDelivery::Flag {
    value: "-q",
    path: "--query-file",
};

/// The session niles can read usage back from, chosen at launch. A family not listed has none.
pub(crate) fn session_link(family: &str, source: &str, dir: &Utf8Path) -> Option<SessionLink> {
    Some(match profile_for(family)?.id {
        "claude" => SessionLink::Claude {
            session_id: Uuid::new_v4().to_string(),
        },
        "codex" => SessionLink::Codex,
        "hermes" => SessionLink::Hermes {
            source: format!("niles:{source}:{}", Uuid::new_v4()),
        },
        "pi" => SessionLink::Pi {
            session_dir: dir.join("pi"),
        },
        _ => return None,
    })
}

const PROFILES: &[AgentProfile] = &[
    AgentProfile {
        id: "codex",
        binary: "codex",
        tested_version: "0.162.1",
        // The shared daemon lets a running session continue in the background, out of its tmux
        // pane.
        foreground_args: &["--no-daemon"],
        worker_args: &["--no-daemon", "--dangerously-bypass-approvals-and-sandbox"],
        headless: &["exec", "--skip-git-repo-check"],
        opening_turn: BriefDelivery::Arg,
        system_prompt: SystemPromptArg::Config {
            flag: "--config",
            key: "developer_instructions",
        },
        tier_args: TierArgs {
            model_flag: "--model",
            effort: EffortArg::Config {
                flag: "--config",
                key: "model_reasoning_effort",
            },
        },
        launch_env: &[],
        // Codex rust-v0.164.0: bottom_pane/effort_ignition.rs::EffortTier::prompt_glyph.
        composer: Some(&["› ", "» "]),
    },
    AgentProfile {
        id: "claude",
        binary: "claude",
        tested_version: "2.1.296",
        foreground_args: &[],
        worker_args: &["--dangerously-skip-permissions"],
        headless: &["-p"],
        opening_turn: BriefDelivery::Arg,
        system_prompt: SystemPromptArg::Flag("--append-system-prompt"),
        tier_args: TierArgs {
            model_flag: "--model",
            effort: EffortArg::Flag("--effort"),
        },
        // Agent view moves a session into a detached background daemon, out of its tmux pane.
        launch_env: &[
            ("CLAUDE_CODE_ENABLE_PROMPT_SUGGESTION", "false"),
            ("CLAUDE_CODE_DISABLE_AGENT_VIEW", "1"),
        ],
        composer: Some(&["❯\u{a0}"]),
    },
    AgentProfile {
        id: "hermes",
        binary: "hermes",
        tested_version: "0.21.3",
        foreground_args: &["chat"],
        worker_args: &["chat", "--yolo"],
        headless: &["--oneshot"],
        opening_turn: HERMES_QUERY,
        system_prompt: SystemPromptArg::Env("HERMES_EPHEMERAL_SYSTEM_PROMPT"),
        tier_args: TierArgs {
            model_flag: "--model",
            effort: EffortArg::Flag("--reasoning"),
        },
        launch_env: &[],
        composer: None,
    },
    AgentProfile {
        id: "pi",
        binary: "pi",
        tested_version: "1.2.0",
        foreground_args: &[],
        worker_args: &[],
        headless: &["-p"],
        opening_turn: BriefDelivery::Arg,
        system_prompt: SystemPromptArg::Flag("--append-system-prompt"),
        tier_args: TierArgs {
            model_flag: "--model",
            effort: EffortArg::Flag("--thinking"),
        },
        launch_env: &[("PI_OFFLINE", "1")],
        composer: None,
    },
];

pub fn profiles() -> impl Iterator<Item = &'static AgentProfile> {
    PROFILES.iter()
}

pub fn known_agent_ids() -> impl Iterator<Item = &'static str> {
    profiles().map(|profile| profile.id)
}

pub fn profile_for(agent: &str) -> Option<&'static AgentProfile> {
    profiles().find(|profile| profile.id.eq_ignore_ascii_case(agent))
}

pub fn tier_args(profile: &AgentProfile, model: Option<&str>, effort: Option<&str>) -> Vec<String> {
    let mut args = Vec::new();
    if let Some(model) = model {
        args.extend([profile.tier_args.model_flag.to_owned(), model.to_owned()]);
    }
    if let Some(effort) = effort {
        match profile.tier_args.effort {
            EffortArg::Flag(flag) => args.extend([flag.to_owned(), effort.to_owned()]),
            EffortArg::Config { flag, key } => {
                args.extend([flag.to_owned(), format!("{key}=\"{effort}\"")]);
            }
        }
    }
    args
}

pub(crate) struct SwitchStep {
    pub text: String,
    pub keys: Vec<&'static str>,
    pub acknowledgement: (String, String),
}

// Claude 2.1.296: ModelPicker's search gate, Cr/KHe effort clamping and dr's
// session-only action; Hermes 0.21.7 cli_model_switch_mixin.py / cli_commands_mixin.py;
// pi 1.2.0 interactive-mode.js. None exposes a pane acknowledgement API.
pub(crate) fn switch_steps(family: &str, model: &str, effort: &str) -> Result<Vec<SwitchStep>> {
    let step = |text: String, keys, prefix: String, suffix: String| SwitchStep {
        text,
        keys,
        acknowledgement: (prefix, suffix),
    };
    let inline = |command: String, reply: String| step(command, vec!["C-m"], reply, String::new());
    Ok(match family {
        "claude" => {
            let index = ["low", "medium", "high", "xhigh", "max"]
                .iter()
                .position(|level| *level == effort)
                .context("unsupported Claude effort")?;
            // Four Lefts saturate the five-level slider at low, regardless of its initial value.
            let mut keys = vec!["Left"; 4];
            keys.extend(vec!["Right"; index]);
            keys.push("s");
            vec![
                inline("/model".into(), "Type to search".into()),
                step(
                    model.into(),
                    vec!["Down"],
                    String::new(),
                    "use this session only".into(),
                ),
                step(
                    String::new(),
                    keys,
                    format!("Set model to {model}"),
                    format!("for this session only with {effort} effort"),
                ),
            ]
        }
        "hermes" => vec![
            inline(
                format!("/model {model} --session"),
                format!("✓ Model switched: {model}"),
            ),
            inline(
                format!("/reasoning {effort}"),
                format!("✓ Reasoning effort set to '{effort}'"),
            ),
        ],
        "pi" => vec![
            inline(format!("/model {model}"), format!("Model: {model}")),
            inline(
                format!("/thinking {effort}"),
                format!("Thinking level: {effort}"),
            ),
        ],
        "codex" => vec![inline("/model".into(), "Select Model".into())],
        _ => bail!("retier supports only Claude, Codex, Hermes and pi"),
    })
}

#[cfg(test)]
mod tests {
    use super::PROFILES;

    #[test]
    fn readme_lists_tested_versions() {
        let readme = include_str!("../../README.md");
        for profile in PROFILES {
            let tested_version = format!("| {} |", profile.tested_version);
            assert!(readme.contains(&tested_version), "{}", profile.id);
        }
    }
}
