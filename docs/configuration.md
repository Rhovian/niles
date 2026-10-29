# Configuration

Reference for the workspace manifest and model registry. The [README](../README.md#configuration)
shows where the manifest lives and a minimal example.

## Bindings and tiers

Only the entry skill must be bound. `entry:` is required; other bindings are optional.
For direct editing with native sub-agents, [bind `solo` and set `entry: solo`](skills.md#entry-sessions).
Each binding requires a `standard` tier and may list `frontier`, `economy`,
or `mechanical`. Every tier has one `primary` and an ordered `secondaries` list of exact
`family:model:effort` entries. Models without effort support, such as `claude:haiku`, omit
that suffix. Scalar bindings and `when:` groups are rejected.

`niles spawn <id> --skill <name>` uses its standard primary; `--tier <tier>` selects another listed
tier's primary. An unlisted tier errors. `--agent family:model:effort` selects an exact listed
primary or secondary and conflicts with `--tier`. The lead starts at its standard primary,
looks up metadata with `niles models`, and proposes tier or secondary changes with the reason
and cost difference, waiting for the operator's yes. It proposes economical step-downs too.

## Models

The first of `niles.yaml` or `.niles.yaml` extends the model roster for the built-in agent families:

```yaml
models:
  codex:
    gpt-5.7:
      efforts: [low, med, high, xhigh]
      context_window: 400000
```

Model entries extend the built-in roster; listing an existing model replaces its entry.
An empty `efforts: []` marks a model that takes no effort qualifier. The optional
`context_window`, in tokens, is what the telemetry panel measures a live session's context
against; without one, the panel shows the session's tokens and no percentage.

The shipped `src/agents/roster.yaml` registers each effort's tier, description, relative cost,
relative speed, and context window. `~/.niles/roster.yaml` replaces the complete entry list for
any model it names, or adds a model within a built-in family:

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

Tiers are `frontier`, `standard`, `economy`, and `mechanical`. Omit `effort` for a model that takes
no effort flag. Cost ranks run from 1 (cheapest) to 4 (most expensive); speed ranks from 1
(fastest) to 4 (slowest). Ranks are model-level proxies; `null` represents unverified metadata.
`niles models` displays unverified ranks as `unknown` and lists only pairs allowed by at least
one workspace skill. Without a workspace manifest it lists all effective pairs. User overrides
load before project `models:` overrides; new workspace pairs need user-registry metadata to be
listed. Invalid registry overrides error.

## Worktrees

A manifest can configure managed worktrees without tying Niles to a build tool:

```yaml
worktree:
  env:
    CARGO_TARGET_DIR: /absolute/path/to/main/target
  setup: 'test -d "$CARGO_TARGET_DIR"'
```

`env` overrides the inherited worker environment for `spawn --worktree`, including when joining
an existing tree. Values are literal strings: use absolute paths, since `$HOME`, `~`, and shell
substitutions are not expanded. Sharing the main checkout's Cargo target directory reuses its
warm dependency cache; concurrent Cargo builds serialize on Cargo's lock. Niles sets no
language-specific defaults.

Optional `setup` runs synchronously via `sh -c` in a newly created tree, with the same
configured environment, before launching the worker. Use it for project preparation such as
`uv sync` or `pnpm install`. It runs only on creation, never on reuse. Stdin is `/dev/null` (EOF),
so setup must be non-interactive; both output streams go to Niles's stderr, keeping spawn stdout
reserved for its result. A nonzero exit fails the spawn and removes the new tree (including
setup-created files), so a retry runs setup again. The branch remains; omit `--base` when
retrying. Neither setting applies to plain spawns or `--tree`.

Concurrent creation/join of the same branch fails while creation or setup holds its lock. Niles
records pending creation before adding the tree, so failed Git creation, interrupted setup, or
failed cleanup block subsequent joins, even if setup later finishes in an orphan process. Setup
is never rerun on that existing tree. To recover, stop any remaining setup process, inspect/save
needed files, remove the incomplete worktree with Git, then remove the `.pending` file named in
the error (`~/.niles/state/<workspace-key>/worktrees/<branch>/.pending`). Recreate without
`--base` if the branch already exists. Keep the sibling `.lock` file; removing a live lock can
break exclusion. Trees created before this tracking existed are treated as existing trees and
skip setup.

Only put trusted commands in this operator-owned manifest: setup runs with your permissions.

## Check-ins

Optional manifest keys include `checkin` / `recheck` for watcher cadence. Check-ins default
to five minutes, then back off to hourly reminders. Both take a duration: a non-negative integer
followed by `ms`, `s`, `m`, or `h`, such as `90s` or `5m`. `0` and `off` disable a check-in, a
fixed recheck must be greater than zero, and `recheck: backoff` selects backoff. Per-command
`--checkin` overrides the manifest.

## Waiting for workers

`niles wait` covers every live worker in the current workspace, including workers spawned
while it runs. It returns immediately with all pending reports, one escaped
`<id> <kind>: <detail>` line per worker (at most 4096 bytes each). Run one wait, act on
the batch, then wait again; reports accumulate while you act, with no batching timer.
Status logs must be appended to (`>>`). An unsupported in-place truncate and rewrite that
regrows to the cached scan offset or beyond between polls can skip the rewritten prefix;
normal appends and respawns use the supported delivery path.
Unreadable or corrupt worker state, probe errors, and failures to append a generated wake stop
the workspace wait with an error on stderr. Repair the reported state and run wait again;
workers whose lines were not printed keep their cursors. Any lines already printed on stdout
stand as delivered even if a later worker causes a nonzero exit; process those lines before retrying.
A newer wait supersedes the old one. An unreported worker whose window or agent exits
produces `<id> gone: ...`; a worker confirmed idle for 30 seconds without reporting does too.
Gone lines use the same log/cursor delivery path as reports, and reported workers remain quiet.
A kill after printing but before advancing the cursor may replay that printed line. Exit codes: 0 wakes (reports or `gone:`), 11 superseded, 22 timeout.
`spawn --wait` and `send --wait` start this same workspace wait after dispatch.
The watcher also combines pending reports and overdue check-ins in one `niles:` line;
while a wait runs, it leaves report delivery to that wait.
