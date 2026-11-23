use super::families::{AcpAdapter, AcpProfile, AgentProfile};
use super::system_prompt::toml_basic_string;
use crate::mcp::{COMMAND, NAME};
use crate::wait::DEFAULT_TIMEOUT;
use agent_client_protocol::v1::schema::{EnvVariable, McpServer, McpServerStdio};
use anyhow::{Context, Result};
use serde_json::{Map, Value, json};
use std::{env, time::Duration};

const ENV: &[&str] = &[
    "HOME",
    "PATH",
    "TMUX",
    "TMUX_PANE",
    "NILES_AGENT_DIR",
    "NILES_SETUP",
];

#[derive(Debug, Clone, Copy)]
pub(super) enum NativeMcp {
    Codex,
    Claude,
}

impl NativeMcp {
    pub(super) fn args(self) -> Result<[String; 2]> {
        let exe = env::current_exe()?;
        let command = exe.to_str().context("non-UTF8 Niles executable")?;
        Ok(match self {
            // Use the same TOML string encoder as Codex model and prompt overrides.
            Self::Codex => {
                let env = ENV
                    .iter()
                    .map(|name| toml_basic_string(name))
                    .collect::<Vec<_>>()
                    .join(", ");
                let command = toml_basic_string(command);
                let arg = toml_basic_string(COMMAND);
                let timeout = tool_timeout().as_secs();
                [
                    "-c".into(),
                    format!(
                        "mcp_servers.{NAME}={{command={command}, args=[{arg}], env_vars=[{env}], tool_timeout_sec={timeout}}}"
                    ),
                ]
            }
            Self::Claude => [
                "--mcp-config".into(),
                json!({"mcpServers": {NAME: {"command": command, "args": [COMMAND], "timeout": tool_timeout().as_millis()}}}).to_string(),
            ],
        })
    }
}

fn acp_server(launch_env: &[(String, String)]) -> Result<McpServer> {
    let mut vars = Vec::new();
    for &name in ENV {
        let value = match launch_env.iter().rev().find(|(key, _)| key == name) {
            Some((_, value)) => Some(value.clone()),
            None => match env::var(name) {
                Ok(value) => Some(value),
                Err(env::VarError::NotPresent) => None,
                Err(err) => {
                    return Err(err).with_context(|| format!("invalid {name} for ACP MCP server"));
                }
            },
        };
        if let Some(value) = value {
            vars.push(EnvVariable::new(name, value));
        }
    }
    Ok(McpServer::Stdio(
        McpServerStdio::new(NAME, env::current_exe()?)
            .args(vec![COMMAND.into()])
            .env(vars),
    ))
}

impl AgentProfile {
    pub(crate) fn mcp_args(&self) -> Result<Option<[String; 2]>> {
        self.niles_mcp.map(NativeMcp::args).transpose()
    }
}

impl AcpProfile {
    pub(crate) fn configure_mcp(&self, env: &mut Vec<(String, String)>) -> Result<Vec<McpServer>> {
        match self.adapter {
            AcpAdapter::Codex => {
                configure_codex(env)?;
                Ok(Vec::new())
            }
            AcpAdapter::Claude => {
                // claude-agent-acp 0.109.1 acp-agent.ts spreads process.env into SDK options.env.
                env.push((
                    "MCP_TOOL_TIMEOUT".into(),
                    tool_timeout().as_millis().to_string(),
                ));
                Ok(vec![acp_server(env)?])
            }
            AcpAdapter::Pi => Ok(Vec::new()),
        }
    }
}

fn tool_timeout() -> Duration {
    DEFAULT_TIMEOUT.saturating_add(Duration::from_secs(100))
}

fn configure_codex(env: &mut [(String, String)]) -> Result<()> {
    // codex-acp 2.9.0 reads CODEX_CONFIG for both thread/start and thread/resume.
    // Its ACP stdio conversion only forwards command, args and env.
    let (_, raw) = env
        .iter_mut()
        .rev()
        .find(|(key, _)| key == "CODEX_CONFIG")
        .context("Codex ACP launch is missing CODEX_CONFIG")?;
    let mut config: Map<String, Value> =
        serde_json::from_str(raw).context("invalid CODEX_CONFIG")?;
    let exe = env::current_exe()?;
    let command = exe.to_str().context("non-UTF8 Niles executable")?;
    config
        .entry("mcp_servers")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .context("CODEX_CONFIG mcp_servers must be an object")?
        .insert(
            NAME.into(),
            json!({
                "command": command,
                "args": [COMMAND],
                "env_vars": ENV,
                "tool_timeout_sec": tool_timeout().as_secs(),
            }),
        );
    *raw = Value::Object(config).to_string();
    Ok(())
}
