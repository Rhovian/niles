# Configuration

Reference for the workspace manifest, model registry and Niles state. The
[README](../README.md#configuration) shows where the manifest lives and a minimal example.

## Manifest keys

| Key | Where | What it does |
| --- | --- | --- |
| `entry` | top level, required | The skill you talk to. No binding is required. |
| `<skill>` | top level | Optional skill composition; no models or default tier. |
| `tiers` | top level, required | Project-wide model choices for every skill. |
| `standard`, `frontier`, `economy`, `mechanical` | `tiers` | Tiers. `standard` is required; each tier has one `primary` and an ordered `secondaries` list. |
| `skills`, `always`, `on_from`, `on_stage` | binding | Menu and injected skills; see [skill workflows](skills.md#composing-skills). |
| `gates` | binding | Stage rounds and `post` command gates; see [staging](skills.md#staging). |
| `worktree` | top level | `env` and `setup` for `spawn --worktree`; see [worktrees](#worktrees). |
| `checkin`, `recheck` | top level | Watcher cadence; see [check-ins](#check-ins). |

Agents are written `family:model:effort`, such as `claude:sonnet:high`; models without effort
support, such as `claude:haiku`, omit the suffix. Scalar bindings and `when:` groups are rejected.
For direct editing with native sub-agents, [set `entry: solo`](skills.md#entry-sessions).

`niles spawn <id> --skill <name>` uses the standard primary. `--tier <tier>` picks another listed
tier's primary, and `--agent family:model:effort` picks an exact listed primary or secondary; the
two conflict, and an unlisted tier or agent errors. Any installed skill can use any listed tier,
including skills supplied through `--menu` or `--with`. Skill bindings contain composition only;
per-skill models, default tiers and overrides are rejected. Existing manifests must be updated
before upgrading; there is no automatic migration.

## Models

The first of `niles.yaml` or `.niles.yaml` in the repository extends the built-in roster. Listing
an existing model replaces its entry, and `efforts: []` marks a model with no effort qualifier.
The optional `context_window`, in tokens, lets the telemetry panel show a percentage:

```yaml
models:
  codex:
    gpt-5.7:
      efforts: [low, medium, high, xhigh]
      context_window: 400000
```

`~/.niles/roster.yaml` replaces the full entry list for any model it names, or adds a model to a
built-in family. It loads before project overrides, and new project models need an entry here to
appear in `niles models`. The shipped list is `src/agents/roster.yaml`:

```yaml
codex:
  gpt-6-sol:
    - effort: high
      tier: standard
      description: Complex work with a balance of intelligence and cost
      cost: 2
      speed: null
      context_window: 1050000
```

Cost and speed rank from 1 (cheapest, fastest) to 4; `null` means unverified and shows as
`unknown`. Omit `effort` for a model without one. `niles models` lists only pairs some skill in
the manifest allows, or every pair when there is no manifest. Invalid overrides are errors.

### Pi models through OpenRouter

Install `@earendil-works/pi-coding-agent`, launch `pi`, and use `/login` to store your OpenRouter
key in `~/.pi/agent/auth.json`. Pi doesn't read `.env` files, and an exported
`OPENROUTER_API_KEY` reaches workers only if the tmux server started after the export.

Pi's bundled catalog lags the pi models in Niles's roster. Add them to `~/.pi/agent/models.json`,
which merges into pi's OpenRouter provider:

```json
{
  "providers": {
    "openrouter": {
      "models": [
        { "id": "deepseek/deepseek-v4.1-flash", "reasoning": true },
        { "id": "tencent/hy3", "reasoning": true },
        { "id": "z-ai/glm-5.3-flash", "reasoning": true }
      ]
    }
  }
}
```

These entries omit prices, so Niles reports their token usage without a cost estimate.

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

- `~/.niles/state/v2/<workspace key>/@state/`: the manifest, sessions, worker briefs, status
  logs and reports. Encode the entire absolute workspace path's UTF-8 bytes (including `/`)
  as lowercase hexadecimal, then split that string into directory components of at most
  240 characters. For example, `/code/my-app` becomes `v2/2f636f64652f6d792d617070/@state`,
  while `/code/my/app` becomes `v2/2f636f64652f6d792f617070/@state`. This reversible encoding
  distinguishes separators, hyphens, case and Unicode bytes even on case-insensitive filesystems.
  The reserved `@state` leaf keeps prefix keys' state directories disjoint. Individual key
  components stay below the usual 255-byte name limit; the filesystem's total path limit still
  applies. Older binaries used slash-to-hyphen keys; move existing state to the new layout
  by hand before installing this version. Niles does not migrate or read legacy state.
- `~/.niles/projects`: registered projects.
- `~/.niles/cache/`: checkouts of skills pinned with `niles skills import`, which also writes a
  `SKILL.ref` pointer under `.niles/skills/` or `~/.niles/skills/`.
- `~/.niles/config.yaml`: settings saved in the config panel.
- `~/.niles/trees/<absolute workspace path>/<branch>`, plus its Git branch, for `--worktree`
  spawns.
- `~/.niles/remote/`: push keys and subscriptions, if you use [remote access](remote.md).
- One tmux session per project, with status bar options set on that session only.

Niles reads `~/.niles/roster.yaml` and `niles.yaml` but never writes them, and it doesn't edit
your agent CLIs' or tmux configuration. To remove Niles, close its projects in the home view
(`c`), uninstall the binary, and delete `~/.niles`, any
`.niles/` directories in your repositories, and branches left by worktree spawns.
