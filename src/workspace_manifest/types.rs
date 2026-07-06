use std::fmt;

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

pub(in crate::workspace_manifest) const WORKER_REVIEW_LOOP_SUMMARY: &str =
    "planner -> worker <verification> <-> reviewer -> CONSENSUS OR ESCALATE";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "WorkspaceManifestWire")]
pub struct WorkspaceManifest {
    pub manager: String,
    pub planner: String,
    pub worker: String,
    pub reviewer: String,
    pub validation_command: String,
    pub flow: Vec<WorkspaceFlowRole>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkspaceManifestWire {
    manager: String,
    planner: String,
    worker: String,
    reviewer: String,
    validation_command: String,
    #[serde(default = "initial_flow")]
    flow: Vec<WorkspaceFlowRole>,
    #[serde(default, rename = "niles_schema")]
    _niles_schema: Option<u64>,
}

impl From<WorkspaceManifestWire> for WorkspaceManifest {
    fn from(wire: WorkspaceManifestWire) -> Self {
        Self {
            manager: wire.manager,
            planner: wire.planner,
            worker: wire.worker,
            reviewer: wire.reviewer,
            validation_command: wire.validation_command,
            flow: wire.flow,
        }
    }
}

impl Default for WorkspaceManifest {
    fn default() -> Self {
        Self {
            manager: "claude".to_owned(),
            planner: "claude".to_owned(),
            worker: "codex".to_owned(),
            reviewer: "claude".to_owned(),
            validation_command: "test".to_owned(),
            flow: super::initial_flow(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceFlowRole {
    Planner,
    Worker,
    Validation,
    Reviewer,
}

impl WorkspaceFlowRole {
    pub(in crate::workspace_manifest) fn parse(value: &str) -> Result<Self> {
        match value {
            "planner" => Ok(Self::Planner),
            "worker" => Ok(Self::Worker),
            "validation" => Ok(Self::Validation),
            "reviewer" => Ok(Self::Reviewer),
            _ => bail!("unknown workspace role `{value}`"),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Planner => "planner",
            Self::Worker => "worker",
            Self::Validation => "validation",
            Self::Reviewer => "reviewer",
        }
    }
}

impl fmt::Display for WorkspaceFlowRole {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

pub fn initial_flow() -> Vec<WorkspaceFlowRole> {
    vec![
        WorkspaceFlowRole::Planner,
        WorkspaceFlowRole::Worker,
        WorkspaceFlowRole::Reviewer,
    ]
}

pub fn flow_summary(flow: &[WorkspaceFlowRole]) -> String {
    if flow.is_empty() {
        return "<empty>".to_owned();
    }

    if is_worker_review_loop(flow) {
        return WORKER_REVIEW_LOOP_SUMMARY.to_owned();
    }

    flow.iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(" -> ")
}

fn is_worker_review_loop(flow: &[WorkspaceFlowRole]) -> bool {
    matches!(
        flow,
        [
            WorkspaceFlowRole::Planner,
            WorkspaceFlowRole::Worker,
            WorkspaceFlowRole::Reviewer
        ] | [
            WorkspaceFlowRole::Planner,
            WorkspaceFlowRole::Worker,
            WorkspaceFlowRole::Validation,
            WorkspaceFlowRole::Reviewer
        ]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_defaults_carry_flow() {
        let manifest = WorkspaceManifest::default();

        assert_eq!(
            manifest.flow,
            vec![
                WorkspaceFlowRole::Planner,
                WorkspaceFlowRole::Worker,
                WorkspaceFlowRole::Reviewer,
            ]
        );
        assert_eq!(flow_summary(&manifest.flow), WORKER_REVIEW_LOOP_SUMMARY);
    }

    #[test]
    fn manifest_worker_validation_reviewer_flow_renders_worker_review_loop() {
        let flow = [
            WorkspaceFlowRole::Planner,
            WorkspaceFlowRole::Worker,
            WorkspaceFlowRole::Validation,
            WorkspaceFlowRole::Reviewer,
        ];

        assert_eq!(flow_summary(&flow), WORKER_REVIEW_LOOP_SUMMARY);
    }
}
