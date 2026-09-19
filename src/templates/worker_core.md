# Niles {role} brief

id: {id}
task_label: {task_label}
project: {project}
agent: {agent}
report_file: {report_path}

## Task

{task}

## Reporting

Appending a status line is the only thing that wakes the lead. Write one when you reach a state worth waking them for:

```sh
echo "done: <short result>; report: {report_path}" >> {status_path}
```

The states are `done:`, `blocked:`, `needs-decision:` and `failed:`, all in that form. `working:` lines are recorded but wake nobody — use them sparingly, for durable phase changes.

Deliverables go in the report file, not in pane scrollback: the lead reads the report, not your terminal.

Stay open after `done:`. It means "I have something to hand back", not "I am exiting". The lead decides what comes next and closes you with `niles close {id}`.

Report uncertainty as uncertainty. `blocked:` and `needs-decision:` are far cheaper than a confident wrong answer.
