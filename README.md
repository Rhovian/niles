# Niles

Niles is a Rust CLI orchestration harness for coordinating agent CLIs such as Codex and Claude.

The project goal is to keep orchestration deterministic while letting agents handle judgment-heavy handoffs.

## Shape

- Rust owns workspace state, execution, persistence, run lookup, and validation.
- Agent adapters normalize fast-changing CLIs behind a stable internal contract.
- An analyzer probes installed agent CLIs and records their current capabilities.
- Workspace manifests bind manager, planner, implementer, reviewer, and validation roles to local agents and commands.

## CLI

```sh
niles
niles --goal "Fix the flaky auth test and open a PR"
niles ask "fix the failing auth test"
niles ask -a claude "review the current diff"
niles analyze
niles doctor
niles analyze --agent codex
niles spawn auth-fix --task auth --project ../my-app --agent codex "Fix the flaky login test"
niles peek auth-fix
niles send auth-fix "Please run the auth tests again."
niles wait --worker auth-fix
niles workers
niles worker-close --task auth
niles run task.yaml
niles step latest --index 1
niles exec-step latest 1
niles wait latest --index 1
niles step-close latest --index 1
niles status
niles status --json
niles watch
niles show
niles log --step 1
niles diff
niles resume
```

Bare `niles` is tmux-only. When run outside tmux from an interactive terminal, it starts an attached `niles` tmux session and re-runs the original command inside it; non-TTY launches still fail with guidance. Before launching the foreground manager, Niles creates `.niles/worker/` and interactively ensures `.niles/manifest.yaml` exists. The selected manager defaults to Claude on first setup. Niles writes a manager brief under `.niles/sessions/<id>/`; for Claude, that brief is passed as hidden manager context. Other agents receive the brief in their initial prompt. `niles --goal ...` seeds that startup context before the agent starts. Niles does not try to be a chat grammar; the foreground agent uses the explicit Niles commands as orchestration tools.

Short aliases are part of the interface:

```sh
niles a "summarize this repo"
niles r task.yaml
niles s
niles w
niles l
niles d
niles re
```

The default path should feel like an axi: terse commands, obvious defaults, compact output, and YAML only when a workflow needs to be explicit.

## Worker

Spawn a worker agent in tmux:

```sh
niles spawn auth-fix --task auth --project ../my-app --agent codex "Fix the flaky login test"
niles spawn reviewer --task auth --project ../my-app --agent claude "Review the auth fix"
niles peek auth-fix
niles send auth-fix "Please rerun the failing test and report the result."
niles wait --worker auth-fix
niles workers
niles worker-close --task auth
```

Spawn writes a brief and launch script under `.niles/worker/<id>/`, records tmux metadata in `.niles/worker/<id>.json`, and starts a `niles-<id>` window. If you are already inside tmux, the worker appears in your current session; otherwise Niles uses a detached `niles` session. `--task <label>` records a task label in worker metadata so a task or wave can be cleaned up as a group.

Worker briefs include a status file. The foreground manager uses `niles wait --worker <id>` to block until the next actionable wake and print it. `niles wait` is the single wake-delivery mechanism. Workers stay warm after `done:`; `done:` means the manager should wake, inspect, and optionally send follow-up, not close the worker. The normal lifecycle is spawn -> (`wait` <-> `send`)* -> cleanup. Use `niles workers` to list live workers, `niles worker-close <id>` to close one, `niles worker-close --task <label>` to close a current-workspace task group, or `niles worker-close --all` to close all live workers in the current workspace. Batch close reports each worker and continues after individual failures. `--task <label>` exits nonzero when no live workers match the label; `--all` prints `no live workers` and exits successfully when there is nothing to close.

Task labels share the worker id-adjacent namespace: they may contain only ASCII letters, numbers, `_`, and `-`, and `archive` is reserved because `.niles/worker/archive/` stores closed worker archives.

`niles workers` includes a window health column. `window-dead` means worker metadata remains, but the recorded tmux window is not currently present.

## Wake Lines

Actionable wake states are `done:`, `failed:`, `blocked:`, `needs-decision:`, and `closed:`. Unindexed waits, including `niles wait --worker <id>` and `niles wait <run>`, consume returned wake lines by advancing a `status.ack` cursor beside `status.log`; old unacknowledged lines remain deliverable, and acknowledged lines are not delivered again. Re-running `niles wait --worker <id>` after a prior wait returns is the send -> wait follow-up primitive: the second sequential wait attaches normally and returns the next actionable line. Only one unindexed wait may attach to a status log at a time: the active waiter is recorded in `status.waiter` with its pid and start time, and a concurrent duplicate wait fails loudly instead of stealing the wake. Each consumed wake is recorded in `status.ack.log` for later diagnosis. Indexed waits such as `niles wait <run> --index N` scan the full log for an actionable line containing the exact token pair `step N`, so generic wake lines do not satisfy indexed waits. For indexed run waits, `closed:` must also mention the matching step; worker waits treat any `closed:` line as terminal.

## Role Workflows

Role-based workflow YAML can use workspace role bindings:

```yaml
goal: "Fix flaky auth test"
workspace: ../my-app

steps:
  - role: planner
    task: "Analyze likely causes. Do not edit files."
  - role: implementer
    task: "Implement the fix using the planner output."
  - role: validation
  - role: reviewer
    task: "Review the current diff and validation result."
```

Workspace role bindings live in `.niles/manifest.yaml` with `manager`, `planner`, `implementer`, `reviewer`, and `validation_command` keys. Bare `niles` creates that file on first interactive launch, prompts for the foreground manager agent on every launch, and can optionally walk through updating the manifest roles. `niles run` resolves `planner`, `implementer`, `reviewer`, and `validation` role steps from the workspace manifest. It always prepares run state only; advance work one step at a time with `niles step` for interactive agent windows or `niles exec-step` for captured command/agent steps.

Role workflows label each step with a role: `planner`, `implementer`, `validation`, or `reviewer`. `niles status` and `niles show` surface those roles while the manager drives the run.

Run state includes pending, running, completed, and failed steps. Use `niles watch` from another terminal to see the workflow update live while a long agent or validation step is still active.

Before each agent step, Niles writes a markdown handoff context file beside the step logs and appends that file path to the agent prompt. The context includes the goal, current role/task, prior agent output, validation output, and the latest captured diff. `niles show` displays context paths, and `niles status --json` exposes them as structured state.

## Project Config

Put shared defaults in `niles.yaml` or `.niles.yaml`:

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

Niles has built-in profiles for common agents such as `codex` and `claude`. Agent references can optionally select a family-specific model and effort tier with `family:model[:effort]`, for example `codex:gpt-5.5:xhigh`, `codex:gpt-5.4`, `claude:opus:max`, or `claude:sonnet:med`. Task files, `niles spawn --agent`, and workspace manifest role bindings all accept this syntax. Task files and project config can still override agents, commands, and workspace values locally when a project needs an explicit invocation.

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

When `binary` or `args` are omitted for a known agent, Niles resolves them from its built-in profile. The resolved invocation is an implementation detail owned by Niles so fast-changing agent CLIs can be updated in one place.

Captured steps stream stdout/stderr live and also write stdout, stderr, git diff, and metadata into the resolved workspace's `.niles/runs/<id>/steps/`. Niles records run pointers so commands can resolve workspace-anchored runs by id or `latest`. Interactive agent steps run in tmux windows; close them with `niles step-close` after reviewing the worker output.

Agent steps also get `.context.md` handoff files. These are the durable bridge between roles: the implementer can read the planner output, and the reviewer can read both validation output and the current diff without relying on terminal scrollback.

Inspect a run with:

```sh
niles show
niles status
niles status --json
niles watch
niles log
niles log --step 2 --stderr
niles log --both
niles diff
niles diff --step 1
```

When a step fails, Niles prints the failed step, exit code, stderr log path, diff path, and a short stderr tail before exiting nonzero.

Resume a task-backed run after fixing the cause of a failure:

```sh
niles resume
```

Resume keeps completed steps, resets the first incomplete step and everything after it to pending, then prints the command to continue driving that step. It validates that the original task file still has the same step shape before updating state.

`niles status` uses compact, agent-readable output by default. Use `niles status --json` when a tool needs the raw persisted state.

## Status

Niles currently supports sequential manager-driven runs, tmux worker windows, workspace role manifests, local analyzer support, and persisted workspace-anchored run logs.
