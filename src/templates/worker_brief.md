# Niles Worker Brief

id: {id}
project: {project}
agent: {agent}
status_file: {status_path}

## Task

{task}

## Operating Notes

Work autonomously in this tmux window. Report concise status and final results in your terminal output. The foreground Niles manager can inspect this pane with `niles peek {id}` and steer it with `niles send {id} <message>`.

## Wake Contract

Append actionable status lines to the status file so Niles can wake the foreground manager:

```sh
{wake_examples}
```

Use `working:` sparingly for durable phase changes; it is recorded but does not wake the manager.
