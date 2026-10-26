use std::{
    fs::{self, File},
    os::unix::fs::OpenOptionsExt,
};

use anyhow::{Context, Result, ensure};
use camino::{Utf8Path, Utf8PathBuf};

use crate::{
    store,
    util::append_line,
    wake::{self, WakeKind},
    workspace_manifest::Rounds,
};

struct LockedState {
    _lock: File,
    rounds: Vec<usize>,
    log_len: usize,
    count: usize,
}

fn locked_state(dir: &Utf8Path) -> Result<LockedState> {
    let path = dir.join("stage.lock");
    let lock = crate::wait::cursor::open_lock_file(&path)?;
    lock.lock()
        .with_context(|| format!("failed to lock {path}"))?;
    let log = wake::status_log_path(dir);
    let bytes = fs::read(&log).with_context(|| format!("failed to read {log}"))?;
    let last_done = wake::complete_lines(&bytes)
        .filter(|(_, line)| WakeKind::parse_line(line) == Some(WakeKind::Done))
        .map(|(end, _)| end)
        .last();
    #[expect(
        clippy::disallowed_methods,
        reason = "an absent ledger means no completed rounds"
    )]
    let rounds: Vec<usize> =
        store::read_optional_json(&dir.join("gates.json"))?.unwrap_or_default();
    // Equal offsets mean stage immediately followed done without another status.
    let count = rounds
        .iter()
        .filter(|length| last_done.is_none_or(|end| **length >= end))
        .count();
    Ok(LockedState {
        _lock: lock,
        rounds,
        log_len: bytes.len(),
        count,
    })
}

fn report_path(dir: &Utf8Path) -> Result<Utf8PathBuf> {
    let report = super::meta::report_path(dir);
    ensure!(
        fs::metadata(&report)
            .with_context(|| format!("missing report: {report}"))?
            .len()
            > 0,
        "report is empty: {report}"
    );
    Ok(report)
}

pub(super) fn done(dir: &Utf8Path, detail: &str) -> Result<()> {
    let report = report_path(dir)?;
    let meta = super::meta::read_meta(dir)?;
    let state = if meta.on_stage.is_empty() && meta.gates.is_none() {
        None
    } else {
        Some(locked_state(dir)?)
    };
    if let Some(state) = &state {
        ensure!(
            state.count > 0,
            "stage first: run niles agent status stage <summary>"
        );
    }
    append_line(
        &wake::status_log_path(dir),
        &wake::line(WakeKind::Done, &format!("{detail}; report: {report}")),
    )
}

pub(super) fn stage(dir: &Utf8Path, detail: &str) -> Result<()> {
    let report = report_path(dir)?;
    let meta = super::meta::read_meta(dir)?;
    ensure!(
        !meta.on_stage.is_empty() || meta.gates.is_some(),
        "skill {} has no stage configuration; report done",
        meta.skill
    );
    let mut state = locked_state(dir)?;
    let limit = u8::from(
        meta.gates
            .as_ref()
            .map_or_else(Rounds::default, |gates| gates.rounds),
    );
    ensure!(
        state.count < usize::from(limit),
        "round limit {limit} reached; report done"
    );
    let skills = super::skills::stage_skills(&meta.on_stage, &meta.project)?;
    state.rounds.push(state.log_len);
    // Atomic replacement keeps an interrupted stage from truncating previous rounds.
    let temporary = dir.join("gates.tmp");
    let file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&temporary)
        .with_context(|| format!("failed to open {temporary}"))?;
    serde_json::to_writer(file, &state.rounds)?;
    fs::rename(&temporary, dir.join("gates.json")).context("failed to replace gates.json")?;
    append_line(
        &wake::status_log_path(dir),
        &wake::line(
            WakeKind::Stage,
            &format!("{detail}; gates: none; report: {report}"),
        ),
    )?;
    for skill in skills {
        println!("on_stage_skill: {skill}\n{}", skill.body);
    }
    println!("Apply the post-work skills, then report `done`.");
    Ok(())
}
