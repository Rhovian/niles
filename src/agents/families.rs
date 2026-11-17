use super::acp::{AuthSource, ChildCommand, CommandFile, Readiness, SessionConfig};
use crate::telemetry::SessionLink;
use anyhow::Result;
use camino::Utf8Path;
use serde_json::{Map, Value};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AcpProfile {
    pub(crate) binary: &'static str,
    pub(crate) subscription: &'static str,
    pub(crate) auth: AuthSource,
    pub(crate) adapter: AcpAdapter,
    pub(crate) remove_env: &'static [&'static str],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AcpAdapter {
    Codex,
    Claude,
    Pi,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentProfile {
    pub id: &'static str,
    pub(crate) acp: Option<AcpProfile>,
    pub binary: &'static str,
    pub tested_version: &'static str,
    pub foreground_args: &'static [&'static str],
    pub worker_args: &'static [&'static str],
    /// Switches a worker launch to a single non-interactive turn; appended after
    /// `worker_args` because codex's subcommand must follow its root flags.
    pub headless: &'static [&'static str],
    pub system_prompt: SystemPromptArg,
    tier_args: TierArgs,
    pub launch_env: &'static [(&'static str, &'static str)],
    pub composer: Option<&'static [&'static str]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemPromptArg {
    Flag(&'static str),
    Config {
        flag: &'static str,
        key: &'static str,
    },
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

/// The session niles can read usage back from, chosen at launch. A family not listed has none.
pub(crate) fn session_link(family: &str, dir: &Utf8Path) -> Option<SessionLink> {
    Some(match profile_for(family)?.id {
        "claude" => SessionLink::Claude {
            session_id: Uuid::new_v4().to_string(),
        },
        "codex" => SessionLink::Codex,
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
        acp: Some(AcpProfile {
            binary: "codex-acp",
            adapter: AcpAdapter::Codex,
            subscription: "ChatGPT subscription",
            auth: AuthSource::Notification("account"),
            remove_env: &[
                "CODEX_API_KEY",
                "OPENAI_API_KEY",
                "DEFAULT_AUTH_REQUEST",
                "MODEL_PROVIDER",
            ],
        }),
        tested_version: "0.165.0",
        // The shared daemon lets a running session continue in the background, out of its tmux
        // pane.
        foreground_args: &["--no-daemon"],
        worker_args: &["--no-daemon", "--dangerously-bypass-approvals-and-sandbox"],
        headless: &["exec", "--skip-git-repo-check"],
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
        acp: Some(AcpProfile {
            binary: "claude-agent-acp",
            adapter: AcpAdapter::Claude,
            subscription: "Claude.ai subscription",
            auth: AuthSource::Notification("account"),
            remove_env: &[
                "ANTHROPIC_API_KEY",
                "ANTHROPIC_AUTH_TOKEN",
                "ANTHROPIC_BASE_URL",
                "CLAUDE_CODE_USE_BEDROCK",
                "CLAUDE_CODE_USE_VERTEX",
                "CLAUDE_CODE_USE_FOUNDRY",
            ],
        }),
        tested_version: "2.1.312",
        foreground_args: &[],
        worker_args: &["--dangerously-skip-permissions"],
        headless: &["-p"],
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
        id: "pi",
        binary: "pi",
        acp: Some(AcpProfile {
            binary: "pi-acp",
            adapter: AcpAdapter::Pi,
            subscription: "ChatGPT subscription (pi openai OAuth)",
            auth: AuthSource::Command(Readiness {
                binary: "pi",
                args: &["auth", "check", "--provider", "openai", "--json"],
                kind: "oauth",
                hint: "pi requires its own openai ChatGPT login; run pi /login and select openai",
            }),
            remove_env: &["OPENAI_API_KEY", "CODEX_API_KEY", "OPENAI_BASE_URL"],
        }),
        tested_version: "1.4.0",
        foreground_args: &[],
        worker_args: &[],
        headless: &["-p"],
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

impl AcpAdapter {
    // codex-acp 2.6.0 src/index.ts and CodexAcpServer.ts consume this session config.
    pub(crate) fn configure(
        self,
        spec: &super::AgentSpec,
        primary: &str,
        worker: bool,
    ) -> Result<SessionConfig> {
        Ok(match self {
            Self::Codex => {
                let mut config = Map::from_iter([
                    ("forced_login_method".into(), "chatgpt".into()),
                    ("model_provider".into(), "openai".into()),
                    ("developer_instructions".into(), primary.into()),
                    ("approvals_reviewer".into(), "user".into()),
                ]);
                config.extend(spec.model().map(|model| ("model".into(), model.into())));
                config.extend(
                    spec.effort()
                        .map(|effort| ("model_reasoning_effort".into(), effort.into())),
                );
                SessionConfig {
                    env: vec![
                        ("CODEX_CONFIG".into(), Value::Object(config).to_string()),
                        (
                            "INITIAL_AGENT_MODE".into(),
                            if worker {
                                "agent-full-access"
                            } else {
                                "workspace-write"
                            }
                            .into(),
                        ),
                    ],
                    ..Default::default()
                }
            }
            // claude-agent-acp 0.107.3 new-session-meta.ts / session-mode.ts.
            // permissionMode in SDK options is ignored; use ACP session/set_mode.
            Self::Claude => {
                let mut options = Map::new();
                options.extend(spec.model().map(|value| ("model".into(), value.into())));
                options.extend(spec.effort().map(|value| ("effort".into(), value.into())));
                SessionConfig {
                    meta: Some(Map::from_iter([
                        (
                            "systemPrompt".into(),
                            serde_json::json!({"type":"preset", "preset":"claude_code", "append":primary}),
                        ),
                        ("claudeCode".into(), serde_json::json!({"options": options})),
                    ])),
                    mode: Some(
                        if worker {
                            "bypassPermissions"
                        } else {
                            "default"
                        }
                        .into(),
                    ),
                    ..Default::default()
                }
            }
            Self::Pi => {
                anyhow::ensure!(
                    spec.model()
                        .is_some_and(|model| model.starts_with("openai/")),
                    "headless pi requires an explicit openai/<model> and pi's own ChatGPT login"
                );
                let mut invocation = super::AgentInvocation::new(
                    spec.clone(),
                    super::InvocationDefaults::Foreground,
                );
                invocation
                    .args
                    .extend(["--provider".into(), "openai".into()]);
                invocation.pass_primary(primary)?;
                SessionConfig {
                    child: Some(ChildCommand {
                        env_key: "PI_ACP_PI_COMMAND",
                        invocation,
                        file: CommandFile {
                            flag: "--extension",
                            body: include_str!("pi-subscription.js"),
                        },
                    }),
                    ..Default::default()
                }
            }
        })
    }
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
