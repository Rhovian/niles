use crate::{config::spec::PromptMode, telemetry::SessionLink};
use uuid::Uuid;

#[derive(Debug, Clone, Copy)]
pub struct AgentProfile {
    pub id: &'static str,
    pub binary: &'static str,
    pub tested_version: &'static str,
    pub foreground_args: &'static [&'static str],
    pub worker_args: &'static [&'static str],
    pub worker_brief: BriefDelivery,
    pub lead_brief: BriefDelivery,
    pub default_model: &'static str,
    tier_args: TierArgs,
    pub launch_env: &'static [(&'static str, &'static str)],
    pub composer: Option<&'static str>,
}

/// How an agent receives its brief.
///
/// One dial, answered per role on every profile, so both launch paths — the worker's generated
/// script and the lead's argv — read the same decision and a family spells its flags here once.
/// A new spelling is a new variant, which both launchers are then forced to answer.
#[derive(Debug, Clone, Copy)]
pub enum BriefDelivery {
    /// The brief is one argument: the agent's opening turn.
    Arg,
    /// The brief arrives on stdin.
    Stdin,
    /// The brief behind a flag — `value` where the launcher has the text, `path` where it can
    /// point at the file. Handing over the path keeps the brief verbatim (nothing in it is
    /// shell-interpreted) and, unlike a stdin redirect, leaves the pane a real TTY.
    Flag {
        value: &'static str,
        path: &'static str,
    },
    /// The brief as standing context behind a flag, with the opening turn following it as an
    /// argument.
    SystemPrompt(&'static str),
}

impl From<PromptMode> for BriefDelivery {
    fn from(prompt: PromptMode) -> Self {
        match prompt {
            PromptMode::Arg => Self::Arg,
            PromptMode::Stdin => Self::Stdin,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct TierArgs {
    model_flag: &'static str,
    effort: EffortArg,
}

#[derive(Debug, Clone, Copy)]
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
pub(crate) fn session_link(family: &str, source: &str) -> Option<SessionLink> {
    Some(match profile_for(family)?.id {
        "claude" => SessionLink::Claude {
            session_id: Uuid::new_v4().to_string(),
        },
        "codex" => SessionLink::Codex,
        "hermes" => SessionLink::Hermes {
            source: format!("niles:{source}:{}", Uuid::new_v4()),
        },
        _ => return None,
    })
}

const PROFILES: &[AgentProfile] = &[
    AgentProfile {
        id: "codex",
        binary: "codex",
        tested_version: "0.160.0",
        foreground_args: &[],
        worker_args: &["--dangerously-bypass-approvals-and-sandbox"],
        worker_brief: BriefDelivery::Arg,
        lead_brief: BriefDelivery::Arg,
        default_model: "gpt-5.5",
        tier_args: TierArgs {
            model_flag: "--model",
            effort: EffortArg::Config {
                flag: "--config",
                key: "model_reasoning_effort",
            },
        },
        launch_env: &[],
        composer: Some("› "),
    },
    AgentProfile {
        id: "claude",
        binary: "claude",
        tested_version: "2.1.288",
        foreground_args: &[],
        worker_args: &["--dangerously-skip-permissions"],
        worker_brief: BriefDelivery::Arg,
        lead_brief: BriefDelivery::SystemPrompt("--append-system-prompt"),
        default_model: "opus",
        tier_args: TierArgs {
            model_flag: "--model",
            effort: EffortArg::Flag("--effort"),
        },
        launch_env: &[("CLAUDE_CODE_ENABLE_PROMPT_SUGGESTION", "false")],
        composer: Some("❯\u{a0}"),
    },
    AgentProfile {
        id: "hermes",
        binary: "hermes",
        tested_version: "0.21.3",
        foreground_args: &["chat"],
        worker_args: &["chat", "--yolo"],
        worker_brief: HERMES_QUERY,
        // The lead's turn has no single file, so it goes by value.
        lead_brief: HERMES_QUERY,
        default_model: "tencent/hy3",
        tier_args: TierArgs {
            model_flag: "--model",
            effort: EffortArg::Flag("--reasoning"),
        },
        launch_env: &[],
        composer: None,
    },
];

pub fn known_agent_ids() -> impl Iterator<Item = &'static str> {
    PROFILES.iter().map(|profile| profile.id)
}

pub fn profile_for(agent: &str) -> Option<AgentProfile> {
    PROFILES
        .iter()
        .find(|profile| profile.id.eq_ignore_ascii_case(agent))
        .copied()
}

pub fn tier_args(profile: AgentProfile, model: Option<&str>, effort: Option<&str>) -> Vec<String> {
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
