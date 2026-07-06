use std::{
    fs,
    time::{SystemTime, UNIX_EPOCH},
};

use camino::Utf8PathBuf;
use chrono::{Duration, Utc};

use crate::{session::SessionMeta, tmux::TmuxWindowSnapshot, worker::WorkerDashboardMeta};

use super::snapshot::{
    DashboardRole, DashboardRow, DashboardSnapshot, DashboardSources, ManagerLifecycle,
    PaneLiveness, RunStepDashboardMeta, TmuxWindows, assemble_snapshot,
};
use super::status::WakeState;

#[test]
fn assembles_rows_from_tmux_and_metadata_sources() {
    let root = temp_dir("assembly");
    let worker_status = root.join("worker-status.log");
    let step_status = root.join("run-status.log");
    fs::write(
        &worker_status,
        "working: started\ndone: first\nblocked: needs input\n",
    )
    .unwrap();
    fs::write(&step_status, "working: step\ndone: step 1 complete\n").unwrap();
    let now = Utc::now();

    let snapshot = assemble_snapshot(
        DashboardSources {
            tmux: TmuxWindows::Available(vec![
                tmux_window("niles", false, Some(100), Some("niles")),
                tmux_window("niles-manager", true, Some(101), Some("claude")),
                tmux_window("niles-impl53", false, Some(102), Some("codex")),
                tmux_window(
                    "niles-codex-review-s1-abcdef",
                    false,
                    Some(103),
                    Some("codex"),
                ),
                tmux_window("niles-extra", false, Some(104), Some("zsh")),
            ]),
            manager: Some(SessionMeta {
                id: "20260706T000000000000000Z".to_owned(),
                agent: "claude:opus:max".to_owned(),
                agent_family: Some("claude".to_owned()),
                model: Some("opus".to_owned()),
                effort: Some("max".to_owned()),
                created_at: now - Duration::seconds(90),
                workspace: root.clone(),
                brief: root.join("manager.md"),
                window: Some("niles:niles-manager".to_owned()),
                launch: Some(root.join("launch.sh")),
            }),
            workers: vec![WorkerDashboardMeta {
                id: "impl53".to_owned(),
                agent: "codex:gpt-5.5:xhigh".to_owned(),
                agent_family: Some("codex".to_owned()),
                model: Some("gpt-5.5".to_owned()),
                effort: Some("xhigh".to_owned()),
                task_label: Some("homewin".to_owned()),
                created_at: Some(now - Duration::seconds(125)),
                window: "niles:niles-impl53".to_owned(),
                status: worker_status,
            }],
            steps: vec![RunStepDashboardMeta {
                run_id: "runabcdef".to_owned(),
                index: 1,
                label: "codex".to_owned(),
                agent_family: Some("codex".to_owned()),
                model: Some("gpt-5.5".to_owned()),
                effort: Some("high".to_owned()),
                started_at: Some(now - Duration::seconds(245)),
                window: "niles-codex-review-s1-abcdef".to_owned(),
                status: step_status,
            }],
            messages: Vec::new(),
        },
        now,
    );

    assert_eq!(snapshot.rows.len(), 5);
    assert_eq!(snapshot.manager_lifecycle, ManagerLifecycle::Exited);

    let manager = row(&snapshot, "niles-manager");
    assert_eq!(manager.role, DashboardRole::Manager);
    assert_eq!(manager.agent, "claude/opus/max");
    assert_eq!(manager.pane.liveness, PaneLiveness::Dead);
    assert_eq!(manager.wall, "1m");

    let worker = row(&snapshot, "niles-impl53");
    assert_eq!(worker.role, DashboardRole::Worker);
    assert_eq!(worker.subject, "impl53 (homewin)");
    assert_eq!(worker.agent, "codex/gpt-5.5/xhigh");
    assert_eq!(worker.pane.liveness, PaneLiveness::Live);
    assert_eq!(worker.wall, "2m");
    assert_eq!(worker.wake, WakeState::Pending);
    assert_eq!(worker.last_status, "blocked: needs input");

    let step = row(&snapshot, "niles-codex-review-s1-abcdef");
    assert_eq!(step.role, DashboardRole::RunStep);
    assert_eq!(step.agent, "codex/gpt-5.5/high");
    assert_eq!(step.wall, "4m");
    assert_eq!(step.wake, WakeState::NotApplicable);
    assert_eq!(step.last_status, "done: step 1 complete");
    assert_eq!(
        row(&snapshot, "niles-extra").role,
        DashboardRole::UnknownNilesWindow
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn tmux_failure_keeps_metadata_rows_with_unknown_panes() {
    let root = temp_dir("tmux-failure");
    let now = Utc::now();

    let snapshot = assemble_snapshot(
        DashboardSources {
            tmux: TmuxWindows::Unavailable("tmux unavailable".to_owned()),
            manager: Some(SessionMeta {
                id: "session".to_owned(),
                agent: "codex".to_owned(),
                agent_family: None,
                model: None,
                effort: None,
                created_at: now,
                workspace: root.clone(),
                brief: root.join("manager.md"),
                window: Some("niles:niles-manager".to_owned()),
                launch: Some(root.join("launch.sh")),
            }),
            workers: Vec::new(),
            steps: Vec::new(),
            messages: Vec::new(),
        },
        now,
    );

    assert_eq!(snapshot.rows.len(), 2);
    assert!(
        snapshot
            .rows
            .iter()
            .all(|row| row.pane.liveness == PaneLiveness::Unknown)
    );
    assert_eq!(snapshot.manager_lifecycle, ManagerLifecycle::Unknown);
    assert_eq!(snapshot.messages, vec!["tmux unavailable".to_owned()]);
    fs::remove_dir_all(root).unwrap();
}

fn row<'a>(snapshot: &'a DashboardSnapshot, window: &str) -> &'a DashboardRow {
    snapshot
        .rows
        .iter()
        .find(|row| row.window == window)
        .unwrap()
}

fn tmux_window(
    name: &str,
    dead: bool,
    pid: Option<u32>,
    command: Option<&str>,
) -> TmuxWindowSnapshot {
    TmuxWindowSnapshot {
        index: 0,
        name: name.to_owned(),
        active: false,
        pane_pid: pid,
        pane_current_command: command.map(str::to_owned),
        pane_dead: dead,
    }
}

fn temp_dir(label: &str) -> Utf8PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = Utf8PathBuf::from_path_buf(std::env::temp_dir().join(format!(
        "niles-dashboard-snapshot-{label}-{}-{nanos}",
        std::process::id()
    )))
    .unwrap();
    fs::create_dir_all(&path).unwrap();
    path
}
