# Configuration

Reference for the workspace manifest, CLI model discovery and Niles state. The
[README](../README.md#configuration) shows where the manifest lives and a minimal example.

## Manifest keys

| Key | Where | What it does |
| --- | --- | --- |
| `entry` | top level, required | The skill you talk to. No binding is required. |
| `entry_agent` | top level, required | The agent the entry session runs on; must be a listed tier agent. `niles start --agent` overrides it for one launch. |
| `<skill>` | top level | Optional skill composition; no models or default tier. |
| `tiers` | top level, required | Project-wide model choices for every skill. |
| `standard`, `frontier`, `economy`, `mechanical` | `tiers` | Tiers. `standard` is required; each tier has one `primary` and an ordered `secondaries` list. |
| `always`, `on_stage` | binding | Injected and stage skills; see [skill workflows](skills.md#composing-skills). |
| `gates` | binding | Stage rounds and `post` command gates; see [staging](skills.md#staging). |
| `worktree` | top level | `env` and `setup` for `spawn --worktree`; see [worktrees](#worktrees). |
| `checkin`, `recheck` | top level | Watcher cadence; see [check-ins](#check-ins). |

Tier agents may name a family alone, a model, or a model with an effort. Scalar bindings and
`when:` groups are rejected.
For direct editing with native sub-agents, [set `entry: solo`](skills.md#entry-sessions).

`niles spawn <id> --skill <name>` uses the standard primary. `--tier <tier>` picks another listed
tier's primary, and `--agent family:model:effort` picks an exact listed primary or secondary; the
two conflict, and an unlisted tier or agent errors. Any installed skill can use any listed tier,
including skills supplied through `--with`. Skill bindings contain composition only;
per-skill models, default tiers and overrides are rejected. Existing manifests must be updated
before upgrading; there is no automatic migration.

## Models

`niles models`, the model picker and the config panel read available models and efforts from
installed agent CLIs. Codex supplies `debug models`, Claude supplies its stream-JSON initialize
response, and Pi supplies `get_available_models` in RPC mode. Context windows come from session
transcripts, falling back to the CLI's catalogue when available.

Agent specs use `family[:model[:effort]]`. Omit the model and effort to use the CLI's default.
For a model ID containing colons, append `:` when omitting the effort, as in
`pi:provider/model:variant:`. The final colon separates the model from its optional effort.
Niles passes model and effort values unchanged to the CLI, which reports unsupported values at
launch. Manifest tiers still restrict which agent specs workers may launch. Onboarding starts
with each installed family's CLI defaults in the standard tier; choose other tiers yourself.

Model configuration belongs to each agent CLI. The old `niles.yaml` and `.niles.yaml` files are
no longer read. `niles models` lists the available models independently of manifest tier assignments.
Its columns are `family`, `model`, `effort` and `context_window`; an unavailable context window
is `unknown`.

A failed CLI is reported for its family while working catalogues remain available. The picker
shows the failure and disables that family's choices. The config panel retains the manifest and
other model families. Telemetry shows capacity lookup failures in CONTEXT while retaining token
and event data. `niles models` prints working families and exits nonzero with the failed-family
diagnostics on stderr.

An open view caches catalogue results. Restart Niles after changing CLI model configuration or
fixing a discovery error.

### Pi models through OpenRouter

Install `@earendil-works/pi-coding-agent`, launch `pi`, and use `/login` to store your OpenRouter
key in `~/.pi/agent/auth.json`. Pi doesn't read `.env` files, and an exported
`OPENROUTER_API_KEY` reaches workers only if the tmux server started after the export.

## Worktrees

```yaml
worktree:
  env:
    CARGO_TARGET_DIR: /absolute/path/to/main/target
  setup: 'test -d "$CARGO_TARGET_DIR"'
```

`env` sets literal environment variables for workers spawned with `--worktree`, including when
joining an existing tree. `$HOME`, `~` and shell substitutions aren't expanded, so use absolute
paths. Pointing `CARGO_TARGET_DIR` at the main checkout reuses its build cache.

`setup` runs once through `sh -c` in each newly created tree, before the worker starts, with
stdin closed; use it for steps such as `uv sync` or `pnpm install`. If it fails, the spawn fails
and the new tree is removed, so a retry runs setup again; the branch stays, so retry without
`--base`. If an interrupted setup blocks the branch, the error names the `.pending` file to
remove after cleaning up the tree. Setup runs with your permissions, so only put commands here
you would run yourself.

## Check-ins

`checkin` sets when the watcher first reminds the entry agent about a quiet worker (default
`5m`); `recheck` sets the interval after that, either a duration or `backoff` (the default,
growing to hourly). Durations are an integer with `ms`, `s`, `m` or `h`. `checkin: 0` or `off`
disables check-ins, and a fixed `recheck` must be above zero. `--checkin` on a command overrides the manifest.

## State layout

- `~/.niles/state/<workspace key>/@state/`: the manifest, sessions, worker briefs, status
  logs and reports. The workspace key is the full SHA-256 digest of the absolute workspace
  path's UTF-8 bytes, formatted as 64 lowercase hexadecimal characters. This bounds the key
  while distinguishing separators, hyphens, case and Unicode bytes. The `@state` leaf keeps
  state separate from keys. Existing state is not carried over on upgrade; Niles does not
  migrate or look up old keys.
- `~/.niles/projects`: registered projects.
- `~/.niles/cache/`: checkouts of skills pinned with `niles skills import`, which also writes a
  `SKILL.ref` pointer under `.niles/skills/` or `~/.niles/skills/`.
- `~/.niles/config.yaml`: settings saved in the config panel.
- `~/.niles/trees/<absolute workspace path>/<branch>`, plus its Git branch, for `--worktree`
  spawns.
- `~/.niles/remote/`: push keys and subscriptions, if you use [remote access](remote.md).
- One tmux session per project, with status bar options set on that session only.

Niles doesn't edit your agent CLIs' or tmux configuration. To remove Niles, close its projects in the home view
(`c`), uninstall the binary, and delete `~/.niles`, any
`.niles/` directories in your repositories, and branches left by worktree spawns.
