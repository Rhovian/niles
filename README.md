# Niles

Niles is a Rust CLI orchestration harness for coordinating agent CLIs such as Codex and Claude.

The project goal is to keep orchestration deterministic while letting agents handle judgment-heavy handoffs.

## Shape

- Rust owns workspace state, execution, policies, persistence, and validation.
- Agent adapters normalize fast-changing CLIs behind a stable internal contract.
- An analyzer probes installed agent CLIs and records their current capabilities.
- A router agent can decide the next handoff using structured JSON decisions.

## CLI

```sh
niles ask "fix the failing auth test"
niles ask -a claude "review the current diff"
niles analyze
niles doctor
niles analyze --agent codex
niles run task.yaml
niles status
niles show
niles log --step 1
niles diff
niles resume
```

Short aliases are part of the interface:

```sh
niles a "summarize this repo"
niles r task.yaml
niles s
niles l
niles d
```

The default path should feel like an axi: terse commands, obvious defaults, compact output, and YAML only when a workflow needs to be explicit.

## Project Config

Put shared defaults in `niles.yaml` or `.niles.yaml`:

```yaml
workspace: .

agents:
  codex:
    binary: codex
    args: ["exec"]
  claude:
    binary: claude
    args: ["-p"]

commands:
  test:
    run: cargo test
```

Task files can still override agents, commands, and workspace values locally.

## Example Task

```yaml
goal: "Fix flaky auth test"
workspace: .

agents:
  codex:
    binary: codex
    args: ["exec"]
  claude:
    binary: claude
    args: ["-p"]

steps:
  - agent: claude
    task: "Analyze likely causes. Do not edit files."
  - agent: codex
    task: "Implement the fix using the analysis above."
  - command: test

commands:
  test: cargo test auth
```

When `args` are omitted, Niles uses built-in defaults for the common agents:

- `codex`: `codex exec <prompt>`
- `claude`: `claude -p <prompt>`

Each step writes stdout, stderr, git diff, and metadata into `.niles/runs/<id>/steps/`.

Inspect a run with:

```sh
niles show
niles log
niles log --step 2 --stderr
niles log --both
niles diff
niles diff --step 1
```

When a step fails, Niles prints the failed step, exit code, stderr log path, diff path, and a short stderr tail before exiting nonzero.

## Status

Niles is just starting. The first target is a sequential workflow runner with local analyzer support, persisted run logs, and public capability manifests.
