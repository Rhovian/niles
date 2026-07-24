use std::{
    ffi::OsString,
    fs,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

use camino::{Utf8Path, Utf8PathBuf};

use super::{
    UsageAttribution,
    home::CODEX_HOME_ENV,
    snapshot::{
        UsageAgent, UsageSnapshot, UsageSnapshotInput, UsageSnapshotUsage, UsageSubject,
        UsageUnavailableReason, snapshot_usage,
    },
};

static ENV_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn missing_transcript_writes_unavailable_snapshot() {
    let root = temp_test_dir("missing-transcript");
    let output_path = root.join("usage.json");
    let attribution = UsageAttribution::ClaudeSession {
        session_id: "00000000-0000-4000-8000-000000000001".to_owned(),
        cwd: root.join("workspace"),
        launched_at: "2026-07-06T00:00:00Z".parse().unwrap(),
        niles_prompt_count: Some(1),
    };
    let input = test_input(output_path.clone(), attribution);

    let path = snapshot_usage(input).unwrap();
    let snapshot = read_snapshot(&path).unwrap();

    assert_eq!(path, output_path);
    assert!(matches!(
        snapshot.usage,
        UsageSnapshotUsage::Unavailable {
            reason: UsageUnavailableReason::Missing,
            ..
        }
    ));

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn ambiguous_codex_candidates_write_unavailable_snapshot() {
    let _guard = ENV_LOCK.lock().unwrap();
    let root = temp_test_dir("ambiguous-codex");
    let codex_home = root.join("codex-home");
    let workspace = root.join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    let sessions = codex_home.join("sessions/2026/07/06");
    fs::create_dir_all(&sessions).unwrap();
    write_codex_rollout(
        &sessions.join("rollout-a.jsonl"),
        &workspace,
        "session-a",
        "2026-07-06T00:00:00Z",
    );
    write_codex_rollout(
        &sessions.join("rollout-b.jsonl"),
        &workspace,
        "session-b",
        "2026-07-06T00:00:01Z",
    );
    let previous = set_env(CODEX_HOME_ENV, Some(codex_home.as_str()));

    let path = snapshot_usage(UsageSnapshotInput {
        subject: UsageSubject::Worker {
            id: "worker-1".to_owned(),
            task_label: None,
        },
        agent: UsageAgent {
            spec: "codex:gpt-5.5:xhigh".to_owned(),
            family: Some("codex".to_owned()),
            model: Some("gpt-5.5".to_owned()),
            effort: Some("xhigh".to_owned()),
        },
        attribution: Some(UsageAttribution::CodexCwdTime {
            cwd: workspace,
            launched_at: "2026-07-06T00:00:00Z".parse().unwrap(),
            niles_prompt_count: Some(1),
        }),
        started_at: Some("2026-07-06T00:00:00Z".parse().unwrap()),
        finished_at: "2026-07-06T00:01:00Z".parse().unwrap(),
        output_path: root.join("usage.json"),
    })
    .unwrap();
    restore_env(CODEX_HOME_ENV, previous);

    let snapshot = read_snapshot(&path).unwrap();
    assert!(matches!(
        snapshot.usage,
        UsageSnapshotUsage::Unavailable {
            reason: UsageUnavailableReason::AmbiguousCodexCandidates,
            ..
        }
    ));

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn codex_parse_error_writes_unavailable_snapshot() {
    let _guard = ENV_LOCK.lock().unwrap();
    let root = temp_test_dir("parse-error-codex");
    let codex_home = root.join("codex-home");
    let workspace = root.join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    let sessions = codex_home.join("sessions/2026/07/06");
    fs::create_dir_all(&sessions).unwrap();
    fs::write(
        sessions.join("rollout-bad.jsonl"),
        format!(
            "{{\"type\":\"session_meta\",\"payload\":{{\"session_id\":\"session-a\",\"cwd\":\"{}\",\"timestamp\":\"2026-07-06T00:00:00Z\"}}}}\n{{\"type\":\"event_msg\"\n{{\"type\":\"event_msg\",\"payload\":{{\"type\":\"token_count\",\"info\":{{\"total_token_usage\":{{\"input_tokens\":1,\"cached_input_tokens\":0,\"output_tokens\":1,\"reasoning_output_tokens\":0,\"total_tokens\":2}}}}}}}}\n",
            workspace
        ),
    )
    .unwrap();
    let previous = set_env(CODEX_HOME_ENV, Some(codex_home.as_str()));

    let path = snapshot_usage(UsageSnapshotInput {
        subject: UsageSubject::Worker {
            id: "worker-1".to_owned(),
            task_label: None,
        },
        agent: UsageAgent {
            spec: "codex:gpt-5.5:xhigh".to_owned(),
            family: Some("codex".to_owned()),
            model: Some("gpt-5.5".to_owned()),
            effort: Some("xhigh".to_owned()),
        },
        attribution: Some(UsageAttribution::CodexCwdTime {
            cwd: workspace,
            launched_at: "2026-07-06T00:00:00Z".parse().unwrap(),
            niles_prompt_count: Some(1),
        }),
        started_at: Some("2026-07-06T00:00:00Z".parse().unwrap()),
        finished_at: "2026-07-06T00:01:00Z".parse().unwrap(),
        output_path: root.join("usage.json"),
    })
    .unwrap();
    restore_env(CODEX_HOME_ENV, previous);

    let snapshot = read_snapshot(&path).unwrap();
    assert!(matches!(
        snapshot.usage,
        UsageSnapshotUsage::Unavailable {
            reason: UsageUnavailableReason::ParseError,
            ..
        }
    ));

    fs::remove_dir_all(root).unwrap();
}

fn test_input(output_path: Utf8PathBuf, attribution: UsageAttribution) -> UsageSnapshotInput {
    UsageSnapshotInput {
        subject: UsageSubject::Worker {
            id: "worker-1".to_owned(),
            task_label: Some("tokledger".to_owned()),
        },
        agent: UsageAgent {
            spec: "claude:sonnet:high".to_owned(),
            family: Some("claude".to_owned()),
            model: Some("sonnet".to_owned()),
            effort: Some("high".to_owned()),
        },
        attribution: Some(attribution),
        started_at: Some("2026-07-06T00:00:00Z".parse().unwrap()),
        finished_at: "2026-07-06T00:01:00Z".parse().unwrap(),
        output_path,
    }
}

fn read_snapshot(path: &Utf8Path) -> anyhow::Result<UsageSnapshot> {
    crate::schema::read_json(path, crate::schema::ArtifactKind::UsageSnapshot)
}

fn temp_test_dir(label: &str) -> Utf8PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "niles-usage-{label}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    Utf8PathBuf::from_path_buf(dir).unwrap()
}

fn write_codex_rollout(path: &Utf8Path, workspace: &Utf8Path, session_id: &str, timestamp: &str) {
    fs::write(
        path,
        format!(
            "{{\"type\":\"session_meta\",\"payload\":{{\"session_id\":\"{session_id}\",\"cwd\":\"{workspace}\",\"timestamp\":\"{timestamp}\"}}}}\n{{\"type\":\"event_msg\",\"payload\":{{\"type\":\"token_count\",\"info\":{{\"total_token_usage\":{{\"input_tokens\":1,\"cached_input_tokens\":0,\"output_tokens\":1,\"reasoning_output_tokens\":0,\"total_tokens\":2}}}}}}}}\n"
        ),
    )
    .unwrap();
}

fn set_env(key: &str, value: Option<&str>) -> Option<OsString> {
    let previous = std::env::var_os(key);
    match value {
        Some(value) => unsafe {
            std::env::set_var(key, value);
        },
        None => unsafe {
            std::env::remove_var(key);
        },
    }
    previous
}

fn restore_env(key: &str, previous: Option<OsString>) {
    match previous {
        Some(value) => unsafe {
            std::env::set_var(key, value);
        },
        None => unsafe {
            std::env::remove_var(key);
        },
    }
}
