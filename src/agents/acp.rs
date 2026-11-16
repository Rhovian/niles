use super::families::{AcpAdapter, AcpProfile};
use agent_client_protocol as sdk;
use anyhow::{Result, ensure};
use sdk::v1::{self, schema as acp};
use serde::{Deserialize, Serialize};

// codex-acp 2.6.0 AuthStatusMeta.ts predates the schema's unstable auth-status payload.
#[derive(Debug, Clone, Serialize, Deserialize, sdk::JsonRpcNotification)]
#[notification(method = "_auth/status_update")]
struct CodexAuthStatus {
    #[serde(rename = "authStatus")]
    identity: Identity,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Identity {
    kind: String,
}

impl AcpProfile {
    pub(crate) fn client(
        self,
        mut authenticated: impl FnMut(Result<()>) -> sdk::Result<()> + Send,
    ) -> v1::Builder<sdk::Client, impl sdk::HandleDispatchFrom<sdk::Agent>> {
        match self.adapter {
            AcpAdapter::Codex => sdk::Client.v1().on_receive_notification(
                async move |status: CodexAuthStatus, _| {
                    authenticated(self.verify_identity(&status.identity.kind))
                },
                sdk::on_receive_notification!(),
            ),
        }
    }
    fn verify_identity(self, kind: &str) -> Result<()> {
        ensure!(
            kind == self.auth_kind,
            "adapter authentication is not the declared subscription"
        );
        Ok(())
    }
    pub(crate) fn verify_initialize(self, init: &acp::InitializeResponse) -> Result<()> {
        match self.adapter {
            AcpAdapter::Codex => ensure!(
                init.agent_capabilities
                    .meta
                    .as_ref()
                    .is_some_and(|meta| meta.contains_key("authStatus")),
                "adapter cannot verify the declared subscription identity"
            ),
        }
        Ok(())
    }
    pub(crate) fn terminal_output(self, update: &acp::ToolCallUpdate) -> Option<&str> {
        match self.adapter {
            AcpAdapter::Codex => update
                .meta
                .as_ref()?
                .get("terminal_output_delta")?
                .get("data")?
                .as_str(),
        }
    }
}
