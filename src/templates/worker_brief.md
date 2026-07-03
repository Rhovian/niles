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

## Wake Contract

Append actionable status lines to the status file so Niles can wake the foreground manager:

```sh
{wake_examples}
```

Use `working:` sparingly for durable phase changes; it is recorded but does not wake the manager.
When the work is complete, the final `done:` line must mention the report file, for example `done: <short result>; report: {report_path}`.
