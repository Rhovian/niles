# Niles

Niles is a Rust CLI orchestration harness for coordinating agent CLIs such as
Codex and Claude in tmux sessions. The Rust CLI owns deterministic work —
workspace manifests, tmux window placement, worker metadata, status logs,
schema-stamped artifacts, token usage snapshots, and agent binary probing —
while agents own the judgment-heavy work: planning, implementation, review,
handoff wording, and deciding when a task is complete.

## Requirements

- A Rust toolchain with edition 2024 support (1.85+).
- `tmux` — Niles runs the manager and workers as tmux windows.
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

Bare `niles` launches the foreground manager agent and is tmux-only. When run
outside tmux from an interactive terminal it starts an attached `niles` tmux
session and re-runs the original command inside it; if that session already
exists it prompts to attach or launch a differently named session. Non-TTY
launches fail with guidance to start or attach tmux. The launch prelude creates
`.niles/worker/` and interactively ensures `.niles/manifest.yaml` exists,
prompting for the foreground `manager` (defaulting to Claude on first setup) and
optionally the other role bindings.

Niles writes a manager brief under `.niles/sessions/<id>/manager.md` pointing at
the manifest and its flow. For Claude the brief is passed via
`--append-system-prompt` (hidden context); other agents receive it in their
initial prompt. Niles owns no chat grammar — the foreground agent drives the
conversation and invokes explicit Niles commands as orchestration tools.

## Worker Lifecycle

Spawn a worker agent into a tmux window:

```sh
niles spawn auth-fix --task auth --project ../my-app --agent codex "Fix the flaky login test"
niles peek auth-fix
niles send auth-fix "Rerun the failing test and report the result."
niles wait --worker auth-fix
niles workers
niles worker-close --task auth
```

Spawn writes a brief and launch script under `.niles/worker/<id>/`, records tmux
metadata in `.niles/worker/<id>.json`, and starts a `niles-<id>` window in the
workspace-pinned tmux session. The pin is resolved from
`.niles/sessions/tmux-session.json`, the latest manager window session, or a
deterministic detached workspace session. If the operator renames the pinned
tmux session, `ensure_session_exists` creates a fresh empty session with the
bound name and later workers land there. The normal lifecycle is
`spawn -> (wait <-> send)* -> cleanup`.

`--task <label>` records a task label so a task or wave can be cleaned up as a
group. Labels use the same ASCII grammar as worker ids (`A-Z`, `a-z`, `0-9`,
`_`, `-`) and reserve `archive`, which names the `.niles/worker/archive/` store
for closed workers. Close a worker with `niles worker-close <id>`, a group with
`--task <label>`, or everything in the workspace with `--all`; batch close
reports each worker and continues past individual failures.

`niles workers` includes a window-health column. `window-dead` means worker
metadata remains but the recorded tmux window is gone — a stale directory that
is a cleanup candidate, not a healthy warm pane.

## Token Usage

Niles records token usage for supported agent families when a worker closes, and
can also compute a read-only live view while an agent is still running:

```sh
niles workers --usage
```

The default `niles workers` output is unchanged. `--usage` adds union columns
for Codex and Claude ledgers: both fill `input`, `output`, and `total`; Codex
fills `cached` and `reasoning`; Claude fills `cache_create` and `cache_read`.
Per-worker task rollups group by `--task` label. `pending` means the live
transcript is not available or has no token events yet; `unavailable` means
Niles could not attribute usage, the family is unsupported, Codex candidates
were ambiguous, or the transcript/sidecar could not be parsed. Numeric fields
render as `-` for pending and unavailable rows.

Usage snapshots are schema-stamped JSON artifacts:

- Workers: `.niles/worker/<id>/usage.json`, archived with the worker on close.

The top-level `usage.json` shape records `subject`, `agent`, `captured_at`,
optional `started_at`, `finished_at`, optional `wall_seconds`, `attribution`,
`turns`, and a tagged `usage` object. `usage.status = "available"` includes
`family`, `input_tokens`, `output_tokens`, `total_tokens`, plus family-specific
cache/reasoning fields. `usage.status = "unavailable"` includes `reason` and
`detail`; reasons include `missing`, `ambiguous_codex_candidates`,
`parse_error`, and `unsupported`.

Ledger home discovery uses `CODEX_HOME` for Codex, falling back to
`$HOME/.codex`, and `CLAUDE_CONFIG_DIR` for Claude, falling back to
`$HOME/.claude`.

## Wake Contract

`niles wait` is the single wake-delivery mechanism: it prints the next
actionable line from a worker status log. Use `niles wait --worker <id>` for
one worker or `niles wait --task <label>` for a live task group. The five
actionable states are `done:`, `failed:`, `blocked:`, `needs-decision:`, and
`closed:`. Workers stay warm after `done:` — it tells the manager to inspect
and optionally send follow-up, not to terminate; cleanup happens explicitly at
integration time. Waits are single-consumer and track what they have already
delivered, so each actionable line is returned exactly once.

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
`binary`/`args` can be omitted for known agents. Agent references accept a
`family:model[:effort]` qualifier — for example `codex:gpt-5.5:xhigh`,
`claude:opus:max`, or `claude:sonnet:med` — in `niles spawn --agent` and
manifest role bindings.

## Example Task

```sh
niles spawn auth-plan --task auth --project . --agent claude:opus:high \
  "Analyze the flaky auth test. Do not edit files; write findings to report.md."
niles wait --worker auth-plan
niles report auth-plan
niles spawn auth-impl --task auth --project . --agent codex:gpt-5.5:xhigh \
  "Implement the auth test fix using the planner report, then run cargo test auth."
niles wait --worker auth-impl
niles report auth-impl
niles worker-close --task auth
```

## Analyzer

Agent CLIs change quickly, so Niles does not assume a fixed flag set. `niles
analyze` builds a local capability manifest under `.niles/capabilities/` by
running safe probes (`--version`, `--help`) and probing model acceptance for
requested specs, built-in aliases, and model-qualified manifest bindings; pass
`--agent <name>` to probe a single agent. Manifests record accepted and rejected
models with CLI version and timestamp;
launch validation consults fresh manifests first, fails before spawning for
known-rejected models, and falls back to static validation otherwise. `niles
doctor` reports environment readiness and schema state.

## Status

Niles currently supports resident manager sessions, tmux worker windows,
workspace role manifests, local analyzer support, worker reports, worker
archives, and worker status-log wake delivery.

## License

Dual-licensed under MIT or Apache-2.0, at your option.
