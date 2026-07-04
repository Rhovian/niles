use anyhow::{Result, bail};

use crate::config::spec::PromptMode;

#[derive(Debug, Clone, Copy)]
pub struct AgentProfile {
    pub id: &'static str,
    pub binary: &'static str,
    pub args: &'static [&'static str],
    pub min_version: &'static str,
    pub tested_version: &'static str,
    pub prompt: PromptMode,
    pub worker_args: &'static [&'static str],
    pub worker_prompt: PromptMode,
    pub model_aliases: &'static [&'static str],
    pub supported_efforts: &'static [&'static str],
    pub model_prefixes: &'static [&'static str],
    tier_args: TierArgs,
    model_group_label: ModelGroupLabel,
    manager_prompt: ManagerPrompt,
    pub launch_env: &'static [(&'static str, &'static str)],
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

#[derive(Debug, Clone, Copy)]
enum ModelGroupLabel {
    Alias,
    HyphenatedAlias,
}

#[derive(Debug, Clone, Copy)]
enum ManagerPrompt {
    CombinedArg,
    AppendSystemPrompt,
}

const PROFILES: &[AgentProfile] = &[
    AgentProfile {
        id: "codex",
        binary: "codex",
        args: &["exec", "--sandbox", "workspace-write"],
        min_version: "0.142.4",
        tested_version: "0.142.4",
        prompt: PromptMode::Arg,
        worker_args: &["--dangerously-bypass-approvals-and-sandbox"],
        worker_prompt: PromptMode::Arg,
        model_aliases: &["gpt-5.5", "o3", "o3-pro", "o4-mini"],
        supported_efforts: &["minimal", "low", "medium", "high", "xhigh"],
        model_prefixes: &["gpt-"],
        tier_args: TierArgs {
            model_flag: "--model",
            effort: EffortArg::Config {
                flag: "--config",
                key: "model_reasoning_effort",
            },
        },
        model_group_label: ModelGroupLabel::Alias,
        manager_prompt: ManagerPrompt::CombinedArg,
        launch_env: &[],
    },
    AgentProfile {
        id: "claude",
        binary: "claude",
        args: &["-p"],
        min_version: "2.1.197",
        tested_version: "2.1.197",
        prompt: PromptMode::Arg,
        worker_args: &["--dangerously-skip-permissions"],
        worker_prompt: PromptMode::Arg,
        model_aliases: &["opus", "sonnet", "fable", "haiku"],
        supported_efforts: &["low", "medium", "high", "xhigh", "max"],
        model_prefixes: &["claude-"],
        tier_args: TierArgs {
            model_flag: "--model",
            effort: EffortArg::Flag("--effort"),
        },
        model_group_label: ModelGroupLabel::HyphenatedAlias,
        manager_prompt: ManagerPrompt::AppendSystemPrompt,
        launch_env: &[("CLAUDE_CODE_ENABLE_PROMPT_SUGGESTION", "false")],
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
        "invalid {} model `{model}` in agent spec; expected letters, digits, '.', '_', or '-'",
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
    profile.model_aliases.contains(&model)
        || profile
            .model_prefixes
            .iter()
            .any(|prefix| model.starts_with(prefix))
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

pub fn manager_prompt_args(
    profile: Option<AgentProfile>,
    brief: String,
    startup_prompt: String,
) -> Vec<String> {
    match profile.map(|profile| profile.manager_prompt) {
        Some(ManagerPrompt::AppendSystemPrompt) => {
            vec!["--append-system-prompt".to_owned(), brief, startup_prompt]
        }
        Some(ManagerPrompt::CombinedArg) | None => vec![format!("{brief}\n\n{startup_prompt}")],
    }
}

fn is_model_token(model: &str) -> bool {
    model
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
}
