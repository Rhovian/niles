use super::acp::{AuthSource, ChildCommand, CommandFile, Readiness, SessionConfig};
use super::catalogue::{Catalogue, Model, Models, StdinRequest};
use super::{AgentInvocation, AgentSpec, InvocationDefaults};
use crate::telemetry::SessionLink;
use anyhow::Result;
use camino::Utf8Path;
use serde::Deserialize;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::num::NonZeroU64;
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

#[derive(Debug, Clone, Copy)]
pub struct AgentProfile {
    pub id: &'static str,
    pub(crate) acp: Option<AcpProfile>,
    pub binary: &'static str,
    pub(crate) catalogue: Catalogue,
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
        catalogue: Catalogue {
            args: &["debug", "models"],
            request: None,
            decode: codex_models,
        },
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
        tested_version: "0.172.0",
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
        catalogue: Catalogue {
            args: &[
                "-p",
                "--input-format",
                "stream-json",
                "--output-format",
                "stream-json",
                "--verbose",
                "--no-session-persistence",
                "--strict-mcp-config",
                "--setting-sources",
                "user",
            ],
            request: Some(StdinRequest {
                body: r#"{"type":"control_request","request_id":"catalogue","request":{"subtype":"initialize"}}"#,
                response_id_pointer: Some("/response/request_id"),
            }),
            decode: claude_models,
        },
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
        tested_version: "2.1.332",
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
        catalogue: Catalogue {
            args: &["--mode", "rpc", "--no-session", "--no-extensions"],
            request: Some(StdinRequest {
                body: r#"{"id":"catalogue","type":"get_available_models"}"#,
                response_id_pointer: None,
            }),
            decode: pi_models,
        },
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
        tested_version: "1.6.1",
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
                args.extend([
                    flag.to_owned(),
                    format!("{key}={}", super::system_prompt::toml_basic_string(effort)),
                ]);
            }
        }
    }
    args
}

// serde_json 1.0.154 owns wire decoding; each family's schema stays with its profile.
// https://docs.rs/serde_json/1.0.154/serde_json/fn.from_value.html
#[derive(Deserialize)]
struct ModelList<T> {
    models: Vec<T>,
}
#[derive(Deserialize)]
struct Response<T> {
    response: T,
}
#[derive(Deserialize)]
struct Rpc<T> {
    data: T,
}

fn codex_models(body: Value) -> Result<Models> {
    #[derive(Deserialize)]
    struct Entry {
        slug: String,
        visibility: String,
        supported_reasoning_levels: Vec<Reasoning>,
        context_window: Option<NonZeroU64>,
    }
    #[derive(Deserialize)]
    struct Reasoning {
        effort: String,
    }
    let list: ModelList<Entry> = serde_json::from_value(body)?;
    Ok(list
        .models
        .into_iter()
        .filter(|model| model.visibility != "hide")
        .map(|model| {
            (
                model.slug,
                Model {
                    efforts: model
                        .supported_reasoning_levels
                        .into_iter()
                        .map(|level| level.effort)
                        .collect(),
                    context_window: model.context_window,
                },
            )
        })
        .collect())
}

fn claude_models(body: Value) -> Result<Models> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Entry {
        value: String,
        #[serde(default)]
        supported_effort_levels: Vec<String>,
    }
    let list: Response<Response<ModelList<Entry>>> = serde_json::from_value(body)?;
    // A bare family already expresses the CLI default; it is not another model choice.
    Ok(list
        .response
        .response
        .models
        .into_iter()
        .filter(|model| model.value != "default")
        .map(|model| {
            (
                model.value,
                Model {
                    efforts: model.supported_effort_levels,
                    context_window: None,
                },
            )
        })
        .collect())
}

fn pi_models(body: Value) -> Result<Models> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Entry {
        provider: String,
        id: String,
        reasoning: bool,
        thinking_level_map: Option<BTreeMap<String, Option<Value>>>,
        context_window: Option<NonZeroU64>,
    }
    let list: Rpc<ModelList<Entry>> = serde_json::from_value(body)?;
    // pi-ai 1.6.1 getSupportedThinkingLevels: absent mappings keep base levels, explicit null
    // disables a level, and extended levels need an explicit mapping. This is CLI vocabulary.
    // https://unpkg.com/@earendil-works/pi-ai@1.6.1/dist/models.js
    let base = ["off", "minimal", "low", "medium", "high"];
    let extended = ["xhigh", "max"];
    Ok(list
        .data
        .models
        .into_iter()
        .map(|model| {
            let efforts = base
                .iter()
                .map(|level| (*level, true))
                .chain(extended.iter().map(|level| (*level, false)))
                .filter(|(level, default)| {
                    if !model.reasoning {
                        return *level == "off";
                    }
                    match model
                        .thinking_level_map
                        .as_ref()
                        .and_then(|map| map.get(*level))
                    {
                        Some(None) => false,
                        Some(Some(_)) => true,
                        None => *default,
                    }
                })
                .map(|(level, _)| level.to_owned())
                .collect();
            (
                format!("{}/{}", model.provider, model.id),
                Model {
                    efforts,
                    context_window: model.context_window,
                },
            )
        })
        .collect())
}

impl AcpAdapter {
    // codex-acp 2.9.0 src/index.ts and CodexAcpServer.ts consume this session config.
    pub(crate) fn configure(
        self,
        spec: &AgentSpec,
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
            // claude-agent-acp 0.109.1 new-session-meta.ts / session-mode.ts.
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
                let mut invocation =
                    AgentInvocation::new(spec.clone(), InvocationDefaults::Foreground);
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
