use camino::Utf8PathBuf;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct RunState {
    pub id: String,
    pub goal: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_file: Option<Utf8PathBuf>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub status: RunStatus,
    pub steps: Vec<StepRecord>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Created,
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct StepRecord {
    pub index: usize,
    pub kind: StepKind,
    pub label: String,
    pub status: StepStatus,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    pub exit_code: Option<i32>,
    pub stdout: Utf8PathBuf,
    pub stderr: Utf8PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff: Option<Utf8PathBuf>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StepKind {
    Agent,
    Command,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Completed,
    Failed,
}

pub fn run_status_label(status: &RunStatus) -> &'static str {
    match status {
        RunStatus::Created => "created",
        RunStatus::Running => "running",
        RunStatus::Completed => "completed",
        RunStatus::Failed => "failed",
    }
}

pub fn step_kind_label(kind: &StepKind) -> &'static str {
    match kind {
        StepKind::Agent => "agent",
        StepKind::Command => "command",
    }
}

pub fn step_status_label(status: &StepStatus) -> &'static str {
    match status {
        StepStatus::Completed => "completed",
        StepStatus::Failed => "failed",
    }
}
