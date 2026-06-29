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

The main fast path launches the foreground supervisor agent:

```sh
niles
niles --goal "Fix the flaky auth test and open a PR"
```

Niles writes a supervisor brief under `.niles/sessions/<id>/supervisor.md` before launching the agent. For Claude, that brief is passed with `--append-system-prompt`, so the raw brief is not injected as a visible user message. Niles then gives Claude a small startup prompt that causes the foreground agent to greet the user and offer the useful paths. `--goal` seeds the startup context before the foreground agent starts. Niles does not own a natural-language command grammar. The foreground agent owns the conversation; Niles provides orchestration commands that agent can invoke.

The one-off batch path is:

```sh
niles ask "fix the failing test"
```

Worker agents can also be spawned into tmux windows:

```sh
niles spawn auth-fix --project ../my-app --agent codex "Fix the flaky login test"
niles peek auth-fix
niles send auth-fix "Run the auth tests again."
niles watch-crew --once
```

Worker briefs include a status-file wake contract. The first watcher slice polls those status files and injects actionable `done`, `failed`, `blocked`, and `needs-decision` lines into the latest foreground supervisor tmux pane. This is intentionally smaller than firstmate's watcher: it does not yet classify stale panes, inspect CI, batch digests, or manage a durable wake daemon.

Explicit workflow files remain the durable automation path:

```sh
niles run --watch task.yaml
```

Niles can also generate an explicit workflow manifest from role flags:

```sh
niles manifest "Fix flaky auth test" --project ../my-app --planner claude --implementer codex --reviewer claude --command test
```

Adding `--run` executes the generated manifest immediately through the same runner path as `niles run`. Adding `--watch` to either execution path prints compact state snapshots inline at step transitions, avoiding the need for a second terminal during a product trial.

Generated manifests label steps with roles. Roles are persisted in run state and shown by inspection commands so future tmux or TUI views can group work by planner, implementer, reviewer, and validation surfaces without changing execution semantics.

Run state is initialized with every planned step as pending. Niles marks the active step running before launching its subprocess, then updates that same record to completed or failed. This gives future tmux views a reliable state source instead of inferring progress from panes or logs.

`niles watch` is a thin live view over that same state file. It keeps the initial milestone usable in a single terminal while preserving the later tmux direction: panes can own process display, and the state file can still drive summaries, role grouping, and completion detection.

`niles resume` reloads the original task file for a persisted run, verifies that the step shape still matches the saved run state, keeps completed steps, resets the first incomplete step and later steps, and continues through the same execution path. This is intentionally narrower than full checkpointing, but it is enough to recover from failed validation or interrupted agent steps during early product trials.

Before launching an agent step, Niles writes a per-step markdown context artifact. The context file includes the goal, current role/task, prior step summary, prior agent stdout/stderr, validation stdout/stderr, and the latest captured diff. Niles records that path on the step state before the process starts and appends the absolute path to the agent prompt.

This keeps handoffs explicit without expanding the manifest format. A future tmux session can show each role in its own pane, while the durable context artifact remains the contract between planner, implementer, reviewer, and validation steps.

## Project Config

Niles loads the first config file it finds from:

- `niles.yaml`
- `.niles.yaml`

Project config can provide shared workspace, agent, and command defaults. Task specs are merged on top, so local workflow files can override project defaults without repeating every common value.

## Runtime Boundary

The Rust CLI owns deterministic work:

- loading project config
- loading task specs
- creating run directories
- probing local agent binaries
- running subprocesses
- streaming and capturing stdout, stderr, exit codes, timestamps, and git diffs
- enforcing approval and command policies
- persisting resumable run state

Agents own judgment-heavy work:

- planning
- implementation
- review
- handoff wording
- deciding whether a task is complete

## Step Execution

The fixed-step runner is the first useful execution mode.

Agent steps resolve to subprocess invocations through agent profiles. Built-in profiles are intentionally small and live in code, not in the manifest format or public docs, so agent CLI churn can be absorbed in one place.

Workflow files can override the binary and args for any agent when a project needs an explicit invocation. Command steps execute named commands from the task spec, which keeps shell execution explicit and auditable.

During each step, Niles tees stdout and stderr to the terminal and per-step log files. After each step, Niles captures `git diff --no-ext-diff --` from the workspace and stores it beside the step logs. This gives the future router and the user a stable artifact for review handoffs.

## Analyzer

Agent CLIs change quickly. Niles should not assume a fixed flag set forever.

The analyzer creates a local capability manifest by running safe probes such as:

- `<binary> --version`
- `<binary> --help`

Remote/model probes should be opt-in because they may require authentication, network access, or paid tokens.

## Router Contract

Router decisions should be structured JSON:

```json
{
  "status": "continue",
  "next_agent": "codex",
  "phase": "implementation",
  "handoff": "Implement the route described in the prior plan.",
  "context": {
    "include_diff": true,
    "include_test_output": false
  },
  "after": {
    "run": ["test"],
    "return_to": "router"
  }
}
```

Niles validates the decision before executing anything.
