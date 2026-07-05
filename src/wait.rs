use std::{
    fs,
    io::{ErrorKind, Write},
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{
    schema::{self, ArtifactKind},
    state::{RunState, StepStatus},
    store,
    util::{append_line, read_optional_to_string, timestamp_id},
    wake::{
        WakeKind, is_actionable_wake, is_closed_wake, is_untagged_actionable_wake, mentions_step,
        status_log_path,
    },
    worker,
};

#[cfg(test)]
use crate::wake::mentions_any_step;

const DEFAULT_WAIT_TIMEOUT: Duration = Duration::from_secs(60 * 60);

pub fn wait(
    run: Option<String>,
    worker: Vec<String>,
    task: Option<String>,
    index: Option<usize>,
    interval: f64,
    timeout: Option<f64>,
) -> Result<()> {
    let targets = WaitTargets::resolve(run, worker, task, index)?;
    let prefix_worker_id = targets.prefix_worker_id();
    let timeout_subject = targets.timeout_subject();

    let interval = positive_seconds_duration(interval, "wait interval")?;
    let timeout = match timeout
        .map(|seconds| non_negative_seconds_duration(seconds, "wait timeout"))
        .transpose()?
    {
        Some(timeout) => timeout,
        None => DEFAULT_WAIT_TIMEOUT,
    };

    let wake = wait_for_targets(targets, interval, timeout)?;
    let worker_id = wake.worker_id;
    match wake.result {
        WaitResult::Line(line) => {
            println!(
                "{}",
                format_wake_line(worker_id.as_deref(), line, prefix_worker_id)
            );
            Ok(())
        }
        WaitResult::WorkerClosed { id, line } => {
            println!(
                "{}",
                format_wake_line(worker_id.as_deref(), line, prefix_worker_id)
            );
            bail!("worker '{id}' closed")
        }
        WaitResult::Timeout => {
            bail!("timeout: no actionable wake line appeared in {timeout_subject}")
        }
    }
}

enum WaitTargets {
    Run {
        target: WaitTarget,
        index: Option<usize>,
    },
    Workers(Vec<WaitTarget>),
}

enum WaitTarget {
    Run {
        status: Utf8PathBuf,
        run_dir: Utf8PathBuf,
    },
    Worker {
        id: String,
        status: Utf8PathBuf,
        dir: Utf8PathBuf,
    },
}

enum WaitResult {
    Line(String),
    WorkerClosed { id: String, line: String },
    Timeout,
}

struct WaitEntry {
    target: WaitTarget,
    scanner: WakeScanner,
    guard: Option<WaiterGuard>,
}

struct FiredWake {
    worker_id: Option<String>,
    result: WaitResult,
}

impl WaitTargets {
    fn resolve(
        run: Option<String>,
        worker_ids: Vec<String>,
        task: Option<String>,
        index: Option<usize>,
    ) -> Result<Self> {
        if let Some(0) = index {
            bail!("step index must be >= 1");
        }

        let has_workers = !worker_ids.is_empty();
        match (run, has_workers, task) {
            (Some(run), false, None) => {
                let run_dir = store::resolve_run_dir(&run)?;
                Ok(Self::Run {
                    target: WaitTarget::Run {
                        status: status_log_path(&run_dir),
                        run_dir,
                    },
                    index,
                })
            }
            (None, true, None) => {
                reject_index_for_worker_targets(index)?;
                Ok(Self::Workers(resolve_worker_targets(dedup_worker_ids(
                    worker_ids,
                ))?))
            }
            (None, false, Some(label)) => {
                reject_index_for_worker_targets(index)?;
                worker::validate_task_label(&label)?;
                let selection = worker::select_worker_ids_by_task(&label)?;
                Ok(Self::Workers(resolve_task_selection(
                    &label,
                    selection,
                    resolve_worker_target,
                )?))
            }
            (Some(_), true, None) => bail!("use either a run id or --worker <id>, not both"),
            (Some(_), false, Some(_)) => bail!("use either a run id or --task <label>, not both"),
            (Some(_), true, Some(_)) => bail!("use exactly one of a run id, --worker, or --task"),
            (None, true, Some(_)) => bail!("use either --worker <id> or --task <label>, not both"),
            (None, false, None) => {
                bail!("wait requires a run id, --worker <id>, or --task <label>")
            }
        }
    }

    fn prefix_worker_id(&self) -> bool {
        match self {
            Self::Run { .. } => false,
            Self::Workers(targets) => targets.len() > 1,
        }
    }

    fn timeout_subject(&self) -> String {
        match self {
            Self::Run { target, .. } => target.status().to_string(),
            Self::Workers(targets) if targets.len() == 1 => targets[0].status().to_string(),
            Self::Workers(_) => "requested workers".to_owned(),
        }
    }
}

impl WaitTarget {
    fn status(&self) -> &Utf8Path {
        match self {
            WaitTarget::Run { status, .. } | WaitTarget::Worker { status, .. } => status,
        }
    }

    fn closed_wake_match(&self) -> ClosedWakeMatch {
        match self {
            WaitTarget::Run { .. } => ClosedWakeMatch::StepMentionOnly,
            WaitTarget::Worker { .. } => ClosedWakeMatch::AnyClosed,
        }
    }

    fn attributed_step(&self, index: usize) -> Result<Option<AttributedStep>> {
        let WaitTarget::Run { run_dir, .. } = self else {
            return Ok(None);
        };
        let Some(state) = read_optional_run_state(run_dir)? else {
            return Ok(None);
        };
        let Some(attributed) = uniquely_attributed_step(&state) else {
            return Ok(None);
        };
        Ok((attributed.index == index).then_some(attributed))
    }

    fn closed_if_missing(&self) -> Option<WaitResult> {
        match self {
            WaitTarget::Run { .. } => None,
            WaitTarget::Worker { id, dir, .. } if !dir.exists() => Some(WaitResult::WorkerClosed {
                id: id.clone(),
                line: crate::wake::line(
                    WakeKind::Closed,
                    &format!("worker '{id}' directory removed"),
                ),
            }),
            WaitTarget::Worker { .. } => None,
        }
    }

    fn result_for_line(&self, line: String) -> WaitResult {
        match self {
            WaitTarget::Worker { id, .. } if is_closed_wake(&line) => WaitResult::WorkerClosed {
                id: id.clone(),
                line,
            },
            WaitTarget::Worker { .. } => WaitResult::Line(line),
            WaitTarget::Run { .. } => WaitResult::Line(line),
        }
    }

    fn worker_id(&self) -> Option<&str> {
        match self {
            WaitTarget::Run { .. } => None,
            WaitTarget::Worker { id, .. } => Some(id),
        }
    }
}

fn reject_index_for_worker_targets(index: Option<usize>) -> Result<()> {
    if index.is_some() {
        bail!("--index is only valid with a run target");
    }
    Ok(())
}

fn format_wake_line(worker_id: Option<&str>, line: String, prefix_worker_id: bool) -> String {
    if prefix_worker_id && let Some(id) = worker_id {
        return format!("{id}: {line}");
    }
    line
}

fn dedup_worker_ids(worker_ids: Vec<String>) -> Vec<String> {
    let mut deduped = Vec::new();
    for id in worker_ids {
        if !deduped.iter().any(|existing| existing == &id) {
            deduped.push(id);
        }
    }
    deduped
}

fn resolve_worker_targets(worker_ids: Vec<String>) -> Result<Vec<WaitTarget>> {
    resolve_worker_targets_with(worker_ids, resolve_worker_target)
}

fn resolve_worker_targets_with(
    worker_ids: Vec<String>,
    mut resolve: impl FnMut(String) -> Result<WaitTarget>,
) -> Result<Vec<WaitTarget>> {
    let mut targets = Vec::with_capacity(worker_ids.len());
    for id in worker_ids {
        targets.push(resolve(id)?);
    }
    if targets.is_empty() {
        bail!("wait requires at least one worker target");
    }
    Ok(targets)
}

fn resolve_task_selection(
    label: &str,
    selection: worker::WorkerCloseSelection,
    resolve: impl FnMut(String) -> Result<WaitTarget>,
) -> Result<Vec<WaitTarget>> {
    if !selection.failures.is_empty() {
        let failures = selection
            .failures
            .into_iter()
            .map(|(id, err)| format!("{id}: {err}"))
            .collect::<Vec<_>>()
            .join("; ");
        bail!("failed to select workers with task label {label}: {failures}");
    }
    if selection.ids.is_empty() {
        bail!("no live workers with task label {label}");
    }
    resolve_worker_targets_with(selection.ids, resolve)
}

fn resolve_worker_target(id: String) -> Result<WaitTarget> {
    let status = worker::status_log_path(&id)?;
    let dir = worker_status_dir(&status)?;
    Ok(WaitTarget::Worker { id, status, dir })
}

fn wait_for_targets(
    targets: WaitTargets,
    interval: Duration,
    timeout: Duration,
) -> Result<FiredWake> {
    match targets {
        WaitTargets::Run { target, index } => {
            let worker_id = target.worker_id().map(str::to_owned);
            Ok(FiredWake {
                worker_id,
                result: wait_for_wake(target, index, interval, timeout)?,
            })
        }
        WaitTargets::Workers(mut targets) if targets.len() == 1 => {
            let target = targets.remove(0);
            let worker_id = target.worker_id().map(str::to_owned);
            Ok(FiredWake {
                worker_id,
                result: wait_for_wake(target, None, interval, timeout)?,
            })
        }
        WaitTargets::Workers(targets) => wait_for_first_wake(
            targets
                .into_iter()
                .map(|target| (target, None))
                .collect::<Vec<_>>(),
            interval,
            timeout,
        ),
    }
}

fn wait_for_wake(
    target: WaitTarget,
    index: Option<usize>,
    interval: Duration,
    timeout: Duration,
) -> Result<WaitResult> {
    Ok(wait_for_first_wake(vec![(target, index)], interval, timeout)?.result)
}

fn wait_for_first_wake(
    targets: Vec<(WaitTarget, Option<usize>)>,
    interval: Duration,
    timeout: Duration,
) -> Result<FiredWake> {
    // Missing worker dirs report WorkerClosed before waiter registration can fail on status.waiter.
    for (target, _) in &targets {
        if let Some(result) = target.closed_if_missing() {
            return Ok(FiredWake {
                worker_id: target.worker_id().map(str::to_owned),
                result,
            });
        }
    }

    let mut entries = prepare_wait_entries(targets)?;
    let deadline = Instant::now() + timeout;

    loop {
        if let Some(wake) = poll_wait_entries(&mut entries)? {
            return Ok(wake);
        }

        let now = Instant::now();
        if now >= deadline {
            return Ok(FiredWake {
                worker_id: None,
                result: WaitResult::Timeout,
            });
        }

        thread::sleep(interval.min(deadline - now));
    }
}

fn prepare_wait_entries(targets: Vec<(WaitTarget, Option<usize>)>) -> Result<Vec<WaitEntry>> {
    let mut guards = Vec::with_capacity(targets.len());
    for (target, index) in &targets {
        let guard = if index.is_none() {
            Some(WaiterGuard::register(target.status()).with_context(|| {
                format!(
                    "failed to attach waiter guard for requested target {}",
                    target.status()
                )
            })?)
        } else {
            None
        };
        guards.push(guard);
    }

    let mut entries = Vec::with_capacity(targets.len());
    for ((target, index), guard) in targets.into_iter().zip(guards) {
        let initial_lines = read_lines(target.status())?;
        let scanner = WakeScanner::new(
            &target,
            index,
            initial_lines.len(),
            guard.as_ref().map(|guard| guard.registration.clone()),
        )?;
        entries.push(WaitEntry {
            target,
            scanner,
            guard,
        });
    }
    Ok(entries)
}

fn poll_wait_entries(entries: &mut [WaitEntry]) -> Result<Option<FiredWake>> {
    for entry in entries.iter_mut() {
        if let Some(result) = entry.target.closed_if_missing() {
            return Ok(Some(FiredWake {
                worker_id: entry.target.worker_id().map(str::to_owned),
                result,
            }));
        }
        if let Some(waiter) = &entry.guard
            && let Err(err) = waiter.verify()
        {
            if let Some(result) = entry.target.closed_if_missing() {
                return Ok(Some(FiredWake {
                    worker_id: entry.target.worker_id().map(str::to_owned),
                    result,
                }));
            }
            return Err(err);
        }

        let lines = read_lines(entry.target.status())?;
        if let Some(result) = entry.scanner.select(&entry.target, &lines)? {
            return Ok(Some(FiredWake {
                worker_id: entry.target.worker_id().map(str::to_owned),
                result,
            }));
        }

        if let Some(result) = entry.target.closed_if_missing() {
            return Ok(Some(FiredWake {
                worker_id: entry.target.worker_id().map(str::to_owned),
                result,
            }));
        }
    }
    Ok(None)
}

struct WakeScanner {
    index: Option<usize>,
    cursor: usize,
    waiter: Option<WaiterRegistration>,
}

impl WakeScanner {
    fn new(
        target: &WaitTarget,
        index: Option<usize>,
        initial_line_count: usize,
        waiter: Option<WaiterRegistration>,
    ) -> Result<Self> {
        let cursor = match index {
            // Indexed waits intentionally scan the whole log for tagged wake
            // lines; the cursor only scopes the untagged attribution fallback.
            Some(_) => initial_line_count,
            // Unindexed run and worker waits consume wake lines explicitly via a
            // status.ack cursor next to the status log. A status.waiter guard
            // makes that single-consumer contract visible before this scanner is
            // constructed, so duplicate waiters cannot silently steal wakes.
            None => read_ack_cursor(target.status(), initial_line_count)?,
        };

        Ok(Self {
            index,
            cursor,
            waiter,
        })
    }

    fn select(&mut self, target: &WaitTarget, lines: &[String]) -> Result<Option<WaitResult>> {
        let start = self.direct_scan_start(lines.len());
        if let Some((offset, line)) =
            select_wake_with_offset(&lines[start..], self.index, target.closed_wake_match())
        {
            if self.index.is_none() {
                self.acknowledge(target.status(), start + offset + 1, &line)?;
            }
            return Ok(Some(target.result_for_line(line)));
        }

        if let Some(index) = self.index
            && let Some(line) = select_state_attributed_wake(target, lines, self.cursor, index)?
        {
            return Ok(Some(target.result_for_line(line)));
        }

        self.cursor = lines.len();
        Ok(None)
    }

    fn direct_scan_start(&self, line_count: usize) -> usize {
        if self.index.is_some() {
            0
        } else {
            self.cursor.min(line_count)
        }
    }

    fn acknowledge(&mut self, status: &Utf8Path, cursor: usize, line: &str) -> Result<()> {
        if let Some(waiter) = &self.waiter {
            log_ack_consumption(status, cursor, line, waiter)?;
        }
        write_ack_cursor(status, cursor)?;
        self.cursor = cursor;
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct WaiterRegistration {
    pid: u32,
    started_at: DateTime<Utc>,
    token: String,
}

struct WaiterGuard {
    path: Utf8PathBuf,
    registration: WaiterRegistration,
}

impl WaiterGuard {
    fn register(status: &Utf8Path) -> Result<Self> {
        let path = waiter_path(status);
        let started_at = Utc::now();
        let pid = std::process::id();
        let registration = WaiterRegistration {
            pid,
            started_at,
            token: format!("{pid}-{}", timestamp_id(&started_at)),
        };
        let body = format!("{}\n", serde_json::to_string_pretty(&registration)?);

        for retry in 0..=1 {
            match Self::try_create(&path, registration.clone(), &body)? {
                Some(guard) => return Ok(guard),
                None => {
                    resolve_existing_waiter(status, &path)?;
                    if retry == 1 {
                        bail!(
                            "waiter registration changed while attaching to {status}; retry wait"
                        );
                    }
                }
            }
        }

        unreachable!()
    }

    fn try_create(
        path: &Utf8Path,
        registration: WaiterRegistration,
        body: &str,
    ) -> Result<Option<Self>> {
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
        {
            Ok(mut file) => {
                if let Err(err) = file
                    .write_all(body.as_bytes())
                    .with_context(|| format!("failed to write {path}"))
                {
                    let _ = fs::remove_file(path);
                    return Err(err);
                }
                Ok(Some(Self {
                    path: path.to_path_buf(),
                    registration,
                }))
            }
            Err(err) if err.kind() == ErrorKind::AlreadyExists => Ok(None),
            Err(err) => Err(err).with_context(|| format!("failed to create {path}")),
        }
    }

    fn verify(&self) -> Result<()> {
        let registration = read_optional_json::<WaiterRegistration>(
            &self.path,
            |path| format!("failed to read {path}"),
            |path| format!("failed to parse {path}"),
        )
        .with_context(|| {
            format!(
                "waiter registration was removed/replaced while waiting: {}",
                self.path
            )
        })?;

        if registration
            .as_ref()
            .is_some_and(|registration| registration.token == self.registration.token)
        {
            return Ok(());
        }

        bail!(
            "waiter registration was removed/replaced while waiting: {}",
            self.path
        )
    }
}

impl Drop for WaiterGuard {
    fn drop(&mut self) {
        let Ok(Some(registration)) = read_optional_json::<WaiterRegistration>(
            &self.path,
            |_| String::new(),
            |_| String::new(),
        ) else {
            return;
        };

        if registration.token == self.registration.token {
            let _ = fs::remove_file(&self.path);
        }
    }
}

// The waiter guard is ephemeral wake-protocol state (like status.ack), not a
// durable stamped artifact class, so it is read with a plain JSON reader
// rather than schema::read_optional_json.
fn read_optional_json<T>(
    path: &Utf8Path,
    read_context: impl FnOnce(&Utf8Path) -> String,
    parse_context: impl FnOnce(&Utf8Path) -> String,
) -> Result<Option<T>>
where
    T: serde::de::DeserializeOwned,
{
    let Some(body) = read_optional_to_string(path, read_context)? else {
        return Ok(None);
    };

    serde_json::from_str(&body)
        .map(Some)
        .with_context(|| parse_context(path))
}

fn resolve_existing_waiter(status: &Utf8Path, path: &Utf8Path) -> Result<()> {
    match read_optional_json::<WaiterRegistration>(
        path,
        |path| format!("failed to read {path}"),
        |path| format!("failed to parse {path}"),
    ) {
        Ok(Some(waiter)) if process_is_running(waiter.pid)? => {
            bail!(
                "another unindexed wait is already attached to {status} (pid {}, since {}; pid {} is running); \
                 active waiter registration: {path}",
                waiter.pid,
                waiter.started_at.to_rfc3339(),
                waiter.pid
            )
        }
        Ok(Some(waiter)) => {
            log_stale_waiter_reclaimed(status, &waiter)?;
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(err) if err.kind() == ErrorKind::NotFound => {}
                Err(err) => {
                    return Err(err).with_context(|| {
                        format!("failed to remove stale waiter registration {path}")
                    });
                }
            }
            Ok(())
        }
        Ok(None) => Ok(()),
        Err(err) => bail!(
            "another unindexed wait appears to be attached to {status}, but {path} could not be read: {err}; \
             remove that file if the waiter is stale"
        ),
    }
}

fn worker_status_dir(status: &Utf8Path) -> Result<Utf8PathBuf> {
    status
        .parent()
        .map(Utf8Path::to_path_buf)
        .with_context(|| format!("worker status path has no parent directory: {status}"))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ClosedWakeMatch {
    AnyClosed,
    StepMentionOnly,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct AttributedStep {
    index: usize,
    started_at: DateTime<Utc>,
}

#[cfg(test)]
fn select_wake(
    lines: &[String],
    index: Option<usize>,
    closed_match: ClosedWakeMatch,
) -> Option<String> {
    select_wake_with_offset(lines, index, closed_match).map(|(_, line)| line)
}

fn select_wake_with_offset(
    lines: &[String],
    index: Option<usize>,
    closed_match: ClosedWakeMatch,
) -> Option<(usize, String)> {
    for (offset, line) in lines.iter().enumerate() {
        if !is_actionable_wake(line) {
            continue;
        }

        match index {
            Some(index) if matches_indexed_wake(line, index, closed_match) => {
                return Some((offset, line.clone()));
            }
            Some(_) => {}
            None => return Some((offset, line.clone())),
        }
    }

    None
}

fn select_state_attributed_wake(
    target: &WaitTarget,
    lines: &[String],
    cursor: usize,
    index: usize,
) -> Result<Option<String>> {
    let Some(attributed) = target.attributed_step(index)? else {
        return Ok(None);
    };
    if !status_may_contain_post_launch_lines(target.status(), attributed.started_at)? {
        return Ok(None);
    }

    let start = cursor.min(lines.len());
    Ok(select_untagged_wake(&lines[start..], WakeScanOrder::First)
        .or_else(|| select_untagged_wake(lines, WakeScanOrder::Last)))
}

#[derive(Clone, Copy)]
enum WakeScanOrder {
    First,
    Last,
}

fn select_untagged_wake(lines: &[String], order: WakeScanOrder) -> Option<String> {
    match order {
        WakeScanOrder::First => lines.iter().find(|line| is_untagged_actionable_wake(line)),
        WakeScanOrder::Last => lines
            .iter()
            .rev()
            .find(|line| is_untagged_actionable_wake(line)),
    }
    .cloned()
}

fn matches_indexed_wake(line: &str, index: usize, closed_match: ClosedWakeMatch) -> bool {
    mentions_step(line, index)
        || (closed_match == ClosedWakeMatch::AnyClosed && is_closed_wake(line))
}

fn read_optional_run_state(run_dir: &Utf8Path) -> Result<Option<RunState>> {
    let path = store::state_path(run_dir);
    schema::read_optional_json(&path, ArtifactKind::RunState)
}

fn uniquely_attributed_step(state: &RunState) -> Option<AttributedStep> {
    let mut running = state
        .steps
        .iter()
        .filter(|step| matches!(step.status, StepStatus::Running));
    if let Some(step) = running.next() {
        if running.next().is_some() {
            return None;
        }
        return step.started_at.map(|started_at| AttributedStep {
            index: step.index,
            started_at,
        });
    }

    let mut launched = state
        .steps
        .iter()
        .filter_map(|step| step.started_at.map(|started_at| (step.index, started_at)))
        .collect::<Vec<_>>();
    launched.sort_by_key(|(_, started_at)| *started_at);
    let (index, started_at) = launched.pop()?;
    if launched
        .last()
        .is_some_and(|(_, prior)| *prior == started_at)
    {
        return None;
    }

    Some(AttributedStep { index, started_at })
}

fn status_may_contain_post_launch_lines(
    status: &Utf8Path,
    launched_at: DateTime<Utc>,
) -> Result<bool> {
    let metadata = match fs::metadata(status) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(err).with_context(|| format!("failed to stat {status}")),
    };
    let modified = metadata
        .modified()
        .with_context(|| format!("failed to read modification time for {status}"))?;
    Ok(DateTime::<Utc>::from(modified) >= launched_at)
}

fn read_lines(status: &Utf8Path) -> Result<Vec<String>> {
    Ok(
        read_optional_to_string(status, |status| format!("failed to read {status}"))?
            .map(|body| body.lines().map(str::to_owned).collect())
            .unwrap_or_default(),
    )
}

fn ack_path(status: &Utf8Path) -> Utf8PathBuf {
    status.with_extension("ack")
}

fn ack_log_path(status: &Utf8Path) -> Utf8PathBuf {
    status.with_extension("ack.log")
}

fn waiter_path(status: &Utf8Path) -> Utf8PathBuf {
    status.with_extension("waiter")
}

fn read_ack_cursor(status: &Utf8Path, line_count: usize) -> Result<usize> {
    let path = ack_path(status);
    let Some(body) = read_optional_to_string(&path, |path| format!("failed to read {path}"))?
    else {
        return Ok(0);
    };

    Ok(body.trim().parse::<usize>().unwrap_or(0).min(line_count))
}

fn write_ack_cursor(status: &Utf8Path, cursor: usize) -> Result<()> {
    let path = ack_path(status);
    fs::write(&path, format!("{cursor}\n")).with_context(|| format!("failed to write {path}"))
}

fn log_ack_consumption(
    status: &Utf8Path,
    cursor: usize,
    line: &str,
    waiter: &WaiterRegistration,
) -> Result<()> {
    let path = ack_log_path(status);
    let record = serde_json::json!({
        "event": "wake-consumed",
        "cursor": cursor,
        "pid": waiter.pid,
        "started_at": waiter.started_at.to_rfc3339(),
        "token": &waiter.token,
        "line": line,
    });
    let line = serde_json::to_string(&record)?;
    append_line(
        &path,
        &line,
        |path| format!("failed to open {path}"),
        |path| format!("failed to inspect {path}"),
        |path| format!("failed to write {path}"),
    )
}

fn log_stale_waiter_reclaimed(status: &Utf8Path, waiter: &WaiterRegistration) -> Result<()> {
    let path = ack_log_path(status);
    let record = serde_json::json!({
        "event": "stale-waiter-reclaimed",
        "pid": waiter.pid,
        "started_at": waiter.started_at.to_rfc3339(),
        "token": &waiter.token,
        "reclaimed_at": Utc::now().to_rfc3339(),
    });
    let line = serde_json::to_string(&record)?;
    append_line(
        &path,
        &line,
        |path| format!("failed to open {path}"),
        |path| format!("failed to inspect {path}"),
        |path| format!("failed to write {path}"),
    )
}

#[cfg(unix)]
fn process_is_running(pid: u32) -> Result<bool> {
    let pid = libc::pid_t::try_from(pid)
        .with_context(|| format!("waiter pid {pid} does not fit platform pid_t"))?;
    let result = unsafe { libc::kill(pid, 0) };
    if result == 0 {
        return Ok(true);
    }

    let err = std::io::Error::last_os_error();
    match err.raw_os_error() {
        Some(code) if code == libc::ESRCH => Ok(false),
        Some(code) if code == libc::EPERM => Ok(true),
        _ => Err(err).with_context(|| format!("failed to check whether pid {pid} is running")),
    }
}

#[cfg(not(unix))]
fn process_is_running(_pid: u32) -> Result<bool> {
    Ok(true)
}

fn positive_seconds_duration(seconds: f64, label: &str) -> Result<Duration> {
    if !seconds.is_finite() || seconds <= 0.0 {
        bail!("{label} must be a finite positive number");
    }
    Ok(Duration::from_secs_f64(seconds))
}

fn non_negative_seconds_duration(seconds: f64, label: &str) -> Result<Duration> {
    if !seconds.is_finite() || seconds < 0.0 {
        bail!("{label} must be a finite non-negative number");
    }
    if seconds == 0.0 {
        Ok(Duration::ZERO)
    } else {
        Ok(Duration::from_secs_f64(seconds))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{RunStatus, StepKind, StepRecord};
    use std::{
        env,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn selects_indexed_wake_before_earlier_generic_wake() {
        let lines = vec![
            "done: generic wake".to_owned(),
            "done: step 2 finished".to_owned(),
        ];

        assert_eq!(
            select_wake(&lines, Some(2), ClosedWakeMatch::AnyClosed).as_deref(),
            Some("done: step 2 finished")
        );
    }

    #[test]
    fn indexed_wake_ignores_generic_and_other_step_wakes() {
        let lines = vec![
            "done: generic wake".to_owned(),
            "failed: step 1 failed".to_owned(),
        ];

        assert_eq!(
            select_wake(&lines, Some(2), ClosedWakeMatch::AnyClosed),
            None
        );
    }

    #[test]
    fn indexed_wake_keeps_closed_terminal_backstop() {
        let lines = vec![
            "done: generic wake".to_owned(),
            "closed: auth-fix".to_owned(),
        ];

        assert_eq!(
            select_wake(&lines, Some(2), ClosedWakeMatch::AnyClosed).as_deref(),
            Some("closed: auth-fix")
        );
    }

    #[test]
    fn indexed_run_wake_requires_closed_line_to_mention_step() {
        let lines = vec!["closed: step 1".to_owned()];

        assert_eq!(
            select_wake(&lines, Some(2), ClosedWakeMatch::StepMentionOnly),
            None
        );
        assert_eq!(
            select_wake(&lines, Some(1), ClosedWakeMatch::StepMentionOnly).as_deref(),
            Some("closed: step 1")
        );
    }

    #[test]
    fn step_matching_does_not_confuse_prefixes() {
        assert!(!mentions_step("done: step 10 finished", 1));
        assert!(mentions_step("done: step 1 finished", 1));
        assert!(mentions_any_step("done: step 10 finished"));
        assert!(!mentions_any_step("done: CONSENSUS - finished"));
    }

    #[test]
    fn closed_is_actionable_and_terminal() {
        assert!(is_actionable_wake("closed: auth-fix"));
        assert!(is_closed_wake("closed: auth-fix"));

        let lines = vec![
            "working: teardown".to_owned(),
            "closed: auth-fix".to_owned(),
        ];
        assert_eq!(
            select_wake(&lines, None, ClosedWakeMatch::AnyClosed).as_deref(),
            Some("closed: auth-fix")
        );
    }

    #[test]
    fn attributed_wake_selects_new_untagged_lines_before_existing_lines() {
        let lines = vec![
            "done: old generic".to_owned(),
            "done: CONSENSUS - current".to_owned(),
        ];

        assert_eq!(
            select_untagged_wake(&lines[1..], WakeScanOrder::First).as_deref(),
            Some("done: CONSENSUS - current")
        );
        assert_eq!(
            select_untagged_wake(&lines, WakeScanOrder::Last).as_deref(),
            Some("done: CONSENSUS - current")
        );
    }

    #[test]
    fn attributed_wake_ignores_other_step_tagged_lines() {
        let lines = vec!["done: step 1 finished".to_owned()];

        assert_eq!(select_untagged_wake(&lines, WakeScanOrder::First), None);
    }

    #[test]
    fn tie_break_returns_first_in_target_order() {
        let root = TestTempDir::new("tie-break");
        let first = test_worker_target(root.path(), "second", "done: second worker\n");
        let second = test_worker_target(root.path(), "first", "done: first worker\n");
        let wake = wait_for_first_wake(
            vec![(first, None), (second, None)],
            Duration::from_millis(1),
            Duration::ZERO,
        )
        .unwrap();

        assert_eq!(wake.worker_id.as_deref(), Some("second"));
        assert_wait_line(wake.result, "done: second worker");
    }

    #[test]
    fn only_firing_targets_cursor_advances() {
        let root = TestTempDir::new("cursor-isolation");
        let first = test_worker_target(root.path(), "first", "done: first\n");
        let second = test_worker_target(root.path(), "second", "done: second\n");
        let mut entries = prepare_wait_entries(vec![(first, None), (second, None)]).unwrap();

        let wake = poll_wait_entries(&mut entries).unwrap().unwrap();

        assert_eq!(wake.worker_id.as_deref(), Some("first"));
        assert_eq!(entries[0].scanner.cursor, 1);
        assert_eq!(entries[1].scanner.cursor, 0);
    }

    #[test]
    fn single_element_set_matches_single_wait() {
        let root = TestTempDir::new("single-element");
        let target = test_worker_target(root.path(), "only", "done: single\n");
        let status = target.status().to_path_buf();
        let wake = wait_for_targets(
            WaitTargets::Workers(vec![target]),
            Duration::from_millis(1),
            Duration::ZERO,
        )
        .unwrap();

        let WaitResult::Line(line) = wake.result else {
            panic!("expected line result");
        };
        assert_eq!(
            format_wake_line(wake.worker_id.as_deref(), line, false),
            "done: single"
        );
        assert_eq!(fs::read_to_string(ack_path(&status)).unwrap(), "1\n");
        assert!(!waiter_path(&status).exists());
    }

    #[test]
    fn acquire_all_release_all_on_conflict() {
        let root = TestTempDir::new("guard-conflict");
        let first = test_worker_target(root.path(), "first", "working: first\n");
        let first_waiter = waiter_path(first.status());
        let second = test_worker_target(root.path(), "second", "working: second\n");
        let second_waiter = waiter_path(second.status());
        write_waiter_registration(second.status(), std::process::id(), "live-token");
        let third = test_worker_target(root.path(), "third", "working: third\n");
        let third_waiter = waiter_path(third.status());

        let err = wait_for_first_wake(
            vec![(first, None), (second, None), (third, None)],
            Duration::from_millis(1),
            Duration::ZERO,
        )
        .err()
        .unwrap();

        assert!(format!("{err:#}").contains("another unindexed wait is already attached"));
        assert!(!first_waiter.exists());
        assert!(second_waiter.exists());
        assert!(!third_waiter.exists());
    }

    #[test]
    fn release_all_on_fire() {
        let root = TestTempDir::new("release-on-fire");
        let first = test_worker_target(root.path(), "first", "done: fire\n");
        let first_waiter = waiter_path(first.status());
        let second = test_worker_target(root.path(), "second", "working: waiting\n");
        let second_waiter = waiter_path(second.status());

        let wake = wait_for_first_wake(
            vec![(first, None), (second, None)],
            Duration::from_millis(1),
            Duration::ZERO,
        )
        .unwrap();

        assert_eq!(wake.worker_id.as_deref(), Some("first"));
        assert!(!first_waiter.exists());
        assert!(!second_waiter.exists());
    }

    #[test]
    fn ack_only_the_firing_worker() {
        let root = TestTempDir::new("ack-isolation");
        let first = test_worker_target(root.path(), "first", "working: first\n");
        let first_ack = ack_path(first.status());
        let first_ack_log = ack_log_path(first.status());
        let second = test_worker_target(root.path(), "second", "done: second\n");
        let second_ack = ack_path(second.status());
        let second_ack_log = ack_log_path(second.status());
        let third = test_worker_target(root.path(), "third", "done: third\n");
        let third_ack = ack_path(third.status());
        let third_ack_log = ack_log_path(third.status());

        let wake = wait_for_first_wake(
            vec![(first, None), (second, None), (third, None)],
            Duration::from_millis(1),
            Duration::ZERO,
        )
        .unwrap();

        assert_eq!(wake.worker_id.as_deref(), Some("second"));
        assert!(!first_ack.exists());
        assert!(!first_ack_log.exists());
        assert_eq!(fs::read_to_string(second_ack).unwrap(), "1\n");
        assert!(
            fs::read_to_string(second_ack_log)
                .unwrap()
                .contains("done: second")
        );
        assert!(!third_ack.exists());
        assert!(!third_ack_log.exists());
    }

    #[test]
    fn stale_guard_reclaimed_per_target() {
        let root = TestTempDir::new("stale-guard");
        let first = test_worker_target(root.path(), "first", "working: first\n");
        write_waiter_registration(first.status(), i32::MAX as u32, "stale-token");
        let first_waiter = waiter_path(first.status());
        let first_ack_log = ack_log_path(first.status());
        let second = test_worker_target(root.path(), "second", "done: second\n");

        let wake = wait_for_first_wake(
            vec![(first, None), (second, None)],
            Duration::from_millis(1),
            Duration::ZERO,
        )
        .unwrap();

        assert_eq!(wake.worker_id.as_deref(), Some("second"));
        assert!(!first_waiter.exists());
        let ack_log = fs::read_to_string(first_ack_log).unwrap();
        assert!(ack_log.contains(r#""event":"stale-waiter-reclaimed""#));
        assert!(ack_log.contains(r#""token":"stale-token""#));
    }

    #[test]
    fn task_resolution_selects_labelled_live_workers() {
        let root = TestTempDir::new("task-resolution");
        let selection = worker::WorkerCloseSelection {
            ids: vec!["alpha".to_owned(), "beta".to_owned()],
            failures: Vec::new(),
        };
        let targets = resolve_task_selection("issue-48", selection, |id| {
            Ok(test_worker_target(root.path(), &id, "working: selected\n"))
        })
        .unwrap();
        let ids = targets
            .iter()
            .map(|target| target.worker_id().unwrap().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["alpha", "beta"]);

        let empty = worker::WorkerCloseSelection {
            ids: Vec::new(),
            failures: Vec::new(),
        };
        let err = resolve_task_selection("missing", empty, |id| {
            Ok(test_worker_target(root.path(), &id, "working: selected\n"))
        })
        .err()
        .unwrap();
        assert!(format!("{err:#}").contains("no live workers with task label missing"));
    }

    #[test]
    fn worker_targets_reject_index_at_boundary() {
        let err = WaitTargets::resolve(None, vec!["known".to_owned()], None, Some(1))
            .err()
            .unwrap();
        assert!(format!("{err:#}").contains("--index is only valid with a run target"));

        let err = WaitTargets::resolve(None, Vec::new(), Some("issue-48".to_owned()), Some(1))
            .err()
            .unwrap();
        assert!(format!("{err:#}").contains("--index is only valid with a run target"));
    }

    #[test]
    fn unknown_worker_id_aborts_batch_resolution() {
        let root = TestTempDir::new("unknown-before-guard");
        let known = test_worker_target(root.path(), "known", "working: known\n");
        let mut known = Some(known);

        let err =
            resolve_worker_targets_with(vec!["known".to_owned(), "missing".to_owned()], |id| {
                match id.as_str() {
                    "known" => Ok(known.take().unwrap()),
                    "missing" => bail!("unknown worker id 'missing'"),
                    other => bail!("unexpected worker id '{other}'"),
                }
            })
            .err()
            .unwrap();

        assert!(format!("{err:#}").contains("unknown worker id 'missing'"));
    }

    #[test]
    fn state_attribution_uses_unique_running_step() {
        let started = "2026-07-02T00:00:00Z".parse().unwrap();
        let state = test_state(vec![
            test_step(1, StepStatus::Completed, Some(started)),
            test_step(2, StepStatus::Running, Some(started)),
        ]);

        assert_eq!(
            uniquely_attributed_step(&state),
            Some(AttributedStep {
                index: 2,
                started_at: started
            })
        );
    }

    #[test]
    fn state_attribution_rejects_multiple_running_steps() {
        let started = "2026-07-02T00:00:00Z".parse().unwrap();
        let state = test_state(vec![
            test_step(1, StepStatus::Running, Some(started)),
            test_step(2, StepStatus::Running, Some(started)),
        ]);

        assert_eq!(uniquely_attributed_step(&state), None);
    }

    #[test]
    fn state_attribution_uses_most_recently_launched_step_when_none_running() {
        let earlier = "2026-07-02T00:00:00Z".parse().unwrap();
        let later = "2026-07-02T00:01:00Z".parse().unwrap();
        let state = test_state(vec![
            test_step(1, StepStatus::Completed, Some(earlier)),
            test_step(2, StepStatus::Completed, Some(later)),
        ]);

        assert_eq!(
            uniquely_attributed_step(&state),
            Some(AttributedStep {
                index: 2,
                started_at: later
            })
        );
    }

    fn test_state(steps: Vec<StepRecord>) -> RunState {
        let now = "2026-07-02T00:00:00Z".parse().unwrap();
        RunState {
            id: "test-run".to_owned(),
            goal: "test".to_owned(),
            workspace: None,
            config_root: None,
            task_file: None,
            created_at: now,
            updated_at: now,
            status: RunStatus::Running,
            steps,
        }
    }

    fn test_step(
        index: usize,
        status: StepStatus,
        started_at: Option<DateTime<Utc>>,
    ) -> StepRecord {
        StepRecord {
            index,
            role: None,
            kind: StepKind::Agent,
            label: format!("step-{index}"),
            agent_family: None,
            model: None,
            effort: None,
            status,
            started_at,
            finished_at: None,
            exit_code: None,
            stdout: None,
            stderr: None,
            diff: None,
            context: None,
            window: None,
        }
    }

    fn assert_wait_line(result: WaitResult, expected: &str) {
        let WaitResult::Line(line) = result else {
            panic!("expected line result");
        };
        assert_eq!(line, expected);
    }

    fn test_worker_target(root: &Utf8Path, id: &str, status_body: &str) -> WaitTarget {
        let dir = root.join(id);
        fs::create_dir_all(&dir).unwrap();
        let status = dir.join("status.log");
        fs::write(&status, status_body).unwrap();
        WaitTarget::Worker {
            id: id.to_owned(),
            status,
            dir,
        }
    }

    fn write_waiter_registration(status: &Utf8Path, pid: u32, token: &str) {
        fs::write(
            waiter_path(status),
            format!(
                r#"{{
  "pid": {pid},
  "started_at": "2000-01-01T00:00:00Z",
  "token": "{token}"
}}
"#
            ),
        )
        .unwrap();
    }

    struct TestTempDir {
        path: Utf8PathBuf,
    }

    impl TestTempDir {
        fn new(name: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path =
                env::temp_dir().join(format!("niles-wait-{name}-{}-{nanos}", std::process::id()));
            fs::create_dir_all(&path).unwrap();
            Self {
                path: Utf8PathBuf::from_path_buf(path).unwrap(),
            }
        }

        fn path(&self) -> &Utf8Path {
            &self.path
        }
    }

    impl Drop for TestTempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
