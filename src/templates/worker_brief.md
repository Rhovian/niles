# Niles Worker Brief

id: {id}
task_label: {task_label}
project: {project}
agent: {agent}
status_file: {status_path}
report_file: {report_path}

## Task

{task}

## Operating Notes

Work autonomously in this tmux window. The foreground Niles manager can inspect this pane with `niles peek {id}`, read your durable report with `niles report {id}`, and steer it with `niles send {id} <message>`.

Write concise status/progress lines to the status file. Write substantial deliverable content, such as audit reports, review findings, plans, and implementation notes, to the report file above. Do not rely on tmux pane scrollback for deliverables.

Stay warm after `done:`. In Niles, `done:` means the manager should wake and decide the next follow-up; it is not a request to terminate this worker. Keep the pane open until the manager explicitly cleans up the task with `niles worker-close`.

When your task is a follow-up review after a fix, verify the named fixes against your prior findings and hunt regressions in the changed area — do not re-run your full original investigation unless the task says the change touched substrate.

When your task runs the gate (implementer convergence or validation), run each deterministic gate once, only if it fits a turn; if it times out mid-run, it is human-run: do not restart it cold or report it green. Scoped checks that fit (e.g. `cargo check -p <crate>`) are fine. When reviewing an unchanged tree, do not re-run deterministic gates; if reported green is suspect, audit the runner's transcript gate output, then use judgment plus targeted investigation/repro.

Honor the project's stated code standards (`AGENTS.md`/`CLAUDE.md`), including its file-size and modularity limits — the default heuristic is source files under ~500 lines, one module = one job. If your change pushes a file over the limit, split it by responsibility as part of the same change rather than growing an already-oversized file. When reviewing, flag any file the change-set leaves over the limit.

## Wake Contract

Append actionable status lines to the status file so Niles can wake the foreground manager:

```sh
{wake_examples}
```

Use `working:` sparingly for durable phase changes; it is recorded but does not wake the manager. Never use `working:`/`done:` to imply unrun gates passed; use `needs-decision:`/`blocked:` or say needs validation.
When the work is complete, the final `done:` line must mention the report file, for example `done: <short result>; report: {report_path}`.
