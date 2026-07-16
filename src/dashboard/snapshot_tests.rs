use std::{
    fs,
    time::{SystemTime, UNIX_EPOCH},
};

use camino::Utf8PathBuf;
use chrono::{Duration, Utc};

use crate::state::{StepKind, StepStatus};
use crate::{
    session::SessionMeta,
    tmux::{SessionName, TargetState, TmuxWindowSnapshot},
    worker::WorkerDashboardMeta,
};

use super::snapshot::{
    DashboardRole, DashboardRow, DashboardSnapshot, DashboardSources, ManagerLifecycle,
    PaneLiveness, RunStepDashboardMeta, TmuxWindows, assemble_snapshot,
};
use super::status::WakeState;
use super::usage_cell::DashboardUsageCache;

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
    let worker_dir = root.join("worker");
    fs::create_dir_all(&worker_dir).unwrap();
    let worker_metadata = worker_dir.join("meta.json");
    fs::write(&worker_metadata, "{}").unwrap();
    let run_dir = root.join("run");
    fs::create_dir_all(&run_dir).unwrap();
    let mut usage_cache = DashboardUsageCache::default();

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
                usage_attribution: None,
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
                usage_attribution: None,
                worker_dir,
                metadata: worker_metadata,
                window: "niles:niles-impl53".to_owned(),
                target_state: TargetState::Live,
                status: worker_status,
            }],
            steps: vec![RunStepDashboardMeta {
                run_dir,
                run_id: "runabcdef".to_owned(),
                index: 1,
                role: Some("reviewer".to_owned()),
                kind: StepKind::Agent,
                label: "codex".to_owned(),
                agent_family: Some("codex".to_owned()),
                model: Some("gpt-5.5".to_owned()),
                effort: Some("high".to_owned()),
                usage_attribution: None,
                usage: None,
                step_status: StepStatus::Completed,
                started_at: Some(now - Duration::seconds(245)),
                finished_at: Some(now),
                window: "niles-codex-review-s1-abcdef".to_owned(),
                status_log: step_status,
            }],
            progress: None,
            messages: Vec::new(),
        },
        now,
        &mut usage_cache,
        true,
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
        row(&snapshot, "niles:niles-extra").role,
        DashboardRole::UnknownNilesWindow
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn tmux_failure_keeps_metadata_rows_with_unknown_panes() {
    let root = temp_dir("tmux-failure");
    let now = Utc::now();
    let mut usage_cache = DashboardUsageCache::default();

    let snapshot = assemble_snapshot(
        DashboardSources {
            tmux: TmuxWindows::Unavailable("tmux unavailable".to_owned()),
            manager: Some(SessionMeta {
                id: "session".to_owned(),
                agent: "codex".to_owned(),
                agent_family: None,
                model: None,
                effort: None,
                usage_attribution: None,
                created_at: now,
                workspace: root.clone(),
                brief: root.join("manager.md"),
                window: Some("niles:niles-manager".to_owned()),
                launch: Some(root.join("launch.sh")),
            }),
            workers: Vec::new(),
            steps: Vec::new(),
            progress: None,
            messages: Vec::new(),
        },
        now,
        &mut usage_cache,
        true,
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

#[test]
fn dashboard_keys_duplicate_window_names_by_full_target() {
    let root = temp_dir("duplicate-window-targets");
    let worker_status = root.join("worker-status.log");
    fs::write(&worker_status, "working: started\n").unwrap();
    let worker_dir = root.join("worker");
    fs::create_dir_all(&worker_dir).unwrap();
    let worker_metadata = worker_dir.join("meta.json");
    fs::write(&worker_metadata, "{}").unwrap();
    let now = Utc::now();
    let mut usage_cache = DashboardUsageCache::default();

    let snapshot = assemble_snapshot(
        DashboardSources {
            tmux: TmuxWindows::Available(vec![
                tmux_window_in("home", "niles", false, Some(100), Some("niles")),
                tmux_window_in("home", "niles-manager", false, Some(101), Some("codex")),
                tmux_window_in("home", "niles-shared", false, Some(102), Some("codex")),
                tmux_window_in("other", "niles-shared", false, Some(103), Some("zsh")),
            ]),
            manager: Some(SessionMeta {
                id: "session".to_owned(),
                agent: "codex".to_owned(),
                agent_family: None,
                model: None,
                effort: None,
                usage_attribution: None,
                created_at: now,
                workspace: root.clone(),
                brief: root.join("manager.md"),
                window: Some("home:niles-manager".to_owned()),
                launch: Some(root.join("launch.sh")),
            }),
            workers: vec![WorkerDashboardMeta {
                id: "shared".to_owned(),
                agent: "codex".to_owned(),
                agent_family: None,
                model: None,
                effort: None,
                task_label: None,
                created_at: Some(now),
                usage_attribution: None,
                worker_dir,
                metadata: worker_metadata,
                window: "home:niles-shared".to_owned(),
                target_state: TargetState::WindowDead,
                status: worker_status,
            }],
            steps: Vec::new(),
            progress: None,
            messages: Vec::new(),
        },
        now,
        &mut usage_cache,
        true,
    );

    let worker = row(&snapshot, "niles-shared");
    assert_eq!(worker.role, DashboardRole::Worker);
    assert_eq!(worker.pane.liveness, PaneLiveness::Live);
    assert_eq!(
        row(&snapshot, "other:niles-shared").role,
        DashboardRole::UnknownNilesWindow
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn dashboard_usage_code_does_not_call_snapshot_writer() {
    assert!(!include_str!("usage_cell.rs").contains("usage::snapshot_usage"));
    assert!(!include_str!("snapshot.rs").contains("usage::snapshot_usage"));
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
    tmux_window_in("niles", name, dead, pid, command)
}

fn tmux_window_in(
    session: &str,
    name: &str,
    dead: bool,
    pid: Option<u32>,
    command: Option<&str>,
) -> TmuxWindowSnapshot {
    TmuxWindowSnapshot {
        session: SessionName::new(session).unwrap(),
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
