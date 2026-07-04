# Niles

Niles is a Rust CLI orchestration harness for coordinating agent CLIs such as
Codex and Claude in tmux sessions. The Rust CLI owns deterministic work —
config and spec loading, workspace-anchored run directories, agent binary
probing, captured step execution, stdout/stderr/diff capture, and resumable run
state — while agents own the judgment-heavy work: planning, implementation,
review, handoff wording, and deciding when a task is complete.

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

Short aliases are part of the interface: `niles r` (run), `niles s` (status),
`niles w` (watch), `niles l` (log), `niles d` (diff), `niles re` (resume).

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
metadata in `.niles/worker/<id>.json`, and starts a `niles-<id>` window (in your
current session if inside tmux, otherwise a detached `niles` session). The
normal lifecycle is `spawn -> (wait <-> send)* -> cleanup`.

`--task <label>` records a task label so a task or wave can be cleaned up as a
group. Labels use the same ASCII grammar as worker ids (`A-Z`, `a-z`, `0-9`,
`_`, `-`) and reserve `archive`, which names the `.niles/worker/archive/` store
for closed workers. Close a worker with `niles worker-close <id>`, a group with
`--task <label>`, or everything in the workspace with `--all`; batch close
reports each worker and continues past individual failures.

`niles workers` includes a window-health column. `window-dead` means worker
metadata remains but the recorded tmux window is gone — a stale directory that
is a cleanup candidate, not a healthy warm pane.

## Wake Contract

`niles wait` is the single wake-delivery mechanism: it polls the relevant worker
or run status log and prints the next actionable line. The five actionable wake
states are `done:`, `failed:`, `blocked:`, `needs-decision:`, and `closed:`.
Workers stay warm after `done:` — it is a wake telling the manager to inspect
and optionally send follow-up, not a terminate request; cleanup happens
explicitly at integration time.

Unindexed waits (`niles wait --worker <id>`, `niles wait <run>`) consume wake
lines by advancing a `status.ack` cursor beside the log, so pre-attach lines
stay deliverable once but are never returned twice; re-running the wait after it
returns is the send -> wait follow-up primitive. These waits are single-consumer:
the active waiter is recorded in `status.waiter`, and a concurrent duplicate
fails loudly instead of stealing the wake.

Indexed waits (`niles wait <run> --index N`) scan the full log for an actionable
line containing the exact token pair `step N`, so generic wake lines do not
satisfy them. `closed:` is terminal for worker waits; for indexed run waits it
must also mention the matching step.

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

Explicit workflow files remain available for user-supplied automation. They can
use manifest role names but do not define or replace the workspace flow:

```yaml
goal: "Fix flaky auth test"
workspace: ../my-app

steps:
  - role: planner
    task: "Analyze likely causes. Do not edit files."
  - role: worker
    task: "Implement the fix using the planner output."
  - role: validation
  - role: reviewer
    task: "Review the current diff and validation result."
```

`niles run` resolves `planner`, `worker`, `reviewer`, and `validation` role
steps from the manifest and only prepares run state; the manager advances work
one step at a time with `niles step` for interactive agent windows or `niles
exec-step` for captured command/agent steps. Run state tracks pending, running,
completed, and failed steps; `niles status`, `niles show`, and `niles watch`
surface roles and live progress from that state file.

Before each agent step, Niles writes a markdown `.context.md` handoff artifact
beside the step logs — goal, current role/task, prior agent output, validation
output, and the latest captured diff — and appends its path to the agent prompt.
That artifact is the durable contract between roles, not terminal scrollback.

## Project Config

Niles loads the first of `niles.yaml` or `.niles.yaml` for shared workspace,
agent, and command defaults for explicit workflow files (role bindings and flow
stay in the manifest):

```yaml
workspace: .

agents:
  local-reviewer:
    binary: review-agent
    args: ["--format", "plain"]

commands:
  test:
    run: cargo test
```

Niles has built-in profiles for common agents such as `codex` and `claude`, so
`binary`/`args` can be omitted for known agents. Agent references accept a
`family:model[:effort]` qualifier — for example `codex:gpt-5.5:xhigh`,
`claude:opus:max`, or `claude:sonnet:med` — in task files, `niles spawn
--agent`, and manifest role bindings.

## Example Task

```yaml
goal: "Fix flaky auth test"
workspace: .

steps:
  - agent: claude
    task: "Analyze likely causes. Do not edit files."
  - agent: codex
    task: "Implement the fix using the analysis above."
  - command: test

commands:
  test: cargo test auth
```

Captured steps stream stdout/stderr live and also write stdout, stderr, git
diff, and metadata into `.niles/runs/<id>/steps/`. Interactive agent steps run
in tmux windows; close them with `niles step-close` after review. Inspect a run
with `niles show`, `niles status` (`--json` for raw state), `niles watch`,
`niles log` (`--step N`, `--stderr`, `--both`), and `niles diff` (`--step N`).
When a step fails, Niles prints the failed step, exit code, log and diff paths,
and a short stderr tail before exiting nonzero.

## Analyzer

Agent CLIs change quickly, so Niles does not assume a fixed flag set. `niles
analyze` builds a local capability manifest under `.niles/capabilities/` by
running safe probes (`--version`, `--help`) and probing model acceptance for
requested specs, built-in aliases, and model-qualified manifest bindings; pass
`--agent <name>` to probe a single agent. Manifests record accepted and rejected
models with CLI version and timestamp;
launch validation consults fresh manifests first, fails before spawning for
known-rejected models, and falls back to static validation otherwise. `niles
doctor` reports environment readiness.

## Resume

`niles resume` reloads the original task file for a persisted run, verifies the
step shape still matches saved state, keeps completed steps, resets the first
incomplete step and everything after it to pending, and prints the next command
to continue. It is intentionally narrower than full checkpointing but enough to
recover from failed validation or interrupted agent steps.

## Status

Niles currently supports sequential manager-driven runs, tmux worker windows,
workspace role manifests, local analyzer support, and persisted
workspace-anchored run logs.

## License

Dual-licensed under MIT or Apache-2.0, at your option.
