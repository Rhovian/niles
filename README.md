# Niles

Niles is a Rust CLI orchestration harness for coordinating agent CLIs such as
Codex and Claude in tmux sessions. The Rust CLI owns deterministic work —
workspace manifests, tmux window placement, worker metadata, status logs,
and schema-stamped artifacts —
while agents own the judgment-heavy work: planning, implementation, review,
handoff wording, and deciding when a task is complete.

## Requirements

- A Rust toolchain with edition 2024 support (1.85+).
- `tmux` — Niles runs the lead and its workers as windows of your current tmux
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

Bare `niles` turns the current tmux pane into the lead agent. Niles never
creates, names, pins, or attaches a tmux session: run it inside the session you
are already attached to, and it fails with one line of guidance if you are not
in tmux. That is also what makes worker placement a fact rather than a
resolution strategy — workers are windows of the session you are looking at.

The launch prelude creates `.niles/worker/` and interactively ensures
`.niles/manifest.yaml` exists, prompting for the `manager` (defaulting to
Claude on first setup) and optionally the other role bindings.

Niles writes a lead brief under `.niles/sessions/<id>/manager.md` pointing at
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
`closed:`. Workers stay warm after `done:` — it tells the lead to inspect
and optionally send follow-up, not to terminate; cleanup happens explicitly at
integration time. Each wait records a byte offset into the status log in
`.niles/worker/<id>/status.cursor` and advances it only when it delivers a
line, so each actionable line is returned exactly once. Concurrent waits on one
worker are serialised by an advisory lock on that cursor rather than rejected:
one is handed the line, the other keeps waiting.

## Roles

Niles composes a brief per role rather than handing every agent the same one.
A worker's brief is a short shared contract — its id, its report file, and the
status lines that wake the lead — plus exactly one role fragment:

```sh
niles spawn impl --role worker   --agent codex  "Implement the fix"
niles spawn rev  --role reviewer --agent claude "Review impl's change"
```

- **lead** — the foreground agent. Owns the outcome *and the plan*: decides what
  gets built and how, then delegates the implementation, independent judgment on
  it, and anything needing parallelism or a fresh context. Does not implement.
- **worker** — owns the change, and owns the gate. Runs the project's build,
  tests and linters before reporting `done:`, and says what they printed.
- **reviewer** — owns judgment about the change. Does not re-run the gate, and
  must name a reachable attacker before writing a hardening finding.

Gate ownership is the reason the fragments are split: when every agent is told
the same thing about verification, every agent runs the test suite.

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

Manifest bindings accept built-in agent families and agents from project
config; unknown bare agent names are rejected.

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
niles spawn auth-impl --task auth --agent codex:gpt-5.5:xhigh \
  "Fix the flaky auth test, then run the project's checks."
niles wait auth-impl
niles report auth-impl
niles spawn auth-rev --task auth --role reviewer --agent claude:opus:high \
  "Review auth-impl's fix. Its report says which checks it ran."
niles wait auth-rev
niles report auth-rev
niles close --task auth
```

## Status

Niles currently supports lead sessions in the current tmux pane, tmux worker
windows, workspace role manifests, worker reports, worker archives, and
worker status-log wake delivery.

## License

Dual-licensed under MIT or Apache-2.0, at your option.
