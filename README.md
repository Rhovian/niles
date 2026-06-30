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
niles
niles --goal "Fix the flaky auth test and open a PR"
niles ask "fix the failing auth test"
niles ask -a claude "review the current diff"
niles analyze
niles doctor
niles analyze --agent codex
niles spawn auth-fix --project ../my-app --agent codex "Fix the flaky login test"
niles peek auth-fix
niles send auth-fix "Please run the auth tests again."
niles wait --crew auth-fix
niles manifest "Fix flaky auth test" --project ../my-app --planner claude --implementer codex --reviewer claude --command test
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

Bare `niles` launches the foreground supervisor agent, currently Claude by default. Niles writes a supervisor brief under `.niles/sessions/<id>/`, passes it as hidden supervisor context, and gives the foreground agent a small startup prompt so the agent greets you with useful paths: handle the task directly, create a manifest, resume existing Niles work, or spawn workers. `niles --goal ...` seeds that startup context before the agent starts. Niles does not try to be a chat grammar; the foreground agent uses the explicit Niles commands as orchestration tools.

Short aliases are part of the interface:

```sh
niles a "summarize this repo"
niles m "fix the flaky test" --project ../my-app
niles r task.yaml
niles s
niles w
niles l
niles d
niles re
```

The default path should feel like an axi: terse commands, obvious defaults, compact output, and YAML only when a workflow needs to be explicit.

## Crew

Spawn a worker agent in tmux:

```sh
niles spawn auth-fix --project ../my-app --agent codex "Fix the flaky login test"
niles peek auth-fix
niles send auth-fix "Please rerun the failing test and report the result."
niles wait --crew auth-fix
```

Spawn writes a brief and launch script under `.niles/crew/<id>/`, records tmux metadata in `.niles/crew/<id>.json`, and starts a `niles-<id>` window. If you are already inside tmux, the worker appears in your current session; otherwise Niles uses a detached `niles` session.

Worker briefs include a status file. When a worker appends `done:`, `failed:`, `blocked:`, or `needs-decision:` lines, the foreground supervisor uses `niles wait --crew <id>` to block until the next actionable wake and print it. `niles wait` is the single wake-delivery mechanism.

## Manifests

Generate a role-based workflow manifest:

```sh
niles manifest "Fix flaky auth test" \
  --project ../my-app \
  --planner claude \
  --implementer codex \
  --reviewer claude \
  --command test \
  --run
```

Niles writes the manifest to `.niles/manifests/<id>.yaml`. Without `--run`, it prints the follow-up `niles run ...` command. With `--run`, it generates the manifest and prepares a run from it. `niles run` always prepares run state only; advance work one step at a time with `niles step` for interactive agent windows or `niles exec-step` for captured command/agent steps. Project config from the target project is copied into the manifest when available, so the generated YAML is runnable and editable.

Generated manifests label each step with a role: `planner`, `implementer`, `validation`, or `reviewer`. `niles status` and `niles show` surface those roles while the supervisor drives the run.

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

Niles has built-in profiles for common agents such as `codex` and `claude`. Task files and project config can still override agents, commands, and workspace values locally when a project needs an explicit invocation.

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

Captured steps stream stdout/stderr live and also write stdout, stderr, git diff, and metadata into `.niles/runs/<id>/steps/`. Interactive agent steps run in tmux windows; close them with `niles step-close` after reviewing the worker output.

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

Niles is just starting. The first target is a sequential workflow runner with local analyzer support, persisted run logs, and public capability manifests.
