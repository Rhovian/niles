# Niles

Niles is a Rust CLI orchestration harness for coordinating agent CLIs such as
Codex and Claude in tmux sessions. The Rust CLI owns deterministic work —
workspace manifests, tmux window placement, worker metadata, status logs,
and schema-stamped artifacts —
while agents own the judgment-heavy work: planning, implementation, review,
handoff wording, and deciding when a task is complete.

## Requirements

- A Rust toolchain with edition 2024 support (1.85+).
- `tmux` — Niles runs the lead and its workers as windows of your current tmux
  session, so `niles` must be run from inside tmux.
- The agent CLIs you intend to use, on your `PATH` (e.g. `codex`, `claude`).

## Install

```sh
cargo install --path .   # installs the `niles` binary on your PATH
cargo build --release    # or a local build at target/release/niles
```

## Checks

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --no-fail-fast
```

Run these before reporting a change done: formatting, the clippy gate across all
targets (tests included), and the test suite. Keep `--no-fail-fast`: plain `cargo test`
stops at the first failing binary, so a failure in one test file hides every later one.

## Launch

```sh
niles
```

Bare `niles` turns the current tmux pane into the lead agent. Niles never
creates, names, pins, or attaches a tmux session: run it inside the session you
are already attached to, and it fails with one line of guidance if you are not
in tmux. That is also what makes worker placement a fact rather than a
resolution strategy — workers are windows of the session you are looking at.

The launch prelude creates `.niles/worker/` and interactively ensures
`.niles/manifest.yaml` exists, prompting for the `lead` agent (defaulting to
Claude on first setup) and optionally the other role bindings.

Niles writes a lead brief under `.niles/sessions/<id>/lead.md` pointing at
the manifest and its flow. For Claude the brief is passed via
`--append-system-prompt` (hidden context); other agents receive it in their
initial prompt. Niles owns no chat grammar — the foreground agent drives the
conversation and invokes explicit Niles commands as orchestration tools.

## Worker Lifecycle

Spawn a worker agent into a tmux window:

```sh
niles spawn auth-fix --wait --agent codex "Fix the flaky login test"  # spawn, block for the report
niles send auth-fix --wait "Rerun it and report."                     # steer, block for the reply
niles peek auth-fix
niles report auth-fix
niles workers
niles close auth-fix
```

Workers always belong to the workspace the spawn ran from; to work in another
one, `cd` there first. Spawn writes a brief and launch script under `.niles/worker/<id>/`,
records tmux metadata in `.niles/worker/<id>.json`, and starts a `niles-<id>`
window in the tmux session the spawn was run from. `niles spawn` outside tmux
fails before writing anything, rather than placing a worker in a session nobody
is attached to. The normal lifecycle is `spawn -> (wait <-> send)* -> cleanup`.

`niles send` types the message into the worker's pane and then presses the
submit key, which are two separate tmux calls. It watches the pane across both:
the paste has to render and go quiet before the submit is sent, and the pane has
to change after it. A submit a busy TUI swallows leaves the message sitting
unsent in the composer, and `send` reports that as a failure rather than
printing `sent:` over it.

`--task <label>` records a task label so a task or wave can be cleaned up as a
group. Labels use the same ASCII grammar as worker ids (`A-Z`, `a-z`, `0-9`,
`_`, `-`) and reserve `archive`, which names the `.niles/worker/archive/` store
for closed workers. Close a worker with `niles close <id>`, a group with
`--task <label>`, or everything in the workspace with `--all`; batch close
reports each worker and continues past individual failures.

`niles workers` lists only workers in the current workspace and includes a
`wake` column and a window-health column. `wake` is `pending` when the worker's
status log holds an actionable line no `niles wait` has collected yet — without
it a worker that finished twenty minutes ago and one still working render the
same `done:` and the listing hides the one thing the lead needs to notice.
`agent-exited` means the agent finished or died and its
window is being kept so the pane stays readable — close it when you are done
with it. `window-dead` means the window itself is gone while worker metadata
remains: a stale directory that is a cleanup candidate.

A worker's window runs the agent as a child rather than replacing the shell
with it, so when the agent exits the script records it:

```
closed: agent exited (status 3)
```

That is an ordinary wake, so `niles wait` returns immediately rather than
blocking on a log nothing can append to again, and the pane survives for
`niles peek` — whatever killed the agent is still on it.

## Wake Contract

`niles wait` is the single wake-delivery mechanism: it prints the next
actionable line from a worker status log. `spawn --wait` and `send --wait`
are the same thing folded into the command that caused the work, so a
single-worker turn never needs a bare `wait`; reach for `niles wait <id>`
or `niles wait --task <label>` when blocking on a group you did not just
act on, which is the one thing the folded forms cannot do. The five
actionable states are `done:`, `failed:`, `blocked:`, `needs-decision:`, and
`closed:`. Workers stay warm after `done:` — it tells the lead to inspect
and optionally send follow-up, not to terminate; cleanup happens explicitly at
integration time. Each wait records a byte offset into the status log in
`.niles/worker/<id>/status.cursor` and advances it only when it delivers a
line, so each actionable line is returned exactly once. Concurrent waits on one
worker are serialised by an advisory lock on that cursor rather than rejected:
one is handed the line, the other keeps waiting.

## Nudges and Check-ins

Niles is what moves an idle lead. The foreground `niles` process runs a watcher thread for as long
as the lead does, and it types one line into the lead's own pane when there is something to look at:

```
niles: impl reported (done) — check workers
niles: no report from impl in 5m — check it
```

A nudge is state, not an event: it says where things stand and carries no status-line content, so
`niles wait` stays the only consumer of the wake cursor and a nudge that is lost, doubled or read
twice costs a look. The thread never writes to the lead's stdout or stderr — they belong to the
lead's TUI — and records what it did in `.niles/sessions/<id>/watch.log`. The pane it types into is
the one recorded in `session.json` at startup: a session with no pane (niles run outside tmux)
starts no thread and behaves exactly as it did before.

Check-ins are armed by the lead, never by the worker. `niles spawn` and `niles send` arm one at 5
minutes; `--checkin 90s`, `--checkin 5m`, `--checkin 1h` or a bare number of minutes sets another
delay, and `--checkin 0`/`--checkin off` arms none. An actionable line written after the moment of
arming disarms it, and `niles quiet <id>` disarms one by hand — for a worker that is idle on
purpose. A check-in that comes due with no report nudges and re-arms three minutes later, repeating
until someone looks. A `working:` line never disarms a check-in and never nudges: a worker looping
on progress notes cannot buy itself silence.

## Roles

Niles composes a brief per role rather than handing every agent the same one.
A worker's brief is a short shared contract — its id, its report file, and the
status lines that wake the lead — plus exactly one role fragment:

```sh
niles spawn impl --role worker   --agent codex  "Implement the fix"
niles spawn rev  --role reviewer --agent claude "Review impl's change"
niles spawn aud  --role security --agent claude "Attack impl's change"
```

- **lead** — the foreground agent. Owns the outcome *and the plan*: decides what
  gets built and how, then delegates the implementation, independent judgment on
  it, and anything needing parallelism or a fresh context. Does not implement.
- **worker** — owns the change, and owns the gate. Runs the project's build,
  tests and linters before reporting `done:`, and says what they printed.
- **reviewer** — owns correctness, idiom and economy: does it work, does it read
  like the code around it, could it have been done in less code, and are the
  tests redundant. Does not run the gate and does not write hardening findings.
- **security** — owns the adversarial pass, commissioned only when the change is
  a security boundary. Must name a reachable attacker before any finding.

Two splits do the work here. **Gate ownership**: when every agent is told the
same thing about verification, every agent runs the test suite. And **security
as its own pass**: fused into code review, it turns every small change into a
hardening exercise against an attacker nobody named.

Workspace role bindings live in `.niles/manifest.yaml`:

```yaml
lead: claude
worker: codex
reviewer: claude
security: claude
```

That is the whole manifest: which agent plays each role. Every role with its own
brief has its own binding — a security pass is commissioned rarely, but the tier
it runs at is a workspace decision rather than something the lead has to
remember per spawn.

Bindings accept built-in agent families and agents from project config; unknown
bare agent names are rejected.

## Project Config

Niles loads the first of `niles.yaml` or `.niles.yaml` for shared agent
defaults. Role bindings and flow stay in `.niles/manifest.yaml`:

```yaml
agents:
  local-reviewer:
    binary: review-agent
    args: ["--format", "plain"]
```

Niles has built-in profiles for common agents such as `codex` and `claude`, so
`binary` can be omitted for known agents. Agent references accept a
`family:model[:effort]` qualifier — for example `codex:gpt-5.5:xhigh`,
`claude:opus:max`, or `claude:sonnet:med` — in `niles spawn --agent` and
manifest role bindings.

Each family carries a roster, and the roster is the whole of what it will
launch:

| family | models |
| ------ | ------ |
| `codex` | `gpt-5.5`, `gpt-6-astra`, `gpt-5.6-sol`, `gpt-5.6-terra`, `gpt-5.6-luna` |
| `claude` | `opus`, `sonnet`, `fable`, `haiku` |
| `hermes` | `tencent/hy3`, `deepseek/deepseek-v4.1-flash`, `z-ai/glm-5.3-flash` |

A model that is not on it — a full id like `claude-opus-5`, a sibling like
`gpt-5.4`, a vendor path hermes could route — is rejected, not guessed at:

    $ niles spawn x --agent codex:gpt-5.4 "..."
    Error: unsupported codex model `gpt-5.4` in agent spec

Adding one is a line in `src/agents/families.rs` carrying the efforts that model
accepts, because effort belongs to the model rather than the family:
`codex:gpt-6-astra:ultra` is accepted where `codex:gpt-5.5:ultra` is not, and
`claude:haiku` takes no effort at all, so it rejects the qualifier outright
rather than sending a level its model cannot answer.

## Example Task

```sh
niles spawn auth-impl --task auth --agent codex:gpt-5.5:xhigh \
  "Fix the flaky auth test, then run the project's checks."
niles wait auth-impl
niles report auth-impl
niles spawn auth-rev --task auth --role reviewer --agent claude:opus:high \
  "Review auth-impl's fix. Its report says which checks it ran."
niles wait auth-rev
niles report auth-rev
niles close --task auth
```

## Status

Niles currently supports lead sessions in the current tmux pane, tmux worker
windows, workspace role manifests, worker reports, worker archives, and
worker status-log wake delivery.

## License

Dual-licensed under MIT or Apache-2.0, at your option.
