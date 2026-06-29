use std::{collections::BTreeMap, fs};

use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use chrono::Utc;

use crate::runner;
use crate::spec::{
    AgentConfig, CommandConfig, PromptMode, TaskSpec, TaskStep, load_project_config_from,
};

pub fn generate(
    goal: Vec<String>,
    project: Utf8PathBuf,
    planner: String,
    implementer: String,
    reviewer: String,
    command: String,
    run: bool,
    watch: bool,
) -> Result<()> {
    let goal = goal.join(" ");
    if goal.trim().is_empty() {
        bail!("manifest goal cannot be empty");
    }
    if watch && !run {
        bail!("--watch requires --run");
    }
    if !project.is_dir() {
        bail!("project path is not a directory: {project}");
    }

    let project_config = load_project_config_from(&project)?;
    let mut agents = BTreeMap::new();
    for agent in [&planner, &implementer, &reviewer] {
        let config = project_config
            .agents
            .get(agent)
            .cloned()
            .unwrap_or_else(|| default_agent_config(agent));
        agents.insert(agent.to_owned(), config);
    }

    let command_config = project_config
        .commands
        .get(&command)
        .cloned()
        .unwrap_or_else(|| default_command_config(&command));

    let spec = TaskSpec {
        goal: goal.clone(),
        workspace: Some(project),
        agents,
        steps: vec![
            TaskStep::Agent {
                agent: planner.clone(),
                task: planner_prompt(&goal),
                role: Some("planner".to_owned()),
            },
            TaskStep::Agent {
                agent: implementer.clone(),
                task: implementer_prompt(&goal),
                role: Some("implementer".to_owned()),
            },
            TaskStep::Command {
                command: command.clone(),
                role: Some("validation".to_owned()),
            },
            TaskStep::Agent {
                agent: reviewer.clone(),
                task: reviewer_prompt(&goal, &command),
                role: Some("reviewer".to_owned()),
            },
            TaskStep::Agent {
                agent: implementer,
                task: review_fix_prompt(&goal),
                role: Some("implementer".to_owned()),
            },
            TaskStep::Command {
                command: command.clone(),
                role: Some("validation".to_owned()),
            },
        ],
        commands: BTreeMap::from([(command, command_config)]),
    };

    let dir = Utf8Path::new(".niles").join("manifests");
    fs::create_dir_all(&dir).with_context(|| format!("failed to create {dir}"))?;
    let now = Utc::now();
    let id = format!(
        "{}{:09}Z",
        now.format("%Y%m%dT%H%M%S"),
        now.timestamp_subsec_nanos()
    );
    let path = dir.join(format!("{id}.yaml"));
    let body = serde_yaml::to_string(&spec).context("failed to serialize manifest")?;
    fs::write(&path, body).with_context(|| format!("failed to write {path}"))?;

    println!("manifest: {path}");
    if run {
        runner::run_manifest(path, watch)
    } else {
        println!("next: Run `niles run {path}`");
        Ok(())
    }
}

fn default_agent_config(agent: &str) -> AgentConfig {
    AgentConfig {
        binary: Some(agent.to_owned()),
        args: match agent {
            "codex" => vec!["exec".to_owned()],
            "claude" => vec!["-p".to_owned()],
            _ => Vec::new(),
        },
        prompt: PromptMode::Arg,
    }
}

fn default_command_config(command: &str) -> CommandConfig {
    CommandConfig::Full {
        run: if command == "test" {
            "cargo test".to_owned()
        } else {
            command.to_owned()
        },
    }
}

fn planner_prompt(goal: &str) -> String {
    format!(
        "Plan this task for the project.\n\nGoal:\n{goal}\n\nReturn a concise implementation plan, likely files to inspect or edit, risks, and validation steps. Do not edit files."
    )
}

fn implementer_prompt(goal: &str) -> String {
    format!(
        "Implement this task in the project.\n\nGoal:\n{goal}\n\nUse the planner output from the prior step if available in the Niles run logs. Keep changes scoped to the goal and avoid unrelated refactors."
    )
}

fn reviewer_prompt(goal: &str, command: &str) -> String {
    format!(
        "Review the current project diff for this task.\n\nGoal:\n{goal}\n\nCheck the diff and the `{command}` validation result. Report only actionable issues, missing tests, or concrete risks. Do not edit files."
    )
}

fn review_fix_prompt(goal: &str) -> String {
    format!(
        "Address actionable review feedback for this task if any was reported.\n\nGoal:\n{goal}\n\nIf the review found no issues, make no changes."
    )
}
