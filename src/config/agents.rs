use crate::config::spec::{AgentConfig, PromptMode};

#[derive(Debug, Clone, Copy)]
pub struct AgentProfile {
    pub id: &'static str,
    pub binary: &'static str,
    pub args: &'static [&'static str],
    pub prompt: PromptMode,
}

const PROFILES: &[AgentProfile] = &[
    AgentProfile {
        id: "codex",
        binary: "codex",
        args: &["exec", "--sandbox", "workspace-write"],
        prompt: PromptMode::Arg,
    },
    AgentProfile {
        id: "claude",
        binary: "claude",
        args: &["-p"],
        prompt: PromptMode::Arg,
    },
];

pub fn known_agent_ids() -> impl Iterator<Item = &'static str> {
    PROFILES.iter().map(|profile| profile.id)
}

pub fn profile_for(agent: &str) -> Option<AgentProfile> {
    PROFILES.iter().find(|profile| profile.id == agent).copied()
}

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

pub fn foreground_binary(agent: &str) -> String {
    default_binary(agent)
}

pub fn foreground_args(_agent: &str) -> Vec<String> {
    Vec::new()
}

pub fn worker_binary(agent: &str) -> String {
    default_binary(agent)
}

pub fn worker_args(agent: &str) -> Vec<String> {
    // Workers run autonomously in their own window: skip interactive approval
    // prompts so a step runs to completion without a human driving each tool.
    match agent {
        "codex" => ["--sandbox", "workspace-write", "--ask-for-approval", "never"]
            .map(str::to_owned)
            .to_vec(),
        "claude" => vec!["--dangerously-skip-permissions".to_owned()],
        _ => default_args(agent),
    }
}

pub fn worker_prompt(agent: &str) -> PromptMode {
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
}
