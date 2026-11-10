use crate::skills::SkillName;
use crate::store;
use crate::util::append_line;
use crate::wait::cursor;
use crate::wake::{self, WakeKind};
use crate::workspace_manifest::Rounds;
use anyhow::{Context, Result, ensure};
use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};

struct LockedState {
    _lock: File,
    rounds: Vec<Round>,
    log_len: usize,
    count: usize,
}

#[derive(Serialize, Deserialize)]
struct Round {
    log_len: usize,
    failed: Vec<SkillName>,
}

fn limit(meta: &super::meta::WorkerMeta) -> usize {
    usize::from(u8::from(
        meta.gates
            .as_ref()
            .map_or_else(Rounds::default, |gates| gates.rounds),
    ))
}

fn retry(count: usize, limit: usize) -> String {
    format!("gates failing; fix and stage again (round {count} of {limit}), or report `blocked`")
}

fn locked_state(dir: &Utf8Path) -> Result<LockedState> {
    let path = dir.join("stage.lock");
    let lock = cursor::open_lock_file(&path)?;
    lock.lock()
        .with_context(|| format!("failed to lock {path}"))?;
    let log = wake::status_log_path(dir);
    let bytes = fs::read(&log).with_context(|| format!("failed to read {log}"))?;
    let boundary = wake::complete_lines(&bytes)
        .filter(|(_, line)| {
            *line == wake::FOLLOWUP_STARTED || WakeKind::parse_line(line) == Some(WakeKind::Done)
        })
        .map(|(end, _)| end)
        .last();
    #[expect(
        clippy::disallowed_methods,
        reason = "an absent ledger means no completed rounds"
    )]
    let rounds: Vec<Round> =
        store::read_optional_json(&dir.join("gates.json"))?.unwrap_or_default();
    // Equal offsets mean stage immediately followed the boundary without another status.
    let count = rounds
        .iter()
        .filter(|round| boundary.is_none_or(|end| round.log_len >= end))
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
    let mut annotation = String::new();
    if let Some(state) = &state {
        ensure!(
            state.count > 0,
            "stage first: run niles agent status stage <summary>"
        );
        let latest = state
            .rounds
            .last()
            .context("staged worker has no gate round")?;
        if let Some(gates) = meta.gates.as_ref().filter(|gates| !gates.post.is_empty()) {
            if latest.failed.is_empty() {
                annotation = format!("; gates: passed ({})", gates.post.len());
            } else {
                ensure!(
                    state.count >= limit(&meta),
                    "{}",
                    retry(state.count, limit(&meta))
                );
                let names: Vec<_> = latest.failed.iter().map(SkillName::as_str).collect();
                annotation = format!("; gates: failed {} (round limit reached)", names.join(", "));
            }
        }
    }
    append_line(
        &wake::status_log_path(dir),
        &wake::line(
            WakeKind::Done,
            &format!("{detail}{annotation}; report: {report}"),
        ),
    )
}

#[expect(
    clippy::arithmetic_side_effects,
    reason = "rounds own storage; current count is below the round limit"
)]
pub(super) fn stage(dir: &Utf8Path, detail: &str) -> Result<()> {
    let report = report_path(dir)?;
    let meta = super::meta::read_meta(dir)?;
    ensure!(
        !meta.on_stage.is_empty() || meta.gates.is_some(),
        "skill {} has no stage configuration; report done",
        meta.skill
    );
    let mut state = locked_state(dir)?;
    let limit = limit(&meta);
    ensure!(
        state.count < limit,
        "round limit {limit} reached; report done, the lead will see the failed gates"
    );
    let skills = super::skills::stage_skills(&meta.on_stage, &meta.project)?;
    let mut round = Round {
        log_len: state.log_len,
        failed: Vec::new(),
    };
    let gates = meta
        .gates
        .as_ref()
        .map_or(&[][..], |gates| gates.post.as_slice());
    for gate in gates {
        if !super::gates::run(gate, meta.agent_dir(), dir, state.rounds.len() + 1)? {
            round.failed.push(gate.name.clone());
        }
    }
    let passed = round.failed.is_empty();
    let outcome = match (gates.is_empty(), passed) {
        (true, _) => "none",
        (false, true) => "pass",
        (false, false) => "fail",
    };
    state.rounds.push(round);
    store::write_json(&dir.join("gates.json"), &state.rounds)?;
    append_line(
        &wake::status_log_path(dir),
        &wake::line(
            WakeKind::Stage,
            &format!("{detail}; gates: {outcome}; report: {report}"),
        ),
    )?;
    for skill in skills {
        println!("on_stage_skill: {skill}\n{}", skill.body);
    }
    let next = state.count + 1;
    if passed {
        println!("Apply the post-work skills, then report `done`.");
    } else if next < limit {
        println!("{}", retry(next, limit));
    } else {
        println!("report `done`: the failed gates will be shown to the lead");
    }
    Ok(())
}
