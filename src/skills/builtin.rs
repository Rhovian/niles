use super::SkillName;
use anyhow::{Result, bail};

const SHARED_PREFACE: &str = "Work from this brief and the code. Do not read other agents' briefs, reports or notes in Niles state unless this brief or a message from the lead names one.

Deliverables go in the report file; the lead reads it, not your terminal.

Report uncertainty as uncertainty.";

// R1 retains the compiled prompt table; R2 replaces it with skill resolution.
pub(crate) fn primary(skill: &SkillName) -> Result<String> {
    let fragment = match skill.as_str() {
        "lead" => return Ok(include_str!("../../prompts/lead_brief.md").to_owned()),
        "worker" => include_str!("../../prompts/role_worker.md"),
        "reviewer" => include_str!("../../prompts/role_reviewer.md"),
        "security" => include_str!("../../prompts/role_security.md"),
        "research" => include_str!("../../prompts/role_research.md"),
        "design" => include_str!("../../prompts/role_design.md"),
        _ => bail!("no compiled primary skill {skill}"),
    };
    Ok(format!("{SHARED_PREFACE}\n\n{fragment}"))
}

pub(crate) const BOOTSTRAP: &str = r#"# Niles {skill} brief
You are niles worker `{id}`. Report with `niles agent status --help`.

id: {id}
task_label: {task_label}
project: {project}
report_file: {report_path}{from_report}

## Task

{task}
"#;
