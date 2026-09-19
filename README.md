# Niles

Niles is a Rust CLI orchestration harness for coordinating agent CLIs such as
Codex and Claude in tmux sessions. The Rust CLI owns deterministic work —
workspace manifests, tmux window placement, worker metadata, status logs,
and schema-stamped artifacts —
while agents own the judgment-heavy work: planning, implementation, review,
handoff wording, and deciding when a task is complete.

## Requirements

- A Rust toolchain with edition 2024 support (1.85+).
- `tmux` — Niles runs the manager and workers as windows of your current tmux
  session, so `niles` must be run from inside tmux.
- The agent CLIs you intend to use, on your `PATH` (e.g. `codex`, `claude`).

## Install

```sh
cargo install --path .   # installs the `niles` binary on your PATH
cargo build --release    # or a local build at target/release/niles
```

## Launch

```sh
niles
```

Bare `niles` turns the current tmux pane into the manager agent. Niles never
creates, names, pins, or attaches a tmux session: run it inside the session you
are already attached to, and it fails with one line of guidance if you are not
in tmux. That is also what makes worker placement a fact rather than a
resolution strategy — workers are windows of the session you are looking at.

The launch prelude creates `.niles/worker/` and interactively ensures
`.niles/manifest.yaml` exists, prompting for the `manager` (defaulting to
Claude on first setup) and optionally the other role bindings.

Niles writes a manager brief under `.niles/sessions/<id>/manager.md` pointing at
the manifest and its flow. For Claude the brief is passed via
`--append-system-prompt` (hidden context); other agents receive it in their
initial prompt. Niles owns no chat grammar — the foreground agent drives the
conversation and invokes explicit Niles commands as orchestration tools.

## Worker Lifecycle

Spawn a worker agent into a tmux window:

```sh
niles spawn auth-fix --task auth --agent codex "Fix the flaky login test"
niles peek auth-fix
niles send auth-fix "Rerun the failing test and report the result."
niles wait auth-fix
niles send auth-fix --wait "Rerun it and report."   # send, then block for the reply
niles workers
niles close --task auth
```

Workers always belong to the workspace the spawn ran from; to work in another
one, `cd` there first. Spawn writes a brief and launch script under `.niles/worker/<id>/`,
records tmux metadata in `.niles/worker/<id>.json`, and starts a `niles-<id>`
window in the tmux session the spawn was run from. `niles spawn` outside tmux
fails before writing anything, rather than placing a worker in a session nobody
is attached to. The normal lifecycle is `spawn -> (wait <-> send)* -> cleanup`.

`--task <label>` records a task label so a task or wave can be cleaned up as a
group. Labels use the same ASCII grammar as worker ids (`A-Z`, `a-z`, `0-9`,
`_`, `-`) and reserve `archive`, which names the `.niles/worker/archive/` store
for closed workers. Close a worker with `niles close <id>`, a group with
`--task <label>`, or everything in the workspace with `--all`; batch close
reports each worker and continues past individual failures.

`niles workers` lists only workers in the current workspace and includes a
window-health column. `window-dead` means worker metadata remains but the
recorded tmux window is gone — a stale directory that is a cleanup candidate,
not a healthy warm pane.

## Wake Contract

`niles wait` is the single wake-delivery mechanism: it prints the next
actionable line from a worker status log. Use `niles wait <id>` for
one worker or `niles wait --task <label>` for a live task group. The five
actionable states are `done:`, `failed:`, `blocked:`, `needs-decision:`, and
`closed:`. Workers stay warm after `done:` — it tells the manager to inspect
and optionally send follow-up, not to terminate; cleanup happens explicitly at
integration time. Each wait records a byte offset into the status log in
`.niles/worker/<id>/status.cursor` and advances it only when it delivers a
line, so each actionable line is returned exactly once. Concurrent waits on one
worker are serialised by an advisory lock on that cursor rather than rejected:
one is handed the line, the other keeps waiting.

## Role Workflows

Workspace role bindings live in `.niles/manifest.yaml`:

```yaml
manager: claude
planner: claude
worker: codex
reviewer: claude
validation_command: test
flow:
  - planner
  - worker
  - reviewer
```

`flow` holds manifest role tokens, not a one-shot plan. The manager-facing flow
is a worker-verification-reviewer loop ending in reviewer consensus or
escalation, with `validation_command` supplying verification between worker and
reviewer passes — the manifest is the only source of truth for the orchestration
path. Manifest prompts accept built-in agent families and agents from project
config; unknown bare agent names are rejected.

The manager applies this flow by spawning planner, worker, validation, and
reviewer workers as needed, using worker reports as durable handoff artifacts.

## Project Config

Niles loads the first of `niles.yaml` or `.niles.yaml` for shared agent
defaults. Role bindings and flow stay in `.niles/manifest.yaml`:

```yaml
agents:
  local-reviewer:
    binary: review-agent
    args: ["--format", "plain"]
```

Niles has built-in profiles for common agents such as `codex` and `claude`, so
`binary` can be omitted for known agents. Agent references accept a
`family:model[:effort]` qualifier — for example `codex:gpt-5.5:xhigh`,
`claude:opus:max`, or `claude:sonnet:med` — in `niles spawn --agent` and
manifest role bindings.

## Example Task

```sh
niles spawn auth-plan --task auth --agent claude:opus:high \
  "Analyze the flaky auth test. Do not edit files; write findings to report.md."
niles wait auth-plan
niles report auth-plan
niles spawn auth-impl --task auth --agent codex:gpt-5.5:xhigh \
  "Implement the auth test fix using the planner report, then run cargo test auth."
niles wait auth-impl
niles report auth-impl
niles close --task auth
```

## Status

Niles currently supports manager sessions in the current tmux pane, tmux worker
windows, workspace role manifests, worker reports, worker archives, and
worker status-log wake delivery.

## License

Dual-licensed under MIT or Apache-2.0, at your option.
