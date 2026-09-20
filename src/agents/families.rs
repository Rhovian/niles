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
    pub model_aliases: &'static [&'static str],
    pub supported_efforts: &'static [&'static str],
    pub model_prefixes: &'static [&'static str],
    model_shape: ModelShape,
    tier_args: TierArgs,
    model_group_label: ModelGroupLabel,
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

/// How a family spells the models it accepts.
#[derive(Debug, Clone, Copy)]
enum ModelShape {
    /// Bare names, where a known prefix marks family membership: `gpt-5.5`, `claude-opus-5`.
    Prefixed,
    /// `vendor/name`, where the vendor set is open — hermes routes to whatever its provider
    /// serves, so the shape is the only rule we can check without asking the provider.
    VendorPath,
}

#[derive(Debug, Clone, Copy)]
enum ModelGroupLabel {
    Alias,
    HyphenatedAlias,
}

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
        model_aliases: &["gpt-5.5", "gpt-5.6-sol", "gpt-5.6-terra", "gpt-5.6-luna"],
        supported_efforts: &["minimal", "low", "medium", "high", "xhigh", "max"],
        model_prefixes: &["gpt-"],
        model_shape: ModelShape::Prefixed,
        tier_args: TierArgs {
            model_flag: "--model",
            effort: EffortArg::Config {
                flag: "--config",
                key: "model_reasoning_effort",
            },
        },
        model_group_label: ModelGroupLabel::Alias,
        launch_env: &[],
    },
    AgentProfile {
        id: "claude",
        binary: "claude",
        foreground_args: &[],
        worker_args: &["--dangerously-skip-permissions"],
        worker_brief: BriefDelivery::Arg,
        lead_brief: BriefDelivery::SystemPrompt("--append-system-prompt"),
        model_aliases: &["opus", "sonnet", "fable", "haiku"],
        supported_efforts: &["low", "medium", "high", "xhigh", "max"],
        model_prefixes: &["claude-"],
        model_shape: ModelShape::Prefixed,
        tier_args: TierArgs {
            model_flag: "--model",
            effort: EffortArg::Flag("--effort"),
        },
        model_group_label: ModelGroupLabel::HyphenatedAlias,
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
        model_aliases: &["tencent/hy3"],
        supported_efforts: &[
            "none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra",
        ],
        model_prefixes: &[],
        model_shape: ModelShape::VendorPath,
        tier_args: TierArgs {
            model_flag: "--model",
            effort: EffortArg::Flag("--reasoning"),
        },
        model_group_label: ModelGroupLabel::Alias,
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

pub fn normalize_effort(profile: AgentProfile, effort: &str) -> Result<String> {
    let normalized = match effort.to_ascii_lowercase().as_str() {
        "med" => "medium".to_owned(),
        value => value.to_owned(),
    };

    if profile.supported_efforts.contains(&normalized.as_str()) {
        return Ok(normalized);
    }

    bail!("unsupported {} effort `{effort}` in agent spec", profile.id)
}

pub fn supports_model(profile: AgentProfile, model: &str) -> bool {
    if profile.model_aliases.contains(&model) {
        return true;
    }

    match profile.model_shape {
        ModelShape::Prefixed => profile
            .model_prefixes
            .iter()
            .any(|prefix| model.starts_with(prefix)),
        ModelShape::VendorPath => is_vendor_path(model),
    }
}

/// `vendor/name`, both halves present and no second slash: `anthropic/claude-opus-5`, `tencent/hy3`.
fn is_vendor_path(model: &str) -> bool {
    match model.split_once('/') {
        Some((vendor, name)) => !vendor.is_empty() && !name.is_empty() && !name.contains('/'),
        None => false,
    }
}

pub fn model_group_label(profile: AgentProfile, model: &str) -> String {
    if profile.model_aliases.contains(&model) {
        return model.to_owned();
    }

    if matches!(profile.model_group_label, ModelGroupLabel::HyphenatedAlias)
        && profile
            .model_prefixes
            .iter()
            .any(|prefix| model.starts_with(prefix))
        && let Some(alias) = profile
            .model_aliases
            .iter()
            .find(|alias| model.split('-').any(|part| part == **alias))
    {
        return (*alias).to_owned();
    }

    model.to_owned()
}

fn is_model_token(model: &str) -> bool {
    model
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-' | '/'))
}
