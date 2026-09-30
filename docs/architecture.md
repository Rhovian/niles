# Architecture

Niles separates deterministic orchestration from agent judgment. The Rust process owns workspace
discovery, manifests, tmux targets, artifacts, wake delivery, and cleanup. The foreground lead owns
the plan and integration; spawned agents implement or review it. Niles supplies a brief and
explicit commands, not a chat protocol.

## Lead session

Bare `niles` must run interactively inside an existing tmux session. It does not create, name,
attach, or pin a session. Before launch it creates `.niles/worker/`, interactively creates or
updates `.niles/manifest.yaml`, and asks which configured agent is the lead. It then writes
`.niles/sessions/<session-id>/lead.md` and `session.json`; `sessions/latest` points at that session.
Claude receives its lead brief with `--append-system-prompt`; other built-ins receive the brief in
their opening prompt. Using the current tmux session makes worker placement a fact rather than a
resolution strategy: workers appear in the session the operator is already looking at.

The foreground process owns a watcher thread for exactly the lead agent's lifetime. The watcher
uses the lead pane recorded in `session.json`, never writes into the lead TUI's stdout or stderr,
and records diagnostics in `.niles/sessions/<session-id>/watch.log`. On exit it asks the watcher to
stop and bounds the join, so a tmux send cannot indefinitely delay shutdown.

## Worker lifecycle and artifacts

The normal lifecycle is `spawn -> (wait <-> send)* -> close`. A spawn belongs to the workspace it
ran from and creates a `niles-<id>` window in the caller's current tmux session. Outside tmux it
fails before writing worker state rather than placing a worker in a session nobody is attached to.

Each live worker is a directory, not a sibling JSON file:

```text
.niles/worker/<id>/
  brief.md       generated assignment and reporting contract
  launch.sh      quoted agent invocation
  meta.json      agent, project, task label, and exact tmux target
  report.md      durable handoff written by the agent
  status.log     append-only status lines
  status.cursor  byte offset of the last delivered wake, created on demand
  checkin        optional watcher cadence state
```

Worker IDs and task labels allow only ASCII letters, digits, `_`, and `-`; `archive` is reserved.
Labels group a wave for `wait --task` and `close --task`. Close first appends a `closed:` sentinel,
tries to capture up to 2,000 pane-history lines, closes the exact recorded window when safe, and
moves the directory to `.niles/worker/archive/<id>-<timestamp>/`. Batch cleanup continues after an
individual failure. `report` can read the newest archive after close.

The launch script runs the agent as a child instead of replacing its shell. When the child exits,
the script appends `closed: agent exited (status N)` and tmux keeps the pane readable. Thus a lead
wakes instead of waiting forever, and can still use `peek`. `workers` distinguishes `agent-exited`,
`window-dead`, and unreadable metadata; its `wake` column says `pending` only when an actionable
line has not yet been consumed. That column distinguishes a `done:` waiting for collection from an
old `done:` whose worker is still working. `agent-exited` keeps the pane readable, so close it when
you are done with it; `window-dead` leaves stale metadata and is a cleanup candidate.

## Sending and confirmation

`send` advances the cursor past pre-existing actionable lines (printing those it skipped), records
the status-log length, and then types the message. Pasting and pressing submit are separate tmux
operations: Niles waits for the paste to render and settle before submitting, then requires the
pane to change. If a busy TUI swallows submit and leaves text in the composer, the command fails
instead of falsely printing `sent:`. This confirms visible pane behavior, not that the agent
understood or completed the request.

## Wake delivery

The actionable states are `done:`, `failed:`, `blocked:`, `needs-decision:`, and Niles-owned
`closed:`. `working:` is durable progress but does not wake the lead. `done:` is a handoff, not a
request to terminate; the lead may inspect, send follow-up, and close only at integration time.

`wait` is the sole wake consumer. Under the cursor lock, it selects one complete actionable line
after the byte offset in `status.cursor` and advances the cursor before returning that line for
output. `spawn --wait` and `send --wait` use the same mechanism. If the status log is shorter than
the current scanned offset, scanning resets to its start. A worker whose window is confirmed gone
returns only after queued status lines have been delivered.

Concurrent waits are supported. An exclusive advisory `flock` on `status.cursor` covers cursor
read, log scan, and cursor advance; one waiter receives a line while another continues waiting,
and the kernel releases the lock if a waiter dies. Final opens of both cursor and status log use
`O_NOFOLLOW`. These are narrow local-state protections, not a sandbox; see the
[threat model](security.md).

## Watcher, nudges, and check-ins

An idle agent TUI only moves when something types into it, so the watcher sends a short lead-pane
nudge when a worker reports or misses a check-in. A nudge is state, not an event: it names the
worker and wake kind, includes no status text, and never consumes the cursor. If delivery fails,
the unchanged state is retried on a later tick. A nudge that is lost, doubled, or read twice merely
costs a look. The lead sees lines such as:

```text
niles: impl reported (done) — check workers
niles: no report from impl in 5m — check it
```

Check-ins are armed by the lead, never by the worker: `spawn` and `send` arm one, by default at five
minutes. The baseline is the status-log length immediately before dispatch, so a report arriving
during launch or send counts as the answer. A later actionable line disarms it; `working:` does not,
because a worker looping on progress notes cannot buy itself silence. `quiet` disarms a worker that
is idle intentionally. The default recheck schedule doubles the previous gap to an hour and then
stays hourly (`5m`, `10m`, `20m`, `40m`, `1h`, `1h`): a worker still silent an hour in wants a
look, not a nudge every three minutes. A manifest may instead choose a fixed gap. A per-command
`--checkin` overrides manifest `checkin`, which overrides the five-minute default; `0` or `off`
disables and removes an existing arm. The manifest's `recheck` controls subsequent fires. Invalid
values fail before dispatch.

Check-in state carries its original policy so later manifest edits do not retime in-flight work.
Before disarming or rearming, the watcher re-reads the file and changes it only if it still matches
the state it planned against. A partial or unreadable file is retried rather than guessed at.

For the first 30 seconds of a new worker with an empty status log, the watcher recognizes bounded,
exact-workspace Codex and Claude trust-prompt shapes. It appends one `blocked:` wake naming the pane
but never answers the prompt or sends a key. Any status-log content ends inspection. This is
limited detection, not general prompt-injection protection.
