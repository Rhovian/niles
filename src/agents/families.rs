use anyhow::{Result, bail};

use crate::config::spec::PromptMode;

#[derive(Debug, Clone, Copy)]
pub struct AgentProfile {
    pub id: &'static str,
    pub binary: &'static str,
    pub foreground_args: &'static [&'static str],
    pub worker_args: &'static [&'static str],
    pub worker_brief: BriefDelivery,
    pub lead_brief: BriefDelivery,
    models: &'static [ModelEntry],
    tier_args: TierArgs,
    pub launch_env: &'static [(&'static str, &'static str)],
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

/// One model Niles carries, with the efforts that model accepts.
///
/// The roster is the whole of what a family will launch: a model it does not name is rejected,
/// never guessed at. Effort belongs to the model too — `gpt-6-astra` takes `ultra` where `gpt-5.5`
/// stops at `xhigh` and `haiku` takes none at all — so a family-wide list would either reject a
/// spec the CLI honours or wave through one it rejects. Each ladder is what the installed CLI
/// reports for that model; a model the CLI grows is a line here, not a rule that infers it.
#[derive(Debug, Clone, Copy)]
struct ModelEntry {
    name: &'static str,
    efforts: &'static [&'static str],
}

/// A model that takes no effort at all: the flag is not a level it can be given, so no spelling of
/// one is right and the picker skips the question.
const NO_EFFORTS: &[&str] = &[];
/// codex spells a per-model ladder in its model table; these are the three it uses today.
const CODEX_TO_XHIGH: &[&str] = &["low", "medium", "high", "xhigh"];
const CODEX_TO_MAX: &[&str] = &["low", "medium", "high", "xhigh", "max"];
const CODEX_TO_ULTRA: &[&str] = &["low", "medium", "high", "xhigh", "max", "ultra"];
/// claude gates each level on a model capability (`effort`, `xhigh_effort`, `max_effort`): the
/// three flagships carry all three, and haiku carries none of them, so it takes no effort at all.
const CLAUDE_TO_MAX: &[&str] = &["low", "medium", "high", "xhigh", "max"];
/// hermes routes to whatever its provider serves, so its `--reasoning` vocabulary is one list for
/// every model — the only family here where one ladder is the honest answer.
const HERMES_EFFORTS: &[&str] = &[
    "none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra",
];

const HERMES_QUERY: BriefDelivery = BriefDelivery::Flag {
    value: "-q",
    path: "--query-file",
};

const PROFILES: &[AgentProfile] = &[
    AgentProfile {
        id: "codex",
        binary: "codex",
        foreground_args: &[],
        worker_args: &["--dangerously-bypass-approvals-and-sandbox"],
        worker_brief: BriefDelivery::Arg,
        lead_brief: BriefDelivery::Arg,
        models: &[
            ModelEntry {
                name: "gpt-5.5",
                efforts: CODEX_TO_XHIGH,
            },
            ModelEntry {
                name: "gpt-6-astra",
                efforts: CODEX_TO_ULTRA,
            },
            ModelEntry {
                name: "gpt-5.6-sol",
                efforts: CODEX_TO_ULTRA,
            },
            ModelEntry {
                name: "gpt-5.6-terra",
                efforts: CODEX_TO_ULTRA,
            },
            ModelEntry {
                name: "gpt-5.6-luna",
                efforts: CODEX_TO_MAX,
            },
        ],
        tier_args: TierArgs {
            model_flag: "--model",
            effort: EffortArg::Config {
                flag: "--config",
                key: "model_reasoning_effort",
            },
        },
        launch_env: &[],
    },
    AgentProfile {
        id: "claude",
        binary: "claude",
        foreground_args: &[],
        worker_args: &["--dangerously-skip-permissions"],
        worker_brief: BriefDelivery::Arg,
        lead_brief: BriefDelivery::SystemPrompt("--append-system-prompt"),
        models: &[
            ModelEntry {
                name: "opus",
                efforts: CLAUDE_TO_MAX,
            },
            ModelEntry {
                name: "sonnet",
                efforts: CLAUDE_TO_MAX,
            },
            ModelEntry {
                name: "fable",
                efforts: CLAUDE_TO_MAX,
            },
            // The alias resolves to claude-haiku-4-5, which the CLI's model table gives no effort
            // capability at all: an `--effort` on haiku is a flag the model cannot answer.
            ModelEntry {
                name: "haiku",
                efforts: NO_EFFORTS,
            },
        ],
        tier_args: TierArgs {
            model_flag: "--model",
            effort: EffortArg::Flag("--effort"),
        },
        launch_env: &[("CLAUDE_CODE_ENABLE_PROMPT_SUGGESTION", "false")],
    },
    // hermes puts the session behind a `chat` subcommand, and takes its opening turn behind a flag
    // rather than as a positional argument. A worker's brief is a file already, so it goes over as
    // `--query-file`: the brief stays verbatim, nothing in it is shell-interpreted, and unlike
    // redirecting it on stdin the pane stays a real TTY, which is the difference between seeding
    // an interactive session and answering once and exiting. The lead's turn is its brief plus the
    // startup line, which no one file holds, so that one goes by value as `-q`.
    AgentProfile {
        id: "hermes",
        binary: "hermes",
        foreground_args: &["chat"],
        worker_args: &["chat", "--yolo"],
        worker_brief: HERMES_QUERY,
        lead_brief: HERMES_QUERY,
        models: &[ModelEntry {
            name: "tencent/hy3",
            efforts: HERMES_EFFORTS,
        }],
        tier_args: TierArgs {
            model_flag: "--model",
            effort: EffortArg::Flag("--reasoning"),
        },
        launch_env: &[],
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

pub fn normalize_model(profile: AgentProfile, model: &str) -> Result<String> {
    let normalized = model.to_ascii_lowercase();
    if is_model_token(&normalized) {
        return Ok(normalized);
    }

    bail!(
        "invalid {} model `{model}` in agent spec; expected letters, digits, '.', '_', '-', or '/'",
        profile.id
    )
}

pub fn normalize_effort(
    profile: AgentProfile,
    model: Option<&str>,
    effort: &str,
) -> Result<String> {
    let normalized = match effort.to_ascii_lowercase().as_str() {
        "med" => "medium".to_owned(),
        value => value.to_owned(),
    };

    if supported_efforts(profile, model).contains(&normalized.as_str()) {
        return Ok(normalized);
    }

    // A model off the roster is the error, not its effort, and `validate_static_model` is where
    // that is said. Answering here would name the wrong half of the spec.
    match model_entry(profile, model) {
        Some(entry) if entry.efforts.is_empty() => bail!(
            "{} model `{}` takes no effort; drop the effort from the agent spec",
            profile.id,
            entry.name
        ),
        Some(entry) => bail!(
            "unsupported {} effort `{effort}` for model `{}` in agent spec",
            profile.id,
            entry.name
        ),
        None => Ok(normalized),
    }
}

pub fn model_names(profile: AgentProfile) -> Vec<&'static str> {
    profile.models.iter().map(|entry| entry.name).collect()
}

/// The efforts a model takes. Empty for a model that takes none, and for a model off the roster —
/// which `supports_model` rejects, so effort never has to answer for one.
pub fn supported_efforts(profile: AgentProfile, model: Option<&str>) -> &'static [&'static str] {
    match model_entry(profile, model) {
        Some(entry) => entry.efforts,
        None => NO_EFFORTS,
    }
}

fn model_entry(profile: AgentProfile, model: Option<&str>) -> Option<ModelEntry> {
    let model = model?;
    profile
        .models
        .iter()
        .copied()
        .find(|entry| entry.name == model)
}

pub fn supports_model(profile: AgentProfile, model: &str) -> bool {
    model_entry(profile, Some(model)).is_some()
}

fn is_model_token(model: &str) -> bool {
    model
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-' | '/'))
}
