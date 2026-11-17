use super::families::{AcpAdapter, AcpProfile};
use agent_client_protocol as sdk;
use anyhow::{Context, Result, ensure};
use sdk::v1::schema as acp;
use serde::{Deserialize, Serialize, ser::Error as _};
use tokio::process::Command;

// codex-acp 2.6.0 AuthStatusMeta.ts and claude-agent-acp 0.107.3 auth-status.ts
// share this extension, predating the schema's unstable auth-status payload.
#[derive(Debug, Clone, Serialize, Deserialize, sdk::JsonRpcNotification)]
#[notification(method = "_auth/status_update")]
pub(crate) struct SubscriptionStatus {
    #[serde(rename = "authStatus")]
    identity: Identity,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Identity {
    kind: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AuthSource {
    Notification(&'static str),
    Command(Readiness),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Readiness {
    pub(crate) binary: &'static str,
    pub(crate) args: &'static [&'static str],
    pub(crate) kind: &'static str,
    pub(crate) hint: &'static str,
}
impl SubscriptionStatus {
    pub(crate) fn verify(self, kind: &str) -> Result<()> {
        ensure!(
            kind == self.identity.kind,
            "adapter authentication is not the declared subscription"
        );
        Ok(())
    }
}
impl Readiness {
    pub(crate) async fn check(self) -> Result<()> {
        // pi 1.4.0 cli/auth-command.ts: auth check returns metadata; never request --credentials.
        let output = Command::new(self.binary)
            .args(self.args)
            .output()
            .await
            .context(self.hint)?;
        let failure = || format!("{}: {}", self.hint, String::from_utf8_lossy(&output.stderr));
        ensure!(output.status.success(), "{}", failure());
        let value: serde_json::Value =
            serde_json::from_slice(&output.stdout).with_context(failure)?;
        ensure!(
            value.get("status").and_then(serde_json::Value::as_str) == Some("ready")
                && value.get("authType").and_then(serde_json::Value::as_str) == Some(self.kind),
            "{}",
            self.hint
        );
        Ok(())
    }
}

impl AcpProfile {
    pub(crate) fn verify_initialize(self, init: &acp::InitializeResponse) -> Result<()> {
        match self.auth {
            AuthSource::Notification(_) => ensure!(
                init.agent_capabilities
                    .meta
                    .as_ref()
                    .is_some_and(|meta| meta.contains_key("authStatus")),
                "adapter cannot verify the declared subscription identity"
            ),
            AuthSource::Command(_) => (),
        }
        Ok(())
    }
    pub(crate) fn terminal_output(self, update: &acp::ToolCallUpdate) -> Option<&str> {
        // pi-acp 0.0.34 translate/bash.ts uses the Zed terminal_output delta.
        let key = match self.adapter {
            AcpAdapter::Claude => return None,
            AcpAdapter::Codex => "terminal_output_delta",
            AcpAdapter::Pi => "terminal_output",
        };
        update.meta.as_ref()?.get(key)?.get("data")?.as_str()
    }
}

#[derive(Default, Serialize, Deserialize)]
pub(crate) struct SessionConfig {
    pub(crate) env: Vec<(String, String)>,
    pub(crate) meta: Option<acp::Meta>,
    pub(crate) mode: Option<acp::SessionModeId>,
    #[serde(
        skip_deserializing,
        skip_serializing_if = "Option::is_none",
        serialize_with = "unprepared_child"
    )]
    pub(crate) child: Option<ChildCommand>,
}

pub(crate) struct ChildCommand {
    pub(crate) env_key: &'static str,
    pub(crate) invocation: super::AgentInvocation,
    pub(crate) file: CommandFile,
}
pub(crate) struct CommandFile {
    pub(crate) flag: &'static str,
    pub(crate) body: &'static str,
}
impl SessionConfig {
    pub(crate) fn prepare(&mut self, dir: &camino::Utf8Path) -> Result<()> {
        use crate::{agent_window::shell_quote, store::atomic_write};
        use std::{fs, os::unix::fs::PermissionsExt};
        if let Some(mut child) = self.child.take() {
            let file = dir.join("acp-extension.js");
            atomic_write(&file, child.file.body)?;
            child
                .invocation
                .args
                .extend([child.file.flag.into(), file.into_string()]);
            let command = dir.join("acp-child");
            let args = child
                .invocation
                .args
                .iter()
                .map(|arg| shell_quote(arg))
                .collect::<Vec<_>>()
                .join(" ");
            atomic_write(
                &command,
                format!(
                    "#!/bin/sh\nexec {} \"$@\" {args}\n",
                    shell_quote(child.invocation.binary)
                ),
            )?;
            fs::set_permissions(&command, fs::Permissions::from_mode(0o700))?;
            self.env.push((child.env_key.into(), command.into_string()));
        }
        Ok(())
    }
}

fn unprepared_child<S: serde::Serializer>(
    _: &Option<ChildCommand>,
    _: S,
) -> Result<S::Ok, S::Error> {
    Err(S::Error::custom(
        "prepare the ACP child before serialization",
    ))
}
