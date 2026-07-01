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
    Worker,
}

#[derive(Debug, Clone)]
pub struct AgentInvocation {
    pub binary: String,
    pub args: Vec<String>,
    pub prompt: PromptMode,
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
    PROFILES.iter().find(|profile| profile.id == agent).copied()
}

#[allow(dead_code)]
pub fn default_config(agent: &str) -> AgentConfig {
    match profile_for(agent) {
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
    profile_for(agent)
        .map(|profile| profile.binary.to_owned())
        .unwrap_or_else(|| agent.to_owned())
}

pub fn default_args(agent: &str) -> Vec<String> {
    profile_for(agent).map(profile_args).unwrap_or_default()
}

pub fn default_prompt(agent: &str) -> PromptMode {
    profile_for(agent)
        .map(|profile| profile.prompt)
        .unwrap_or(PromptMode::Arg)
}

pub fn invocation(
    agent: &str,
    config: Option<&AgentConfig>,
    defaults: InvocationDefaults,
) -> AgentInvocation {
    let default_invocation = default_invocation(agent, defaults);

    match config {
        Some(config) => AgentInvocation {
            binary: config.binary.clone().unwrap_or(default_invocation.binary),
            args: if config.args.is_empty() {
                default_invocation.args
            } else {
                config.args.clone()
            },
            prompt: config.prompt,
        },
        None => default_invocation,
    }
}

pub fn foreground_binary(agent: &str) -> String {
    default_binary(agent)
}

pub fn foreground_args(_agent: &str) -> Vec<String> {
    Vec::new()
}

fn default_invocation(agent: &str, defaults: InvocationDefaults) -> AgentInvocation {
    match defaults {
        InvocationDefaults::Default => AgentInvocation {
            binary: default_binary(agent),
            args: default_args(agent),
            prompt: default_prompt(agent),
        },
        InvocationDefaults::Worker => AgentInvocation {
            binary: default_binary(agent),
            args: worker_args(agent),
            prompt: worker_prompt(agent),
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
        let invocation = invocation("codex", None, InvocationDefaults::Default);

        assert_eq!(invocation.binary, "codex");
        assert_eq!(
            invocation.args,
            ["exec", "--sandbox", "workspace-write"].map(str::to_owned)
        );
        assert!(matches!(invocation.prompt, PromptMode::Arg));
    }

    #[test]
    fn invocation_applies_worker_defaults() {
        let invocation = invocation("codex", None, InvocationDefaults::Worker);

        assert_eq!(invocation.binary, "codex");
        assert_eq!(
            invocation.args,
            ["--dangerously-bypass-approvals-and-sandbox"].map(str::to_owned)
        );
        assert!(matches!(invocation.prompt, PromptMode::Arg));
    }
}
