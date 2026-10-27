use crate::workspace_manifest::setup::SetupSelection;
use std::{fs, os::unix::fs::OpenOptionsExt};

use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use chrono::Utc;

use crate::{
    agent_window, agents,
    config::spec::load_project_config_from,
    session,
    skills::{self, SkillName},
    store,
    tmux::{self, WindowTarget},
    util::{
        append_line, current_dir_utf8, find_on_path, remove_dir_all_if_exists, render_template,
    },
    wake, watch, workspace_manifest,
};

use super::{
    archive::archive_worker_dir,
    meta::{WorkerMeta, report_path, write_meta},
    resolve::resolve_live_worker_if_exists,
    snapshot::{WorkerSnapshot, status_log, status_log_len},
    validation::{validate_id, validate_task_label},
    worktree::{self, SpawnTree},
};

const BOOTSTRAP: &str = r#"# Niles {skill} brief
You are niles worker `{id}`. Report with `niles agent status --help`.

id: {id}
task_label: {task_label}
project: {project}
report_file: {report_path}{from_report}

## Task

{task}
"#;

const UNLABELED_TASK_LABEL: &str = "-";

#[derive(Debug, clap::Args)]
pub struct Assignment {
    /// Primary skill to launch from the workspace bindings.
    #[arg(long)]
    pub skill: SkillName,
    /// Add a skill to the discovery menu (repeatable).
    #[arg(long)]
    menu: Vec<skills::reference::MenuItem>,
    /// Inject an additional skill into the system prompt (repeatable).
    #[arg(long = "with")]
    with: Vec<SkillName>,
    /// Manifest tier whose primary to launch (standard by default).
    #[arg(long, value_enum, default_value_t = agents::roster::Tier::Standard, conflicts_with = "agent")]
    pub tier: agents::roster::Tier,
    /// Build on the report of this completed worker.
    #[arg(long, value_name = "ID", conflicts_with = "mechanical")]
    from: Option<String>,
    /// Optionally record mechanical work in the lead session log.
    #[arg(long, value_name = "REASON")]
    mechanical: Option<String>,
}

pub struct SpawnTask {
    pub label: Option<String>,
    pub text: String,
}

pub fn spawn(
    id: String,
    assignment: Assignment,
    agent: Option<String>,
    task: SpawnTask,
    checkin: Option<String>,
    tree: Option<SpawnTree>,
    selection: Option<&SetupSelection>,
) -> Result<()> {
    let SpawnTask {
        label: task_label,
        text: task,
    } = task;
    let skill = assignment.skill.clone();
    let tier = assignment.tier;
    validate_id(&id)?;
    if let Some(label) = &task_label {
        validate_task_label(label)?;
    }
    let project = current_dir_utf8()?;
    let basis = assignment.resolve(&project, resolve_live_worker_if_exists)?;
    let config = load_project_config_from(&project)?;
    let manifest_path = workspace_manifest::manifest_path(&project, selection)?;
    let manifest = workspace_manifest::load(&project, selection).with_context(|| {
        format!(
            "cannot resolve agent for skill '{}' from {manifest_path}",
            skill.as_str()
        )
    })?;
    let manifest = manifest.with_context(|| format!("cannot resolve skill '{skill}': manifest {manifest_path} does not exist. Run spawn from the lead's workspace and pass --tree <path> or --worktree <branch> to place the worker elsewhere"))?;
    let agent = resolve_agent(&manifest_path, &manifest, &skill, tier, agent, &config)?;
    // Resolved with the agent, before any worker state is written: a typo in `--checkin` or in the
    // manifest's check-in keys is the lead's, and it must not leave a launched worker behind it.
    let cadence = watch::resolve_cadence(checkin.as_deref(), Some(&manifest), &manifest_path)?;
    // The one launch decision, resolved before any worker state is written: unknown agent names
    // and models the family does not offer are rejected here, by the same resolver the lead uses.
    let mut invocation =
        agents::invocation(&agent, agents::InvocationDefaults::Worker, &config.models)?;
    let primary_skill = skills::resolve(&skill, &project, &skills::global_dir()?)?;
    let binding = manifest.binding(&skill)?;
    let mut resolved = super::skills::prepare(
        &primary_skill,
        binding,
        &assignment.menu,
        &assignment.with,
        assignment.from.is_some(),
        &project,
    )?;
    if !binding.on_stage.is_empty() || binding.gates.is_some() {
        resolved
            .prompt
            .push_str(include_str!("../../skills/stage_note.md"));
    }
    invocation.pass_primary(&resolved.prompt)?;
    if find_on_path(invocation.binary).is_none() {
        bail!("agent binary {} not found on PATH", invocation.binary);
    }
    let dir = store::workers_dir(&project)?.join(&id);
    let session_link = agents::session_link(invocation.spec.family(), &id, &dir);
    if let Some(link) = &session_link {
        invocation.args.extend(link.args());
    }
    if resolve_live_worker_if_exists(&id)?.is_some() {
        bail!("worker id '{id}' already exists");
    }
    // Resolved before any worker state is written: a spawn that cannot place a window should
    // leave no half-built worker directory behind.
    let session = tmux::current_session()?;

    let tree = match tree {
        None => None,
        Some(SpawnTree::Path(path)) => {
            let canonical = path
                .canonicalize_utf8()
                .with_context(|| format!("failed to resolve tree {path}"))?;
            if !canonical.is_dir() {
                bail!("tree is not a directory: {canonical}");
            }
            Some(canonical)
        }
        Some(SpawnTree::Worktree { branch, base }) => {
            let config = manifest.worktree.as_ref();
            let path = worktree::create_or_join(&project, &branch, base.as_deref(), config)?;
            if let Some(config) = config {
                invocation.env.extend(
                    config
                        .env
                        .iter()
                        .map(|(key, value)| (key.clone(), value.clone())),
                );
            }
            Some(path)
        }
    };
    invocation
        .env
        .push(("NILES_AGENT_DIR".to_owned(), dir.to_string()));
    let agent_spec = &invocation.spec;
    let agent_dir = tree.as_deref().map_or(project.as_path(), |path| path);

    let setup = manifest.source.as_ref();
    if let Some(setup) = setup {
        println!("setup: {setup}");
    }
    println!("skill: {primary_skill}");
    print!("{}", resolved.provenance);
    basis.record_mechanical(&id)?;
    archive_worker_dir(&project, &id, &dir, Utc::now())?;
    fs::create_dir_all(&dir).with_context(|| format!("failed to create {dir}"))?;

    let brief_path = write_brief(
        &dir,
        &id,
        &skill,
        task_label.as_deref(),
        agent_dir,
        &format!("{task}{}", resolved.menu),
        &basis,
    )?;

    let launch_path = dir.join("launch.sh");
    let status_path = wake::status_log_path(&dir);
    fs::OpenOptions::new()
        .create(true)
        .append(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&status_path)
        .with_context(|| format!("failed to create {status_path}"))?;

    let window_name = agent_window::worker_window_name(&id);
    // The status log exists and is empty; read it as the check-in baseline *before* the window is
    // launched, so a line the worker writes on its way up can still answer this dispatch.
    let armed_len = status_log_len(&dir)?;
    let created_at = Utc::now();
    let target = match agent_window::launch_worker_window(
        &session,
        &window_name,
        agent_dir,
        &invocation,
        &agent_window::WorkerPaths {
            brief: &brief_path,
            launch: &launch_path,
            status: &status_path,
        },
    ) {
        Ok(target) => target,
        Err(err) => {
            return Err(abandon_spawn(
                err,
                &dir,
                None,
                &format!("launch worker {id}"),
            ));
        }
    };

    let meta = WorkerMeta {
        setup: setup.map(|file| file.setup.clone()),
        id: id.clone(),
        menu: resolved.names,
        imports: resolved.imports,
        always: binding.always.clone(),
        on_from: binding.on_from.clone(),
        on_stage: binding.on_stage.clone(),
        gates: binding.gates.as_ref().map(|config| config.0.clone()),
        with: assignment.with,
        skill,
        agent,
        agent_family: agent_spec.tiered_family(),
        model: agent_spec.model().map(str::to_owned),
        effort: agent_spec.effort().map(str::to_owned),
        task_label,
        created_at,
        starting_until: Some(crate::telemetry::initial_deadline(
            created_at,
            cadence.delay,
        )),
        project: project.clone(),
        tree,
        window: target.render(),
        brief: brief_path,
        launch: launch_path,
        session_link,
    };
    if let Err(err) =
        tag_worker_window(&target, &project, &id).and_then(|()| write_meta(&dir, &meta))
    {
        return Err(abandon_spawn(
            err,
            &dir,
            Some(&target),
            &format!("finish launching worker {id}"),
        ));
    }

    // The lead arms the check-in and is the only one who does: a worker can disarm it by
    // reporting, never by staying quiet about it. The cadence was resolved before the launch —
    // this only writes it.
    let armed = watch::arm_checkin(&dir, cadence, armed_len, created_at)?;

    eprint!("{}", resolved.warnings);
    println!("spawned: {id}");
    println!("window: {window_name}");
    println!("agent: {}", meta.agent);
    print_worker_tier(&meta);
    if let Some(label) = &meta.task_label {
        println!("task: {label}");
    }
    println!("brief: {}", meta.brief);
    println!("wait: niles wait");
    println!("peek: niles peek {id}");
    println!("report: niles report {id}");
    println!("send: niles send {id} <message>");
    println!("close: niles close {id}");
    if let Some(label) = &meta.task_label {
        println!("close_task: niles close --task {label}");
    }
    println!("workers: niles workers");
    match armed {
        Some(delay) => {
            println!("checkin: {}", watch::describe_delay(delay));
            println!("quiet: niles quiet {id}");
        }
        None => println!("checkin: off"),
    }

    Ok(())
}

fn resolve_agent(
    path: &Utf8Path,
    manifest: &workspace_manifest::WorkspaceManifest,
    skill: &SkillName,
    tier: agents::roster::Tier,
    agent: Option<String>,
    config: &crate::config::spec::ProjectConfig,
) -> Result<String> {
    if *skill == manifest.entry {
        bail!("entry skill {skill} cannot be spawned");
    }
    let binding = manifest
        .binding(skill)
        .with_context(|| format!("in {path}"))?;
    if let Some(agent) = agent {
        let requested = agents::AgentSpec::parse(&agent, &config.models)?;
        agents::validate_model(&requested, &config.models)?;
        if !binding.allows(&requested, &config.models)? {
            bail!(
                "agent {agent} is not allowed for skill '{skill}' in manifest {path}; allowed agents: {}",
                binding.agents().collect::<Vec<_>>().join(", ")
            );
        }
        return Ok(agent);
    }
    Ok(binding.tier(tier)?.primary.clone())
}

#[cfg(test)]
#[path = "spawn_tests.rs"]
mod tests;

fn tag_worker_window(target: &WindowTarget, project: &Utf8Path, id: &str) -> Result<()> {
    tmux::set_window_option(target, "@niles-project", project.as_str())?;
    tmux::set_window_option(target, "@niles-worker-id", id)?;
    // Keep the pane after the agent exits. Whatever killed it — a trust prompt, a crash — is on
    // that pane, and destroying the window destroys the only evidence of why.
    tmux::set_window_option(target, "remain-on-exit", "on")
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

fn write_brief(
    dir: &Utf8Path,
    id: &str,
    skill: &SkillName,
    task_label: Option<&str>,
    project: &Utf8Path,
    task: &str,
    basis: &TaskBasis,
) -> Result<Utf8PathBuf> {
    let path = dir.join("brief.md");
    let report_file = report_path(dir);
    let task_label = match task_label {
        Some(task_label) => task_label,
        None => UNLABELED_TASK_LABEL,
    };
    let body = render_template(
        BOOTSTRAP,
        &[
            ("{id}", id),
            ("{skill}", skill.as_str()),
            ("{task_label}", task_label),
            ("{project}", project.as_str()),
            ("{report_path}", report_file.as_str()),
            ("{task}", task),
            (
                "{from_report}",
                &match basis {
                    TaskBasis::FromReport(path) => format!("\nfrom_report: {path}"),
                    TaskBasis::Mechanical { .. } | TaskBasis::None => String::new(),
                },
            ),
        ],
    );
    fs::write(&path, body).with_context(|| format!("failed to write {path}"))?;
    Ok(path)
}

fn abandon_spawn(
    err: anyhow::Error,
    dir: &Utf8Path,
    target: Option<&WindowTarget>,
    failure: &str,
) -> anyhow::Error {
    let (cleaned, location) = match target {
        Some(target) => ("launched worker", target.to_string()),
        None => ("partial worker", dir.to_string()),
    };
    match cleanup_failed_spawn(dir, target) {
        Ok(()) => err.context(format!(
            "failed to {failure}; cleaned up {cleaned} at {location}"
        )),
        Err(cleanup_err) => err.context(format!(
            "failed to {failure}; additionally failed to clean up {cleaned} at {location}: {cleanup_err}"
        )),
    }
}

fn cleanup_failed_spawn(dir: &Utf8Path, target: Option<&WindowTarget>) -> Result<()> {
    let mut failures = Vec::new();
    if let Some(target) = target
        && let Err(err) = tmux::kill_window(target)
    {
        failures.push(format!("failed to kill tmux window {target}: {err:#}"));
    }
    if let Err(err) = remove_dir_all_if_exists(dir) {
        failures.push(format!(
            "failed to remove partial worker dir {dir}: {err:#}"
        ));
    }

    if failures.is_empty() {
        Ok(())
    } else {
        bail!("{}", failures.join("; "))
    }
}

#[derive(Debug)]
enum TaskBasis {
    FromReport(Utf8PathBuf),
    Mechanical { log: Utf8PathBuf, reason: String },
    None,
}

impl Assignment {
    fn resolve(
        &self,
        project: &Utf8Path,
        live_worker: impl FnOnce(&str) -> Result<Option<Utf8PathBuf>>,
    ) -> Result<TaskBasis> {
        match (self.from.as_deref(), self.mechanical.as_deref()) {
            (Some(id), _) => {
                let dir =
                    live_worker(id)?.with_context(|| format!("no such live worker '{id}'"))?;
                let snapshot = WorkerSnapshot::new(id.to_owned(), dir.clone(), status_log(&dir)?);
                if !snapshot
                    .last_status_line()
                    .is_some_and(|line| line.starts_with("done:"))
                {
                    bail!("worker '{id}' latest status is not done:");
                }
                Ok(TaskBasis::FromReport(report_path(&dir)))
            }
            (None, Some(reason)) => {
                if reason.trim().is_empty() || reason.chars().any(char::is_control) {
                    bail!(
                        "--mechanical reason must be non-empty and contain no control characters"
                    );
                }
                let lead = session::latest_lead(project)?
                    .context("--mechanical requires a lead session")?;
                Ok(TaskBasis::Mechanical {
                    log: session::sessions_dir(project)?
                        .join(lead.id)
                        .join("decisions.log"),
                    reason: reason.to_owned(),
                })
            }
            (None, None) => Ok(TaskBasis::None),
        }
    }
}

impl TaskBasis {
    fn record_mechanical(&self, id: &str) -> Result<()> {
        if let Self::Mechanical { log, reason } = self {
            append_line(
                log,
                &format!("{} {id} mechanical: {reason}", Utc::now().format("%FT%TZ")),
            )?;
        }
        Ok(())
    }
}
