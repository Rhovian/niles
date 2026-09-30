# Configuration

Niles has two configuration layers. `.niles/manifest.yaml` binds workspace roles and orchestration
policy. The first present project file, `niles.yaml` or `.niles.yaml`, defines custom agent
executables. Repository configuration is executable selection input; inspect it before launch and
read the [threat model](security.md).

## Workspace manifest

Bare `niles` interactively creates the manifest and prompts for all four role bindings; on later
runs it selects the foreground lead and offers to update the other roles. There is no `niles init`.
Unknown fields are rejected. A complete valid example is:

```yaml
lead: claude:opus:max
worker: codex:gpt-5.5:xhigh
reviewer: claude:sonnet:high
security: claude:opus:max
worker_planning:
  codex:gpt-5.5: &detailed |
    Settle the implementation approach and edge cases. Decompose the work into
    concrete changes with explicit acceptance criteria.
  codex:gpt-6-astra: &frontier |
    Supply the objective, constraints, and explicit acceptance criteria. Leave
    decomposition and implementation details to the worker.
  codex:gpt-5.6-sol: *frontier
  codex:gpt-5.6-terra: *detailed
  codex:gpt-5.6-luna: *detailed
  claude:opus: *frontier
  claude:sonnet: *detailed
  claude:fable: *frontier
  claude:haiku: *detailed
  hermes:tencent/hy3: *detailed
  hermes:deepseek/deepseek-v4.1-flash: *detailed
  hermes:z-ai/glm-5.3-flash: *detailed
checkin: 15m
recheck: backoff
niles_schema: 2
```

Every role is required. `spawn` uses the binding for its `--role` (`worker` by default), while an
explicit `--agent` overrides it. Without either a usable binding or override, spawn fails.

`worker_planning` maps an exact `family:model`—never effort—to free-form implementation-planning
guidance. The [lead brief](../src/templates/lead_brief.md) instructs the lead to read the manifest
and apply a matching entry when planning a worker assignment; the guidance itself is not injected
into the brief. The sample text is operator policy, not Niles's assessment of a model. Missing keys
and model-less worker bindings add no special guidance.

`checkin` accepts `90s`, `5m`, `1h`, bare minutes, or `0`/`off`; absent means five minutes.
`recheck` accepts `backoff` or a fixed delay such as `10m`; absent means backoff. A dispatch flag
wins over `checkin`, which wins over the default. Both keys are workspace-wide—a role does not
carry its own cadence—and `recheck` controls what happens after any check-in fires. Invalid values
name the manifest and fail before dispatch.

## Built-in families and rosters

Agent references use `family:model[:effort]`. Each built-in roster is exhaustive: unsupported
models are rejected instead of guessed. Effort belongs to the selected model; `med` normalizes to
`medium`.

```console
$ niles spawn x --agent codex:gpt-5.4 "..."
Error: unsupported codex model `gpt-5.4` in agent spec
```

| Family | Model | Efforts |
| --- | --- | --- |
| `codex` | `gpt-5.5` | `low`, `medium`, `high`, `xhigh` |
| `codex` | `gpt-6-astra`, `gpt-5.6-sol`, `gpt-5.6-terra` | `low`, `medium`, `high`, `xhigh`, `max`, `ultra` |
| `codex` | `gpt-5.6-luna` | `low`, `medium`, `high`, `xhigh`, `max` |
| `claude` | `opus`, `sonnet`, `fable` | `low`, `medium`, `high`, `xhigh`, `max` |
| `claude` | `haiku` | none (omit the qualifier) |
| `hermes` | `tencent/hy3`, `deepseek/deepseek-v4.1-flash`, `z-ai/glm-5.3-flash` | `none`, `minimal`, `low`, `medium`, `high`, `xhigh`, `max`, `ultra` |

Adding a built-in model is a line in
[`src/agents/families.rs`](../src/agents/families.rs) carrying the efforts that model accepts.
The interactive manifest picker rejects unknown bare agent names unless they have a project
configuration entry.

Built-ins provide their binary and argument conventions. Worker launches deliberately use
Codex `--dangerously-bypass-approvals-and-sandbox`, Claude `--dangerously-skip-permissions`, or
Hermes `chat --yolo`. Foreground defaults differ. Niles does not add a sandbox; access is whatever
the chosen CLI, credentials, inherited environment, and OS permit.

## Project agent configuration

Custom agents live in `niles.yaml` or, if that does not exist, `.niles.yaml`:

```yaml
agents:
  local-reviewer:
    binary: review-agent
    args: ["--format", "plain"]
    prompt: stdin
```

`binary` is optional: Niles uses a built-in profile's executable when one exists, otherwise the
agent name itself. An empty `args` list preserves built-in defaults (custom agents have none); a
non-empty list replaces them, after which model and effort flags are appended. `prompt` is `arg` by
default or `stdin`; project configuration cannot express the built-ins' by-path or system-prompt
delivery modes. A manifest binding can name a custom entry such as `reviewer: local-reviewer`.

Project configuration can replace executable arguments, so shell quoting is launch integrity, not
a trust decision. YAML parsing does not execute hooks, but starting the selected agent does execute
the configured binary. Built-in model roster validation does not apply to arbitrary custom agents.
