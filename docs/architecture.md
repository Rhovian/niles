# Architecture

Niles is intentionally a small runtime around unstable agent CLIs.

## CLI Ergonomics

Niles should behave like an axi:

- common actions need short commands
- defaults should make the first command useful
- output should be compact and scriptable
- raw JSON should be opt-in for inspection commands
- YAML should be available without being mandatory
- every advanced workflow should have a simple one-off equivalent

The main fast path launches the foreground manager agent:

```sh
niles
niles --goal "Fix the flaky auth test and open a PR"
```

Bare `niles` is tmux-only. If `$TMUX` is unset and stdin/stdout are TTYs, it starts an attached `niles` tmux session and re-runs the original argv inside it; when that session already exists, it prompts to attach it or launch a differently named attached session with the original argv. Non-TTY launches fail with instructions to start or attach tmux interactively. Its launch prelude creates `.niles/worker/` and interactively ensures `.niles/manifest.yaml` exists before the foreground manager starts.

The workspace manifest stores persistent role bindings:

```yaml
manager: claude
planner: claude
implementer: codex
reviewer: claude
validation_command: test
```

On every interactive launch, Niles prompts for the foreground `manager` value and can optionally update the other role bindings. Manifest prompts accept built-in agent families and agents configured in project config; unknown bare agent names are rejected. Configured manager agents launch through that same project config. Niles writes a manager brief under `.niles/sessions/<id>/manager.md` before launching the selected agent. For Claude, that brief is passed with `--append-system-prompt`, so the raw brief is not injected as a visible user message. Other manager agents receive the brief in their initial prompt. `--goal` seeds the startup context before the foreground agent starts. Niles does not own a natural-language command grammar. The foreground agent owns the conversation; Niles provides orchestration commands that agent can invoke.

The one-off worker path is:

```sh
niles ask "fix the failing test"
```

Worker agents can also be spawned into tmux windows:

```sh
niles spawn auth-fix --task auth --project ../my-app --agent codex "Fix the flaky login test"
niles peek auth-fix
niles send auth-fix "Run the auth tests again."
niles wait --worker auth-fix
niles workers
niles worker-close --task auth
```

Worker briefs include a status-file wake contract. `niles wait` polls the relevant worker or run status log on demand and prints the next actionable line. It is the single wake-delivery mechanism. Workers and reviewer workers stay warm after `done:` so the manager can inspect, send follow-up, and wait again; `done:` is a wake, not a terminate request. Cleanup happens at task or wave integration with `niles worker-close <id>`, `niles worker-close --task <label>`, or `niles worker-close --all`. Group cleanup is scoped to the invoking workspace's live worker directories; it does not close workers discovered only through the global index.

Task labels are stored in worker metadata and group warm workers for cleanup. They use the same ASCII grammar as worker ids (`A-Z`, `a-z`, `0-9`, `_`, `-`) and reserve `archive`, matching the worker archive directory namespace.

`niles workers` reads worker metadata and probes the recorded tmux target. Rows with metadata but no tmux window are marked `window-dead` so stale worker directories are visible as cleanup candidates rather than healthy warm panes.

Explicit workflow files remain the durable automation path:

```sh
niles run task.yaml
niles step latest --index 1
niles exec-step latest 1
```

Role-bound workflows use the workspace manifest rather than a generated workflow manifest:

```yaml
goal: "Fix flaky auth test"
workspace: ../my-app

steps:
  - role: planner
    task: "Analyze likely causes. Do not edit files."
  - role: implementer
    task: "Implement the fix."
  - role: validation
  - role: reviewer
    task: "Review the current diff and validation result."
```

`niles run` resolves `planner`, `implementer`, `reviewer`, and `validation` role steps from `.niles/manifest.yaml`. The manager advances prepared runs one step at a time with `niles step` for interactive agent windows or `niles exec-step` for captured command/agent steps.

Resolved roles are persisted in run state and shown by inspection commands so tmux or TUI views can group work by planner, implementer, reviewer, and validation surfaces without changing execution semantics.

Run directories live under the resolved workspace's `.niles/runs/<id>/`. Niles also writes pointer files under the launch directory, the workspace, and the global run index so selectors such as `latest` can resolve workspace-anchored runs. Run state is initialized with every planned step as pending. Niles marks the active step running before launching its tmux window or captured subprocess, then updates that same record to completed or failed. This gives tmux views a reliable state source instead of inferring progress from panes or logs.

`niles watch` is a thin live view over that same state file. Panes can own process display, and the state file can still drive summaries, role grouping, and completion detection.

`niles resume` reloads the original task file for a persisted run, verifies that the step shape still matches the saved run state, keeps completed steps, resets the first incomplete step and later steps, and prints the next command for the manager to run. This is intentionally narrower than full checkpointing, but it is enough to recover from failed validation or interrupted agent steps during early product trials.

Before launching an agent step, Niles writes a per-step markdown context artifact. The context file includes the goal, current role/task, prior step summary, prior agent stdout/stderr, validation stdout/stderr, and the latest captured diff. Niles records that path on the step state before the process starts and appends the absolute path to the agent prompt.

This keeps handoffs explicit without expanding the task YAML format. A tmux session can show each role in its own pane, while the durable context artifact remains the contract between planner, implementer, reviewer, and validation steps.

## Wake Contract

Status logs use five actionable wake states: `done:`, `failed:`, `blocked:`, `needs-decision:`, and `closed:`. `closed:` is terminal for worker waits; for indexed run waits it must mention the matching step.

Unindexed waits, including `niles wait --worker <id>` and `niles wait <run>`, consume wake lines by writing a numeric cursor to `status.ack` beside the status log. That keeps pre-attach wake lines visible until a waiter returns them, then prevents the same line from being returned again. Re-running `niles wait --worker <id>` after the previous wait has returned is the send -> wait loop primitive: the next sequential wait attaches normally and returns the next actionable line. Because these waits are single-consumer, an active unindexed waiter records its pid and start time in `status.waiter`; a concurrent second unindexed waiter on the same status log fails at attach instead of silently stealing a wake. Each consumed wake is also recorded in `status.ack.log`.

Indexed waits such as `niles wait <run> --index N` intentionally scan the full log for an actionable line that contains the exact token pair `step N`. Generic `done:` or `failed:` lines do not satisfy an indexed wait.

## Project Config

Niles loads the first config file it finds from:

- `niles.yaml`
- `.niles.yaml`

Project config can provide shared workspace, agent, and command defaults. Task specs are merged on top, so local workflow files can override project defaults without repeating every common value.

## Runtime Boundary

The Rust CLI owns deterministic work:

- loading project config
- loading task specs
- creating workspace-anchored run directories
- probing local agent binaries
- running single captured steps and spawning worker panes
- streaming and capturing stdout, stderr, exit codes, timestamps, and git diffs
- preflighting built-in agent versions
- persisting resumable run state

Agents own judgment-heavy work:

- planning
- implementation
- review
- handoff wording
- deciding whether a task is complete

## Step Execution

For workflow files, `niles run` only prepares run state; the foreground manager advances each step explicitly with `niles step` or `niles exec-step`.

Agent steps resolve to subprocess invocations through agent profiles. Built-in profiles are intentionally small and live in code, so agent CLI churn can be absorbed in one place. Agent references support `family:model[:effort]` qualifiers for built-in families; Niles maps Codex tiers to `--model` plus `model_reasoning_effort` config, and Claude tiers to `--model` plus `--effort`.

Interactive worker windows use worker defaults. For built-in Codex and Claude workers, those defaults bypass agent approval or sandbox prompts: Codex launches with `--dangerously-bypass-approvals-and-sandbox`, and Claude launches with `--dangerously-skip-permissions`. Niles does not enforce an approval-policy gate around worker actions. Captured agent steps use the normal agent profile defaults unless the task or project config overrides them.

Workflow files can override the binary and args for any agent when a project needs an explicit invocation. Command steps execute named commands from the task spec, which keeps shell execution explicit and auditable.

During each captured step, Niles tees stdout and stderr to the terminal and per-step log files. After each completed step, Niles captures `git diff --no-ext-diff --` from the workspace and stores it beside the step logs. This gives the manager and the user a stable artifact for review handoffs.

## Analyzer

Agent CLIs change quickly. Niles should not assume a fixed flag set forever.

The analyzer creates a local capability manifest by running safe probes such as:

- `<binary> --version`
- `<binary> --help`

`niles analyze` also probes model acceptance for requested model-qualified
agent specs, built-in model aliases, and model-qualified agents bound in the
workspace manifest when present. Capability manifests live under
`.niles/capabilities/`; default binaries keep `<family>.json`, and configured
non-default binaries use a family-and-binary-specific filename. Manifests record
accepted and rejected models with the detected CLI version and probe timestamp.
Launch validation consults fresh capability manifests first, fails before
spawning for known rejected models, and falls back to static validation when no
fresh manifest data exists.

## Manager Decisions

There is no router runtime or structured decision validator in the current CLI. The foreground manager agent decides when to call explicit Niles commands; the Rust CLI validates command arguments and task specs, then executes the requested command.
