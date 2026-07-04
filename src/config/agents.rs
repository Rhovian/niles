use std::collections::BTreeMap;

use anyhow::{Result, bail};

use crate::config::spec::{AgentConfig, PromptMode};

#[derive(Debug, Clone, Copy)]
pub struct AgentProfile {
    pub id: &'static str,
    pub binary: &'static str,
    pub args: &'static [&'static str],
    pub min_version: &'static str,
    pub tested_version: &'static str,
    pub prompt: PromptMode,
}

#[derive(Debug, Clone, Copy)]
pub enum InvocationDefaults {
    Default,
    Foreground,
    Worker,
}

#[derive(Debug, Clone)]
pub struct AgentInvocation {
    pub binary: String,
    pub args: Vec<String>,
    pub prompt: PromptMode,
    pub spec: AgentSpec,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSpec {
    original: String,
    family: String,
    model: Option<String>,
    effort: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentTier {
    pub family: String,
    pub model: Option<String>,
    pub effort: Option<String>,
}

const PROFILES: &[AgentProfile] = &[
    AgentProfile {
        id: "codex",
        binary: "codex",
        args: &["exec", "--sandbox", "workspace-write"],
        min_version: "0.142.4",
        tested_version: "0.142.4",
        prompt: PromptMode::Arg,
    },
    AgentProfile {
        id: "claude",
        binary: "claude",
        args: &["-p"],
        min_version: "2.1.197",
        tested_version: "2.1.197",
        prompt: PromptMode::Arg,
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

pub fn parse_spec(agent: &str) -> Result<AgentSpec> {
    AgentSpec::parse(agent)
}

pub fn config_for<'a>(
    configs: &'a BTreeMap<String, AgentConfig>,
    agent: &str,
) -> Result<Option<&'a AgentConfig>> {
    let spec = AgentSpec::parse(agent)?;
    Ok(configs.get(agent).or_else(|| configs.get(spec.family())))
}

#[allow(dead_code)]
pub fn default_config(agent: &str) -> AgentConfig {
    let family = family_or_self(agent);
    match profile_for(&family) {
        Some(profile) => AgentConfig {
            binary: Some(profile.binary.to_owned()),
            args: profile_args(profile),
            prompt: profile.prompt,
        },
        None => AgentConfig {
            binary: Some(agent.to_owned()),
            args: Vec::new(),
            prompt: PromptMode::Arg,
        },
    }
}

pub fn default_binary(agent: &str) -> String {
    let family = family_or_self(agent);
    profile_for(&family)
        .map(|profile| profile.binary.to_owned())
        .unwrap_or(family)
}

pub fn default_args(agent: &str) -> Vec<String> {
    let family = family_or_self(agent);
    profile_for(&family).map(profile_args).unwrap_or_default()
}

pub fn default_prompt(agent: &str) -> PromptMode {
    let family = family_or_self(agent);
    profile_for(&family)
        .map(|profile| profile.prompt)
        .unwrap_or(PromptMode::Arg)
}

pub fn invocation(
    agent: &str,
    config: Option<&AgentConfig>,
    defaults: InvocationDefaults,
) -> Result<AgentInvocation> {
    let spec = AgentSpec::parse(agent)?;
    let default_invocation = default_invocation(&spec, defaults);

    let mut invocation = match config {
        Some(config) => AgentInvocation {
            binary: config.binary.clone().unwrap_or(default_invocation.binary),
            args: if config.args.is_empty() {
                default_invocation.args
            } else {
                config.args.clone()
            },
            prompt: config.prompt,
            spec,
        },
        None => default_invocation,
    };
    invocation.args.extend(mapped_tier_args(&invocation.spec)?);
    Ok(invocation)
}

pub fn foreground_invocation(agent: &str, config: Option<&AgentConfig>) -> Result<AgentInvocation> {
    invocation(agent, config, InvocationDefaults::Foreground)
}

fn default_invocation(spec: &AgentSpec, defaults: InvocationDefaults) -> AgentInvocation {
    match defaults {
        InvocationDefaults::Default => AgentInvocation {
            binary: default_binary(spec.family()),
            args: default_args(spec.family()),
            prompt: default_prompt(spec.family()),
            spec: spec.clone(),
        },
        InvocationDefaults::Foreground => AgentInvocation {
            binary: default_binary(spec.family()),
            args: Vec::new(),
            prompt: worker_prompt(spec.family()),
            spec: spec.clone(),
        },
        InvocationDefaults::Worker => AgentInvocation {
            binary: default_binary(spec.family()),
            args: worker_args(spec.family()),
            prompt: worker_prompt(spec.family()),
            spec: spec.clone(),
        },
    }
}

fn worker_args(agent: &str) -> Vec<String> {
    // Workers run autonomously in their own window. Codex runs fully
    // unrestricted (no sandbox, no approvals), matching Claude's
    // --dangerously-skip-permissions.
    match agent {
        "codex" => vec!["--dangerously-bypass-approvals-and-sandbox".to_owned()],
        "claude" => vec!["--dangerously-skip-permissions".to_owned()],
        _ => default_args(agent),
    }
}

fn worker_prompt(agent: &str) -> PromptMode {
    match agent {
        "codex" | "claude" => PromptMode::Arg,
        _ => default_prompt(agent),
    }
}

fn profile_args(profile: AgentProfile) -> Vec<String> {
    profile.args.iter().map(|arg| (*arg).to_owned()).collect()
}

fn family_or_self(agent: &str) -> String {
    AgentSpec::parse(agent)
        .map(|spec| spec.family().to_owned())
        .unwrap_or_else(|_| agent.to_owned())
}

fn mapped_tier_args(spec: &AgentSpec) -> Result<Vec<String>> {
    match spec.family() {
        "codex" => codex_tier_args(spec),
        "claude" => claude_tier_args(spec),
        _ if spec.model().is_some() || spec.effort().is_some() => bail!(
            "agent `{}` does not support model/effort qualifiers",
            spec.family()
        ),
        _ => Ok(Vec::new()),
    }
}

fn codex_tier_args(spec: &AgentSpec) -> Result<Vec<String>> {
    let mut args = Vec::new();
    if let Some(model) = spec.model() {
        args.extend(["--model".to_owned(), model.to_owned()]);
    }
    if let Some(effort) = spec.effort() {
        args.extend([
            "--config".to_owned(),
            format!("model_reasoning_effort=\"{effort}\""),
        ]);
    }
    Ok(args)
}

fn claude_tier_args(spec: &AgentSpec) -> Result<Vec<String>> {
    let mut args = Vec::new();
    if let Some(model) = spec.model() {
        args.extend(["--model".to_owned(), model.to_owned()]);
    }
    if let Some(effort) = spec.effort() {
        args.extend(["--effort".to_owned(), effort.to_owned()]);
    }
    Ok(args)
}

impl AgentSpec {
    pub fn parse(agent: &str) -> Result<Self> {
        let parts = agent.split(':').collect::<Vec<_>>();
        if parts.is_empty() || parts.len() > 3 {
            bail!("invalid agent spec `{agent}`; expected family[:model[:effort]]");
        }
        if parts.iter().any(|part| part.trim().is_empty()) {
            bail!("invalid agent spec `{agent}`; family, model, and effort cannot be empty");
        }

        let family = canonical_family(parts[0]).unwrap_or_else(|| parts[0].to_owned());
        let model = parts
            .get(1)
            .map(|value| normalize_model(&family, value))
            .transpose()?;
        let effort = parts
            .get(2)
            .map(|value| normalize_effort(&family, value))
            .transpose()?;

        if effort.is_some() && model.is_none() {
            bail!("invalid agent spec `{agent}`; effort requires a model");
        }
        if model.is_some() && profile_for(&family).is_none() {
            bail!(
                "unknown agent family `{}`; model/effort qualifiers are only supported for codex and claude",
                parts[0]
            );
        }

        Ok(Self {
            original: agent.to_owned(),
            family,
            model,
            effort,
        })
    }

    pub fn original(&self) -> &str {
        &self.original
    }

    pub fn family(&self) -> &str {
        &self.family
    }

    pub fn model(&self) -> Option<&str> {
        self.model.as_deref()
    }

    pub fn effort(&self) -> Option<&str> {
        self.effort.as_deref()
    }

    pub fn tier(&self) -> Option<AgentTier> {
        if self.model.is_none() && self.effort.is_none() {
            return None;
        }

        Some(AgentTier {
            family: self.family.clone(),
            model: self.model.clone(),
            effort: self.effort.clone(),
        })
    }

    pub fn canonical(&self) -> String {
        let mut spec = self.family.clone();
        if let Some(model) = &self.model {
            spec.push(':');
            spec.push_str(model);
        }
        if let Some(effort) = &self.effort {
            spec.push(':');
            spec.push_str(effort);
        }
        spec
    }
}

fn canonical_family(family: &str) -> Option<String> {
    profile_for(family).map(|profile| profile.id.to_owned())
}

fn normalize_model(family: &str, model: &str) -> Result<String> {
    let normalized = model.to_ascii_lowercase();
    match family {
        "codex" | "claude" if is_model_token(&normalized) => Ok(normalized),
        "codex" | "claude" => bail!(
            "invalid {family} model `{model}` in agent spec; expected letters, digits, '.', '_', or '-'"
        ),
        _ => Ok(model.to_owned()),
    }
}

fn normalize_effort(family: &str, effort: &str) -> Result<String> {
    let normalized = match effort.to_ascii_lowercase().as_str() {
        "med" => "medium".to_owned(),
        value => value.to_owned(),
    };

    match family {
        "codex" if supported_efforts("codex").contains(&normalized.as_str()) => Ok(normalized),
        "claude" if supported_efforts("claude").contains(&normalized.as_str()) => Ok(normalized),
        "codex" | "claude" => {
            bail!("unsupported {family} effort `{effort}` in agent spec")
        }
        _ => Ok(effort.to_owned()),
    }
}

pub(crate) fn validate_static_model(spec: &AgentSpec) -> Result<()> {
    let Some(model) = spec.model() else {
        return Ok(());
    };

    match spec.family() {
        "codex" if supported_codex_model(model) => Ok(()),
        "claude" if supported_claude_model(model) => Ok(()),
        "codex" | "claude" => bail!(
            "unsupported {} model `{model}` in agent spec",
            spec.family()
        ),
        _ => Ok(()),
    }
}

pub(crate) fn default_model_aliases(family: &str) -> &'static [&'static str] {
    match family {
        "codex" => &["o3", "o3-pro", "o4-mini"],
        "claude" => &["opus", "sonnet", "fable", "haiku"],
        _ => &[],
    }
}

pub(crate) fn supported_efforts(family: &str) -> &'static [&'static str] {
    match family {
        "codex" => &["minimal", "low", "medium", "high", "xhigh"],
        "claude" => &["low", "medium", "high", "xhigh", "max"],
        _ => &[],
    }
}

fn is_model_token(model: &str) -> bool {
    model
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
}

fn supported_codex_model(model: &str) -> bool {
    model.starts_with("gpt-") || matches!(model, "o3" | "o3-pro" | "o4-mini")
}

fn supported_claude_model(model: &str) -> bool {
    matches!(model, "opus" | "sonnet" | "fable" | "haiku") || model.starts_with("claude-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_defaults_to_workspace_write() {
        let config = default_config("codex");

        assert_eq!(config.binary.as_deref(), Some("codex"));
        assert_eq!(
            config.args,
            ["exec", "--sandbox", "workspace-write"].map(str::to_owned)
        );
        assert!(matches!(config.prompt, PromptMode::Arg));
    }

    #[test]
    fn unknown_agents_default_to_binary_name() {
        let config = default_config("custom");

        assert_eq!(config.binary.as_deref(), Some("custom"));
        assert!(config.args.is_empty());
        assert!(matches!(config.prompt, PromptMode::Arg));
    }

    #[test]
    fn invocation_applies_default_agent_args() {
        let invocation = invocation("codex", None, InvocationDefaults::Default).unwrap();

        assert_eq!(invocation.binary, "codex");
        assert_eq!(
            invocation.args,
            ["exec", "--sandbox", "workspace-write"].map(str::to_owned)
        );
        assert!(matches!(invocation.prompt, PromptMode::Arg));
    }

    #[test]
    fn invocation_applies_worker_defaults() {
        let invocation = invocation("codex", None, InvocationDefaults::Worker).unwrap();

        assert_eq!(invocation.binary, "codex");
        assert_eq!(
            invocation.args,
            ["--dangerously-bypass-approvals-and-sandbox"].map(str::to_owned)
        );
        assert!(matches!(invocation.prompt, PromptMode::Arg));
    }

    #[test]
    fn foreground_invocation_preserves_builtin_manager_defaults() {
        let invocation = foreground_invocation("claude:opus:max", None).unwrap();

        assert_eq!(invocation.binary, "claude");
        assert_eq!(
            invocation.args,
            ["--model", "opus", "--effort", "max"].map(str::to_owned)
        );
        assert!(matches!(invocation.prompt, PromptMode::Arg));
    }

    #[test]
    fn foreground_invocation_uses_configured_custom_manager_binary_and_args() {
        let config = AgentConfig {
            binary: Some("/tmp/custom-manager".to_owned()),
            args: ["--mode", "manager"].map(str::to_owned).to_vec(),
            prompt: PromptMode::Arg,
        };

        let invocation = foreground_invocation("gemini", Some(&config)).unwrap();

        assert_eq!(invocation.binary, "/tmp/custom-manager");
        assert_eq!(invocation.args, ["--mode", "manager"].map(str::to_owned));
        assert_eq!(invocation.spec.family(), "gemini");
        assert!(matches!(invocation.prompt, PromptMode::Arg));
    }

    #[test]
    fn parses_agent_model_effort_specs() {
        let codex = AgentSpec::parse("codex:gpt-5.5:xhigh").unwrap();
        assert_eq!(codex.original(), "codex:gpt-5.5:xhigh");
        assert_eq!(codex.family(), "codex");
        assert_eq!(codex.model(), Some("gpt-5.5"));
        assert_eq!(codex.effort(), Some("xhigh"));

        let claude = AgentSpec::parse("claude:sonnet:med").unwrap();
        assert_eq!(claude.family(), "claude");
        assert_eq!(claude.model(), Some("sonnet"));
        assert_eq!(claude.effort(), Some("medium"));

        let haiku = AgentSpec::parse("Claude:Haiku:MAX").unwrap();
        assert_eq!(haiku.family(), "claude");
        assert_eq!(haiku.model(), Some("haiku"));
        assert_eq!(haiku.effort(), Some("max"));

        let full_claude = AgentSpec::parse("claude:claude-haiku-4-5-20251001:low").unwrap();
        assert_eq!(full_claude.model(), Some("claude-haiku-4-5-20251001"));

        let codex_full = AgentSpec::parse("CODEX:gpt-5.5-codex:xhigh").unwrap();
        assert_eq!(codex_full.family(), "codex");
        assert_eq!(codex_full.model(), Some("gpt-5.5-codex"));

        let future = AgentSpec::parse("codex:omega:xhigh").unwrap();
        assert_eq!(future.model(), Some("omega"));

        let bare = AgentSpec::parse("custom").unwrap();
        assert_eq!(bare.family(), "custom");
        assert_eq!(bare.model(), None);
        assert_eq!(bare.effort(), None);
    }

    #[test]
    fn rejects_invalid_agent_specs() {
        assert!(AgentSpec::parse("codex:gpt-5.5:xhigh:extra").is_err());
        assert!(AgentSpec::parse("codex::xhigh").is_err());
        assert!(AgentSpec::parse("custom:model:high").is_err());
        assert!(AgentSpec::parse("claude:opus:turbo").is_err());
        assert!(AgentSpec::parse("codex:gpt-5.5:max").is_err());
        assert!(AgentSpec::parse("codex:not a model:high").is_err());
    }

    #[test]
    fn static_validation_rejects_unknown_builtin_models() {
        let spec = AgentSpec::parse("codex:omega:high").unwrap();
        let err = validate_static_model(&spec).unwrap_err().to_string();

        assert!(err.contains("unsupported codex model `omega`"));
    }

    #[test]
    fn invocation_maps_codex_model_effort_flags() {
        let invocation =
            invocation("codex:gpt-5.5:xhigh", None, InvocationDefaults::Default).unwrap();

        assert_eq!(invocation.binary, "codex");
        assert_eq!(
            invocation.args,
            [
                "exec",
                "--sandbox",
                "workspace-write",
                "--model",
                "gpt-5.5",
                "--config",
                "model_reasoning_effort=\"xhigh\""
            ]
            .map(str::to_owned)
        );
    }

    #[test]
    fn invocation_maps_claude_model_effort_flags() {
        let invocation = invocation("claude:opus:max", None, InvocationDefaults::Worker).unwrap();

        assert_eq!(invocation.binary, "claude");
        assert_eq!(
            invocation.args,
            [
                "--dangerously-skip-permissions",
                "--model",
                "opus",
                "--effort",
                "max"
            ]
            .map(str::to_owned)
        );
    }
}
