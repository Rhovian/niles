macro_rules! skills {
    ($($name:literal),*) => { &[$(($name, include_str!(concat!("../../prompts/skills/", $name, "/SKILL.md")))),*] };
}
pub(super) const SKILLS: &[(&str, &str)] = skills!(
    "worker", "reviewer", "security", "design", "research", "lead"
);

pub(crate) const BOOTSTRAP: &str = r#"# Niles {skill} brief
You are niles worker `{id}`. Report with `niles agent status --help`.

id: {id}
task_label: {task_label}
project: {project}
report_file: {report_path}{from_report}

## Task

{task}
"#;
