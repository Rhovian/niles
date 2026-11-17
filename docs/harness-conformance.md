# Harness conformance

`src/harness/conformance.rs` takes a fixture with a real harness Adapter. CI enumerates every
native family, using fake CLIs, a private tmux server with no host config, and the shipped
`agents::invocation` → `launch_worker_window` path. A pre-exec wrapper records actual argv and
selected public launch-env values. Opening bytes are compared with the generated invocation;
follow-ups wait for complete receipts published by the fake CLI, independently of terminal echo.
The CI argv recorder uses the live driver's exec mode and requires python3.
Startup/receipt and child-process waits have deadlines. Private servers have panic-safe cleanup.
No live CLIs or credentials are used by the CI sequence; each family records `fixture/no-auth`.

| Guarantee | Native coverage / remaining work |
| --- | --- |
| First and multiline input | Shared sequence: actual launch argv and exact fake-CLI follow-up receipt |
| Accepted versus queued; busy input | Native returns Unconfirmed. No busy/queue acknowledgment assertion; needs a real busy peer in #533 |
| Long quiet work | Existing `session_lifecycle_requires_turn_evidence_deadline_and_liveness` exercises production `resolve_session` at a later clock time; not repeated per native adapter |
| Provider failure | Existing Claude/Codex error-frame tests and pi's state-reader test; not synthetic fixture observations |
| Quota classification | No distinct quota-shaped per-family fixtures yet; unverified. A renamed generic error string is not quota evidence |
| Attention without/with response | Native refuses attention/permission dispatch. Supported protocol lifecycle cases await #533 |
| Cancel requested versus confirmed | Native refuses cancel. Existing v1 stop-reason table checks confirmation; supported transport cases await #533 |
| Exact resume / missing identity | Native refuses resume. Supported branch loads the requested session and refuses an identity outside the bound worker session |
| Disconnect, restart, stale generations | Shared sequence: disconnected and retired pane handles fail, replacement receives input. Native has no event stream; no stale-event injection claim |

The shared ACP fixture follow-up must extend this sequence with protocol-peer stimuli and observations from the production
runner/adapter for quiet work, busy acknowledgments, attention, cancel and stale wire events.
Resume stays bound to the worker's existing conversation: foreign identities are refused locally,
and a missing bound identity must be refused by session/load, never silently create a new conversation.
Successful load alone does not prove conversation-history continuity;
a protocol fixture must also probe retained history after resume.

## Live compatibility procedure

Run only with the operator's authorization and each CLI's own login. The driver records
argv before exec; model-written receipts are output evidence, not proof of opening bytes.
It never copies or exports credentials. Its `prepare` mode starts no process.
The native live rerun covers the checkpoints below; the shared ACP fixture is a separate follow-up.

1. Prepare a disposable workspace with an installed skill and Niles manifest,
   approved auth/config, and pinned CLIs from `src/agents/families.rs`. Build the Niles binary
   from the reviewed commit. Keep evidence outside temporary directories; attach it to the PR
   or issue. Record the auth path as a label, never a credential. Choose only public env flag
   names for `environment_keys`; the wrapper records those values, not the full environment.
2. Write a JSON plan with `workspace`, absolute `niles`/`cli` paths, `pin`, `agent`, `skill`,
   `auth_path`, optional `environment_keys`, and `steps`. Each step has a `case`, an `action`
   (`spawn`, `input`, `capture`, `disconnect`), and `text` for spawn/input. Example steps:
   `[{"case":"opening","action":"spawn","text":"Return a unique sample"},
   {"case":"multiline","action":"input","text":"first line\nsecond line"},
   {"case":"disconnect","action":"disconnect"},
   {"case":"restart","action":"spawn","text":"Return a new sample"}]`.
3. `python3 tests/harness_live.py prepare plan.json /persistent/evidence/family` prepares evidence
   only. Once authorized, use `run` with a fresh evidence directory. It calls **niles spawn/send**,
   so model/effort validation, primary skill, session flags, launch env and brief delivery use
   shipped code. It records argv before exec and compares its final argument with Niles's brief
   using the launcher's final-newline convention. Model-written files are output evidence only.
4. Inspect the private tmux session printed by the driver. Each operator checkpoint saves a
   case-labeled result, pane capture, and real `niles workers` lifecycle snapshot. Arrange silent
   foreground work beyond the check-in interval, capture while still running, then send busy
   multiline input. Disconnect during work and spawn a replacement; old-handle refusal is recorded.
   Collect CLI transcripts for received follow-up text; do not infer accepted/queued from `sent:`.
5. Evidence includes the plan, argv/env record, generated brief/launch/meta files, per-case command
   output and snapshots, versions before/after, auth label, and Niles binary SHA256. Records start
   as `needs_review`; mark pass/refusal/untested with evidence paths after inspection. Keep failure
   and quota tests fake-only; do not force subscription failures. Quota-shaped fixtures, supported
   ACP lifecycle, history continuity and live auth remain explicit gaps until separately verified.
