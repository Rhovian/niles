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

The main fast path is:

```sh
niles ask "fix the failing test"
```

Explicit workflow files remain the durable automation path:

```sh
niles run task.yaml
```

Niles can also generate an explicit workflow manifest from role flags:

```sh
niles manifest "Fix flaky auth test" --project ../my-app --planner claude --implementer codex --reviewer claude --command test
```

Adding `--run` executes the generated manifest immediately through the same runner path as `niles run`.

Generated manifests label steps with roles. Roles are persisted in run state and shown by inspection commands so future tmux or TUI views can group work by planner, implementer, reviewer, and validation surfaces without changing execution semantics.

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

Agent steps resolve to subprocess invocations. Built-in defaults are intentionally small:

- `codex` -> `codex exec <prompt>`
- `claude` -> `claude -p <prompt>`

Workflow files can override the binary and args for any agent. Command steps execute named commands from the task spec, which keeps shell execution explicit and auditable.

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
