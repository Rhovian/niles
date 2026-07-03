use std::{fs, io::ErrorKind, time::SystemTime};

use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{
    agent_window,
    config::{agents, spec::load_project_config_from, version},
    schema::{self, ArtifactKind},
    store::{self, WorkerLocation, read_state, resolve_run_dir},
    util::{
        absolute_existing_dir, absolute_existing_file, append_line, remove_dir_all_if_exists,
        remove_file_if_exists, render_template, timestamp_id, write_json_pretty,
    },
    wake::{self, WakeKind},
};

const WORKER_BRIEF_TEMPLATE: &str = include_str!("templates/worker_brief.md");
pub(crate) const DEFAULT_PEEK_LINES: usize = 2000;
const FINAL_PANE_CAPTURE_LINES: usize = 2000;
const REPORT_FILE: &str = "report.md";
const FINAL_PANE_FILE: &str = "final-pane.txt";

#[derive(Debug, Serialize, Deserialize)]
struct WorkerMeta {
    id: String,
    agent: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    agent_family: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    task_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    created_at: Option<DateTime<Utc>>,
    project: Utf8PathBuf,
    window: String,
    brief: Utf8PathBuf,
    launch: Utf8PathBuf,
    status: Option<Utf8PathBuf>,
}

enum PaneTarget {
    Worker {
        id: String,
        target: String,
    },
    RunStep {
        run: String,
        index: usize,
        window_name: String,
    },
}

pub fn spawn(
    id: String,
    task_label: Option<String>,
    project: Utf8PathBuf,
    agent: String,
    brief: Option<Utf8PathBuf>,
    task: Vec<String>,
    allow_cli_mismatch: bool,
) -> Result<()> {
    validate_id(&id)?;
    if let Some(label) = &task_label {
        validate_task_label(label)?;
    }
    if brief.is_none() && task.is_empty() {
        bail!("spawn requires either --brief or task text");
    }

    let agent_spec = agents::parse_spec(&agent)?;
    let project = absolute_existing_dir(&project, "project")?;
    let config = load_project_config_from(&project)?;
    let agent_config = agents::config_for(&config.agents, &agent)?;
    version::preflight_agent(
        &agent,
        agent_config,
        agents::InvocationDefaults::Worker,
        allow_cli_mismatch,
    )?;
    if resolve_live_worker_if_exists(&id)?.is_some() {
        bail!("worker id '{id}' already exists");
    }

    let dir = store::workspace_worker_dir(&project, &id)?;
    if dir.exists() {
        archive_worker_dir(&id, &project, &dir, Utc::now())?;
    }
    fs::create_dir_all(&dir).with_context(|| format!("failed to create {dir}"))?;

    let brief_path = match brief {
        Some(path) => absolute_existing_file(&path, "brief")?,
        None => {
            let path = dir.join("brief.md");
            write_brief(
                &path,
                &id,
                task_label.as_deref(),
                &project,
                &agent,
                &task.join(" "),
            )?;
            path
        }
    };

    let launch_path = dir.join("launch.sh");
    let status_path = wake::status_log_path(&dir);
    fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&status_path)
        .with_context(|| format!("failed to create {status_path}"))?;

    store::register_worker_location(&id, &project, &dir)?;

    let window_name = agent_window::worker_window_name(&id);
    let target = match agent_window::spawn_agent_window(
        &window_name,
        &project,
        &agent,
        &project,
        &brief_path,
        &launch_path,
    ) {
        Ok(target) => target,
        Err(err) => {
            if let Err(cleanup_err) = cleanup_failed_spawn(&id, &project, &dir) {
                return Err(err).context(format!(
                    "failed to launch worker {id}; additionally failed to clean up partial worker at {dir}: {cleanup_err}"
                ));
            }
            return Err(err).context(format!(
                "failed to launch worker {id}; cleaned up partial worker at {dir}"
            ));
        }
    };

    let meta = WorkerMeta {
        id: id.clone(),
        agent,
        agent_family: agent_spec.tier().map(|tier| tier.family),
        model: agent_spec.model().map(str::to_owned),
        effort: agent_spec.effort().map(str::to_owned),
        task_label,
        created_at: Some(Utc::now()),
        project,
        window: target.clone(),
        brief: brief_path,
        launch: launch_path,
        status: Some(status_path),
    };
    write_meta(&dir, &meta)?;

    println!("spawned: {id}");
    println!("window: {window_name}");
    println!("agent: {}", meta.agent);
    print_worker_tier(&meta);
    if let Some(label) = &meta.task_label {
        println!("task: {label}");
    }
    println!("brief: {}", meta.brief);
    println!("peek: niles peek {id}");
    println!("report: niles report {id}");
    println!("send: niles send {id} <message>");
    println!("close: niles worker-close {id}");
    if let Some(label) = &meta.task_label {
        println!("close_task: niles worker-close --task {label}");
    }
    println!("workers: niles workers");

    Ok(())
}

fn print_worker_tier(meta: &WorkerMeta) {
    if let Some(family) = &meta.agent_family {
        println!("agent_family: {family}");
    }
    if let Some(model) = &meta.model {
        println!("model: {model}");
    }
    if let Some(effort) = &meta.effort {
        println!("effort: {effort}");
    }
}

struct WorkerCloseOutcome {
    id: String,
    archive_dir: Utf8PathBuf,
    pane_path: Option<Utf8PathBuf>,
    pane_error: Option<String>,
    window_name: String,
    window_error: Option<String>,
}

/// Tear down spawned workers. The tmux window may already be gone, so window
/// close errors are reported but do not strand metadata.
pub fn worker_close(id: Option<String>, task_label: Option<String>, all: bool) -> Result<()> {
    match (id, task_label, all) {
        (Some(id), None, false) => {
            let outcome = close_worker_once(&id)?;
            print_single_close_outcome(&outcome);
            Ok(())
        }
        (None, Some(label), false) => close_workers_by_task(&label),
        (None, None, true) => close_all_workers(),
        _ => bail!("use a worker id, --task <label>, or --all"),
    }
}

fn close_workers_by_task(label: &str) -> Result<()> {
    validate_task_label(label)?;
    let selection = select_worker_ids_by_task(label)?;
    close_worker_group(format!("--task {label}"), selection.ids, selection.failures)
}

fn close_all_workers() -> Result<()> {
    let ids = close_all_worker_ids()?;
    close_worker_group("--all".to_owned(), ids, Vec::new())
}

struct WorkerCloseSelection {
    ids: Vec<String>,
    failures: Vec<(String, String)>,
}

fn close_worker_group(
    selection: String,
    ids: Vec<String>,
    selection_failures: Vec<(String, String)>,
) -> Result<()> {
    println!(
        "workers[{}]{{id,status,archive}}:",
        ids.len() + selection_failures.len()
    );

    let mut failures = Vec::new();
    for (id, err) in selection_failures {
        println!("  {id},failed,-");
        eprintln!("worker {id} close failed: {err}");
        failures.push(id);
    }
    for id in ids {
        match close_worker_once(&id) {
            Ok(outcome) => print_group_close_success(&outcome),
            Err(err) => {
                println!("  {id},failed,-");
                eprintln!("worker {id} close failed: {err:#}");
                failures.push(id);
            }
        }
    }

    if failures.is_empty() {
        Ok(())
    } else {
        bail!(
            "worker-close {selection} failed for {} worker(s): {}",
            failures.len(),
            failures.join(", ")
        )
    }
}

fn select_worker_ids_by_task(label: &str) -> Result<WorkerCloseSelection> {
    let mut ids = Vec::new();
    let mut failures = Vec::new();
    for entry in store::resolve_worker_locations()? {
        let meta_path = meta_path(&entry.location.worker_dir);
        if !meta_path.exists() {
            continue;
        }
        match read_meta_if_exists(&entry.location.worker_dir) {
            Ok(Some(meta)) if meta.task_label.as_deref() == Some(label) => ids.push(entry.id),
            Ok(_) => {}
            Err(err) => failures.push((entry.id, format!("{err:#}"))),
        }
    }
    ids.sort();
    ids.dedup();
    failures.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(WorkerCloseSelection { ids, failures })
}

fn print_single_close_outcome(outcome: &WorkerCloseOutcome) {
    if let Some(path) = &outcome.pane_path {
        println!("pane: {path}");
    }
    if let Some(err) = &outcome.pane_error {
        println!("pane not captured for worker {}: {err}", outcome.id);
    }
    if let Some(err) = &outcome.window_error {
        println!("window {} not closed: {err}", outcome.window_name);
    } else {
        println!("closed window: {}", outcome.window_name);
    }
    println!("archive: {}", outcome.archive_dir);
    println!("closed: {}", outcome.id);
}

fn print_group_close_success(outcome: &WorkerCloseOutcome) {
    println!("  {},closed,{}", outcome.id, outcome.archive_dir);
    if let Some(err) = &outcome.pane_error {
        println!("  {},pane-not-captured,{err}", outcome.id);
    }
    if let Some(err) = &outcome.window_error {
        println!("  {},window-not-closed,{err}", outcome.id);
    }
}

fn close_worker_once(id: &str) -> Result<WorkerCloseOutcome> {
    validate_id(id)?;
    let location = resolve_worker_if_exists(id)?.with_context(|| no_live_worker_message(id))?;
    let worker_dir = location.worker_dir.clone();
    let meta = read_meta_if_exists(&worker_dir)?.with_context(|| no_live_worker_message(id))?;
    let window_name = agent_window::worker_window_name_from_target(id, &meta.window);
    let status_path = meta
        .status
        .as_ref()
        .cloned()
        .unwrap_or_else(|| wake::status_log_path(&worker_dir));
    append_closed_sentinel(&status_path, id)?;

    let (pane_path, pane_error) = match capture_final_pane(&worker_dir, Some(&meta), &window_name) {
        Ok(path) => (path, None),
        Err(err) => (None, Some(err.to_string())),
    };
    let captured_pane = pane_path.is_some();

    let window_error = agent_window::close_window(&window_name)
        .err()
        .map(|err| err.to_string());

    let archive_dir = archive_worker_dir(id, &meta.project, &worker_dir, Utc::now())?;
    let pane_path = captured_pane.then(|| final_pane_path(&archive_dir));
    store::unregister_worker_location(
        id,
        Some(&location),
        Some(meta.project.as_path()),
        Some(&worker_dir),
    )?;
    Ok(WorkerCloseOutcome {
        id: id.to_owned(),
        archive_dir,
        pane_path,
        pane_error,
        window_name,
        window_error,
    })
}

struct LiveWorker {
    id: String,
    location: WorkerLocation,
    meta: WorkerMeta,
}

pub fn workers() -> Result<()> {
    let workers = live_workers()?;
    println!(
        "workers[{}]{{id,agent,task,age,last_status}}:",
        workers.len()
    );

    let now = Utc::now();
    for worker in workers {
        let task = worker.meta.task_label.as_deref().unwrap_or("-");
        let age = worker_age(&worker, now);
        let status = last_status_line(&worker)?;
        println!(
            "  {},{},{},{},{}",
            worker.id,
            display_agent(&worker.meta),
            task,
            age,
            status.unwrap_or_else(|| "-".to_owned())
        );
    }

    Ok(())
}

fn live_workers() -> Result<Vec<LiveWorker>> {
    let mut workers = Vec::new();
    for entry in store::resolve_worker_locations()? {
        let meta_path = meta_path(&entry.location.worker_dir);
        if !meta_path.exists() {
            continue;
        }
        let Some(meta) = read_meta_if_exists(&entry.location.worker_dir)? else {
            continue;
        };
        workers.push(LiveWorker {
            id: entry.id,
            location: entry.location,
            meta,
        });
    }
    workers.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(workers)
}

fn close_all_worker_ids() -> Result<Vec<String>> {
    let mut ids = store::resolve_worker_locations()?
        .into_iter()
        .filter(|entry| meta_path(&entry.location.worker_dir).exists())
        .map(|entry| entry.id)
        .collect::<Vec<_>>();
    ids.sort();
    ids.dedup();
    Ok(ids)
}

fn display_agent(meta: &WorkerMeta) -> &str {
    &meta.agent
}

fn worker_age(worker: &LiveWorker, now: DateTime<Utc>) -> String {
    let started_at = worker
        .meta
        .created_at
        .or_else(|| path_time(&meta_path(&worker.location.worker_dir)))
        .unwrap_or(now);
    let seconds = now.signed_duration_since(started_at).num_seconds().max(0);

    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 60 * 60 {
        format!("{}m", seconds / 60)
    } else if seconds < 60 * 60 * 24 {
        format!("{}h", seconds / (60 * 60))
    } else {
        format!("{}d", seconds / (60 * 60 * 24))
    }
}

fn path_time(path: &Utf8Path) -> Option<DateTime<Utc>> {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .map(system_time_to_utc)
}

fn system_time_to_utc(time: SystemTime) -> DateTime<Utc> {
    DateTime::<Utc>::from(time)
}

fn last_status_line(worker: &LiveWorker) -> Result<Option<String>> {
    let status_path = worker
        .meta
        .status
        .as_ref()
        .cloned()
        .unwrap_or_else(|| wake::status_log_path(&worker.location.worker_dir));
    let body = match fs::read_to_string(&status_path) {
        Ok(body) => body,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).with_context(|| format!("failed to read {status_path}")),
    };
    Ok(body
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .map(str::to_owned))
}

pub fn report(id: String) -> Result<()> {
    validate_id(&id)?;
    if let Some(location) = resolve_live_worker_if_exists(&id)? {
        return print_report(&id, &report_path(&location.worker_dir), None);
    }

    let Some(archive) = latest_archive(&id)? else {
        bail!("no report found for worker '{id}': no live worker or archive found");
    };
    let path = report_path(&archive.archive_dir);
    print_report(&id, &path, Some(&archive.archive_dir))
}

fn print_report(id: &str, path: &Utf8Path, archive_dir: Option<&Utf8Path>) -> Result<()> {
    let body = match fs::read_to_string(&path) {
        Ok(body) => body,
        Err(err) if err.kind() == ErrorKind::NotFound => {
            let final_pane = path
                .parent()
                .map(final_pane_path)
                .unwrap_or_else(|| Utf8PathBuf::from(FINAL_PANE_FILE));
            if final_pane.exists() {
                bail!(
                    "no report found for worker '{id}' at {path}; final pane snapshot is available at {final_pane}"
                );
            }
            bail!(
                "no report found for worker '{id}' at {path}; workers should write substantial deliverables to report.md"
            );
        }
        Err(err) => return Err(err).with_context(|| format!("failed to read {path}")),
    };
    if let Some(archive_dir) = archive_dir {
        eprintln!("serving archived report from {path} (archive: {archive_dir})");
    }
    print!("{body}");
    Ok(())
}

pub fn peek(
    id: Option<String>,
    run: Option<String>,
    index: Option<usize>,
    lines: usize,
) -> Result<()> {
    let target = resolve_peek_target(id, run, index)?;
    print!("{}", target.capture(lines)?);
    Ok(())
}

pub fn send(
    run: Option<String>,
    index: Option<usize>,
    target_and_message: Vec<String>,
) -> Result<()> {
    if target_and_message.is_empty() {
        bail!("send requires a message");
    }

    let (target, message) = resolve_send_target(run, index, target_and_message)?;
    let message = message.join(" ");
    target.send(&message)?;
    println!("sent: {}", target.label());
    Ok(())
}

impl PaneTarget {
    fn capture(&self, lines: usize) -> Result<String> {
        match self {
            PaneTarget::Worker { target, .. } => agent_window::capture_target(target, lines),
            PaneTarget::RunStep { window_name, .. } => {
                agent_window::capture_window(window_name, lines)
            }
        }
    }

    fn send(&self, message: &str) -> Result<()> {
        match self {
            PaneTarget::Worker { target, .. } => agent_window::send_target(target, message),
            PaneTarget::RunStep { window_name, .. } => {
                agent_window::send_window(window_name, message)
            }
        }
    }

    fn label(&self) -> String {
        match self {
            PaneTarget::Worker { id, .. } => id.clone(),
            PaneTarget::RunStep { run, index, .. } => format!("{run} step {index}"),
        }
    }
}

fn resolve_peek_target(
    id: Option<String>,
    run: Option<String>,
    index: Option<usize>,
) -> Result<PaneTarget> {
    let has_step_target = run.is_some() || index.is_some();
    match (id, has_step_target) {
        (Some(_), true) => bail!("use either a worker id or --run <id> --index <N>, not both"),
        (Some(id), false) => worker_target(id),
        (None, true) => run_step_target(run, index),
        (None, false) => bail!("peek requires a worker id or --run <id> --index <N>"),
    }
}

fn resolve_send_target(
    run: Option<String>,
    index: Option<usize>,
    target_and_message: Vec<String>,
) -> Result<(PaneTarget, Vec<String>)> {
    if run.is_some() || index.is_some() {
        return Ok((run_step_target(run, index)?, target_and_message));
    }

    let mut parts = target_and_message.into_iter();
    let id = parts
        .next()
        .context("send requires a worker id or --run <id> --index <N>")?;
    let message = parts.collect::<Vec<_>>();
    if message.is_empty() {
        bail!("send requires a message");
    }
    Ok((worker_target(id)?, message))
}

fn worker_target(id: String) -> Result<PaneTarget> {
    let meta = read_meta(&id)?.meta;
    Ok(PaneTarget::Worker {
        id,
        target: meta.window,
    })
}

fn run_step_target(run: Option<String>, index: Option<usize>) -> Result<PaneTarget> {
    let run = run.context("run-step target requires --run <id>")?;
    let index = index.context("run-step target requires --index <N>")?;
    if index == 0 {
        bail!("step index must be >= 1");
    }

    let run_dir = resolve_run_dir(&run)?;
    let state = read_state(&run_dir)?;
    let run_id = state.id.clone();
    let step = state
        .steps
        .iter()
        .find(|step| step.index == index)
        .with_context(|| format!("step {index} not found in run {run_id}"))?;
    let window_name = step
        .window
        .clone()
        .with_context(|| format!("step {index} in run {run_id} has no recorded window"))?;

    Ok(PaneTarget::RunStep {
        run: run_id,
        index,
        window_name,
    })
}

pub fn status_log_path(id: &str) -> Result<Utf8PathBuf> {
    validate_id(id)?;
    let location = resolve_worker(id)?;
    if let Some(meta) = read_meta_if_exists(&location.worker_dir)?
        && let Some(status) = meta.status
    {
        return Ok(status);
    }
    Ok(wake::status_log_path(&location.worker_dir))
}

fn write_brief(
    path: &Utf8Path,
    id: &str,
    task_label: Option<&str>,
    project: &Utf8Path,
    agent: &str,
    task: &str,
) -> Result<()> {
    let status_path = path
        .parent()
        .map(wake::status_log_path)
        .unwrap_or_else(|| Utf8PathBuf::from("status.log"));
    let report_path = path
        .parent()
        .map(report_path)
        .unwrap_or_else(|| Utf8PathBuf::from(REPORT_FILE));
    let wake_examples = wake::worker_contract_examples(&status_path);
    let body = render_template(
        WORKER_BRIEF_TEMPLATE,
        &[
            ("{id}", id),
            ("{task_label}", task_label.unwrap_or("-")),
            ("{project}", project.as_str()),
            ("{agent}", agent),
            ("{status_path}", status_path.as_str()),
            ("{report_path}", report_path.as_str()),
            ("{task}", task),
            ("{wake_examples}", &wake_examples),
        ],
    );
    fs::write(path, body).with_context(|| format!("failed to write {path}"))
}

fn write_meta(worker_dir: &Utf8Path, meta: &WorkerMeta) -> Result<()> {
    let path = meta_path(worker_dir);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("failed to create {parent}"))?;
    }
    write_json_pretty(&path, meta)
}

struct LoadedWorker {
    meta: WorkerMeta,
}

fn read_meta(id: &str) -> Result<LoadedWorker> {
    validate_id(id)?;
    let location = resolve_worker(id)?;
    let path = meta_path(&location.worker_dir);
    let meta = read_meta_if_exists(&location.worker_dir)?
        .with_context(|| format!("worker metadata missing for '{id}' at {path}"))?;
    Ok(LoadedWorker { meta })
}

fn read_meta_if_exists(worker_dir: &Utf8Path) -> Result<Option<WorkerMeta>> {
    let path = meta_path(worker_dir);
    schema::read_optional_json(&path, ArtifactKind::WorkerMetadata)
}

fn meta_path(worker_dir: &Utf8Path) -> Utf8PathBuf {
    worker_dir.join("meta.json")
}

fn report_path(worker_dir: &Utf8Path) -> Utf8PathBuf {
    worker_dir.join(REPORT_FILE)
}

fn final_pane_path(worker_dir: &Utf8Path) -> Utf8PathBuf {
    worker_dir.join(FINAL_PANE_FILE)
}

fn capture_final_pane(
    worker_dir: &Utf8Path,
    meta: Option<&WorkerMeta>,
    window_name: &str,
) -> Result<Option<Utf8PathBuf>> {
    if !worker_dir.exists() {
        return Ok(None);
    }

    let text = match meta {
        Some(meta) => agent_window::capture_target(&meta.window, FINAL_PANE_CAPTURE_LINES),
        None => agent_window::capture_window(window_name, FINAL_PANE_CAPTURE_LINES),
    }?;
    if text.is_empty() {
        return Ok(None);
    }
    let path = final_pane_path(worker_dir);
    fs::write(&path, text).with_context(|| format!("failed to write {path}"))?;
    Ok(Some(path))
}

fn archive_worker_dir(
    id: &str,
    workspace: &Utf8Path,
    worker_dir: &Utf8Path,
    archived_at: DateTime<Utc>,
) -> Result<Utf8PathBuf> {
    if !worker_dir.exists() {
        return Ok(worker_dir.to_path_buf());
    }
    let archive_root = archive_root(worker_dir)?;
    fs::create_dir_all(&archive_root)
        .with_context(|| format!("failed to create {archive_root}"))?;
    let archive_dir = archive_root.join(format!("{id}-{}", timestamp_id(&archived_at)));
    move_dir(worker_dir, &archive_dir)?;
    store::register_worker_archive(id, workspace, &archive_dir, archived_at)?;
    Ok(archive_dir)
}

fn archive_root(worker_dir: &Utf8Path) -> Result<Utf8PathBuf> {
    let workers_dir = worker_dir
        .parent()
        .with_context(|| format!("worker dir {worker_dir} has no parent"))?;
    Ok(workers_dir.join("archive"))
}

fn move_dir(source: &Utf8Path, destination: &Utf8Path) -> Result<()> {
    match fs::rename(source, destination) {
        Ok(()) => Ok(()),
        Err(err) if err.raw_os_error() == Some(libc::EXDEV) => {
            copy_dir_all(source, destination)?;
            remove_dir_all_if_exists(source)
        }
        Err(err) => Err(err).with_context(|| format!("failed to move {source} to {destination}")),
    }
}

fn copy_dir_all(source: &Utf8Path, destination: &Utf8Path) -> Result<()> {
    fs::create_dir_all(destination).with_context(|| format!("failed to create {destination}"))?;
    for entry in fs::read_dir(source).with_context(|| format!("failed to read {source}"))? {
        let entry = entry.with_context(|| format!("failed to read entry in {source}"))?;
        let path = Utf8PathBuf::from_path_buf(entry.path())
            .map_err(|path| anyhow::anyhow!("path is not UTF-8: {}", path.display()))?;
        let target = destination.join(entry.file_name().to_string_lossy().as_ref());
        let file_type = entry
            .file_type()
            .with_context(|| format!("failed to inspect {path}"))?;
        if file_type.is_dir() {
            copy_dir_all(&path, &target)?;
        } else {
            fs::copy(&path, &target)
                .with_context(|| format!("failed to copy {path} to {target}"))?;
        }
    }
    Ok(())
}

fn cleanup_failed_spawn(id: &str, project: &Utf8Path, dir: &Utf8Path) -> Result<()> {
    store::unregister_worker_location(id, None, Some(project), Some(dir))?;
    remove_file_if_exists(&meta_path(dir))?;
    remove_dir_all_if_exists(dir)
}

fn resolve_worker(id: &str) -> Result<WorkerLocation> {
    resolve_worker_if_exists(id)?.with_context(|| format!("unknown worker id '{id}'"))
}

fn resolve_worker_if_exists(id: &str) -> Result<Option<WorkerLocation>> {
    validate_id(id)?;
    store::resolve_worker_location(id)
}

fn resolve_live_worker_if_exists(id: &str) -> Result<Option<WorkerLocation>> {
    let Some(location) = resolve_worker_if_exists(id)? else {
        return Ok(None);
    };
    Ok(read_meta_if_exists(&location.worker_dir)?
        .is_some()
        .then_some(location))
}

fn no_live_worker_message(id: &str) -> String {
    match latest_archive(id) {
        Ok(Some(archive)) => format!(
            "no live worker '{id}'; latest archive: {}",
            archive.archive_dir
        ),
        Ok(None) => format!("no live worker '{id}'"),
        Err(err) => format!("no live worker '{id}'; failed to inspect archives: {err}"),
    }
}

fn latest_archive(id: &str) -> Result<Option<store::WorkerArchivePointer>> {
    Ok(store::resolve_worker_archives(id)?
        .into_iter()
        .rev()
        .find(|archive| archive.archive_dir.exists()))
}

fn append_closed_sentinel(path: &Utf8Path, id: &str) -> Result<()> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    if !parent.exists() {
        return Ok(());
    }

    append_line(
        path,
        &wake::line(WakeKind::Closed, id),
        |path| format!("failed to open {path} for worker close sentinel"),
        |path| format!("failed to inspect {path} before worker close sentinel"),
        |path| format!("failed to write worker close sentinel to {path}"),
    )
}

fn validate_id(id: &str) -> Result<()> {
    if id.is_empty() {
        bail!("worker id cannot be empty");
    }
    if id == "archive" {
        bail!("worker id 'archive' is reserved for closed worker archives");
    }
    if !id
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        bail!("worker id may only contain ASCII letters, numbers, '-' and '_'");
    }
    Ok(())
}

fn validate_task_label(label: &str) -> Result<()> {
    if label.is_empty() {
        bail!("task label cannot be empty");
    }
    if label == "archive" {
        bail!("task label 'archive' is reserved for closed worker archives");
    }
    if !label
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        bail!("task label may only contain ASCII letters, numbers, '-' and '_'");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn closed_sentinel_starts_on_its_own_line() {
        let dir = std::env::temp_dir().join(format!(
            "niles-worker-sentinel-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = Utf8PathBuf::from_path_buf(dir.join("status.log")).unwrap();
        fs::write(&path, "working: close requested").unwrap();

        append_closed_sentinel(&path, "auth-fix").unwrap();

        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "working: close requested\nclosed: auth-fix\n"
        );

        fs::remove_dir_all(&dir).unwrap();
    }
}
