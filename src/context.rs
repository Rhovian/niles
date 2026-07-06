use std::fs;

use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};

use crate::{
    display::MISSING_DISPLAY_FIELD_PLACEHOLDER,
    state::{RunState, StepKind},
    util::{absolute_path, slugify},
};

const CONTEXT_ARTIFACT_MAX_CHARS: usize = 12_000;

pub fn write_agent_context(
    state: &RunState,
    step_number: usize,
    role: Option<&str>,
    agent: &str,
    task: &str,
    workspace: &Utf8Path,
    steps_dir: &Utf8Path,
) -> Result<Utf8PathBuf> {
    let path = steps_dir.join(format!("{step_number:03}-{}.context.md", slugify(agent)));
    let mut body = String::new();

    body.push_str("# Niles Step Context\n\n");
    body.push_str(&format!("run: {}\n", state.id));
    body.push_str(&format!("run_status: {}\n", state.status));
    body.push_str(&format!("goal: {}\n", state.goal));
    body.push_str(&format!("workspace: {workspace}\n"));
    body.push_str(&format!("step: {step_number}\n"));
    body.push_str(&format!("role: {}\n", display_field(role)));
    body.push_str(&format!("agent: {agent}\n\n"));

    body.push_str("## Current Task\n\n");
    append_fenced(&mut body, "text", task);

    append_prior_step_summary(&mut body, state, step_number);
    append_prior_agent_output(&mut body, state, step_number);
    append_validation_output(&mut body, state, step_number);
    append_latest_diff(&mut body, state, step_number);

    fs::write(&path, body).with_context(|| format!("failed to write {path}"))?;
    Ok(path)
}

pub fn agent_prompt(task: &str, context_path: Option<&Utf8Path>) -> Result<String> {
    let Some(context_path) = context_path else {
        return Ok(task.to_owned());
    };
    let context_path = absolute_path(context_path)?;

    Ok(format!(
        "{task}\n\nNiles handoff context: {context_path}\nRead that file before acting. It contains the task goal, prior agent output, validation output, and the latest captured diff."
    ))
}

fn append_prior_step_summary(body: &mut String, state: &RunState, step_number: usize) {
    body.push_str("## Prior Steps\n\n");
    let prior_steps = state
        .steps
        .iter()
        .filter(|step| step.index < step_number)
        .collect::<Vec<_>>();

    if prior_steps.is_empty() {
        body.push_str("No prior steps.\n\n");
        return;
    }

    body.push_str("| index | role | kind | label | status | exit |\n");
    body.push_str("| --- | --- | --- | --- | --- | --- |\n");
    for step in prior_steps {
        let exit = step
            .exit_code
            .map(|code| code.to_string())
            .unwrap_or_else(|| "-".to_owned());
        body.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} |\n",
            step.index,
            markdown_cell(display_field(step.role.as_deref())),
            step.kind,
            markdown_cell(&step.label),
            step.status,
            exit
        ));
    }
    body.push('\n');
}

fn append_prior_agent_output(body: &mut String, state: &RunState, step_number: usize) {
    body.push_str("## Prior Agent Output\n\n");
    let mut found = false;

    for step in state
        .steps
        .iter()
        .filter(|step| step.index < step_number && matches!(&step.kind, StepKind::Agent))
    {
        found = true;
        body.push_str(&format!(
            "### Step {}: {} agent {}\n\n",
            step.index,
            display_field(step.role.as_deref()),
            step.label
        ));
        append_artifact_excerpt(body, "stdout", step.stdout.as_deref(), "text");
        append_artifact_excerpt(body, "stderr", step.stderr.as_deref(), "text");
    }

    if !found {
        body.push_str("No prior agent output.\n\n");
    }
}

fn append_validation_output(body: &mut String, state: &RunState, step_number: usize) {
    body.push_str("## Validation Output\n\n");
    let mut found = false;

    for step in state
        .steps
        .iter()
        .filter(|step| step.index < step_number && matches!(&step.kind, StepKind::Command))
    {
        found = true;
        body.push_str(&format!(
            "### Step {}: {} command {}\n\n",
            step.index,
            display_field(step.role.as_deref()),
            step.label
        ));
        append_artifact_excerpt(body, "stdout", step.stdout.as_deref(), "text");
        append_artifact_excerpt(body, "stderr", step.stderr.as_deref(), "text");
    }

    if !found {
        body.push_str("No validation output yet.\n\n");
    }
}

fn append_latest_diff(body: &mut String, state: &RunState, step_number: usize) {
    body.push_str("## Latest Diff\n\n");

    let latest = state
        .steps
        .iter()
        .rev()
        .find(|step| step.index < step_number && step.diff.is_some());

    match latest.and_then(|step| step.diff.as_deref().map(|diff| (step.index, diff))) {
        Some((index, diff)) => {
            body.push_str(&format!("from_step: {index}\n\n"));
            append_artifact_excerpt(body, "diff", Some(diff), "diff");
        }
        None => body.push_str("No captured diff yet.\n\n"),
    }
}

fn append_artifact_excerpt(
    body: &mut String,
    label: &str,
    path: Option<&Utf8Path>,
    language: &str,
) {
    let Some(path) = path else {
        body.push_str(&format!("#### {label}\n\n<not available>\n\n"));
        return;
    };

    body.push_str(&format!("#### {label}\n\npath: {path}\n\n"));
    match read_excerpt(path) {
        Ok(excerpt) if excerpt.is_empty() => body.push_str("<empty>\n\n"),
        Ok(excerpt) => append_fenced(body, language, &excerpt),
        Err(err) => body.push_str(&format!("<failed to read {path}: {err}>\n\n")),
    }
}

fn read_excerpt(path: &Utf8Path) -> Result<String> {
    let body = fs::read_to_string(path).with_context(|| format!("failed to read {path}"))?;
    let mut chars = body.chars();
    let mut excerpt = chars
        .by_ref()
        .take(CONTEXT_ARTIFACT_MAX_CHARS)
        .collect::<String>();

    if chars.next().is_some() {
        excerpt.push_str("\n\n<truncated by niles>\n");
    }

    Ok(excerpt)
}

fn append_fenced(body: &mut String, language: &str, value: &str) {
    body.push_str(&format!("~~~{language}\n{value}\n~~~\n\n"));
}

fn markdown_cell(value: &str) -> String {
    value.replace('\n', " ").replace('|', "\\|")
}

fn display_field(value: Option<&str>) -> &str {
    match value {
        Some(value) => value,
        None => MISSING_DISPLAY_FIELD_PLACEHOLDER,
    }
}
