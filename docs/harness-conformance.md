# Harness conformance

`src/harness/conformance.rs` takes a fixture with a real harness Adapter. CI enumerates every
native family, using fake CLIs, a private tmux server with no host config, and the shipped
`agents::invocation` → `launch_worker_window` path. A pre-exec wrapper records actual argv and
selected public launch-env values. Opening bytes are compared with the generated invocation;
follow-ups wait for complete receipts published by the fake CLI, independently of terminal echo.
The CI argv recorder is `tests/harness_exec.py` and requires python3.
Startup/receipt and child-process waits have deadlines. Private servers have panic-safe cleanup.
No live CLIs or credentials are used by the CI sequence; each family records `fixture/no-auth`.
Installed CLI checks use `niles compat`; see
[Agent CLI compatibility](../CONTRIBUTING.md#agent-cli-compatibility).

| Guarantee | Native coverage / remaining work |
| --- | --- |
| First and multiline input | Shared sequence: actual launch argv and exact fake-CLI follow-up receipt |
| Accepted versus queued; busy input | Native returns Unconfirmed. No busy/queue acknowledgment assertion; needs the ACP peer fixture in #574 |
| Long quiet work | Existing `session_lifecycle_requires_turn_evidence_deadline_and_liveness` exercises production `resolve_session` at a later clock time; not repeated per native adapter |
| Provider failure | Existing Claude/Codex error-frame tests and pi's state-reader test; not synthetic fixture observations |
| Quota classification | No distinct quota-shaped per-family fixtures yet; unverified. A renamed generic error string is not quota evidence |
| Attention without/with response | Native refuses attention/permission dispatch. Supported protocol lifecycle cases await #574 |
| Cancel requested versus confirmed | Native refuses cancel. Existing v1 stop-reason table checks confirmation; supported transport cases await #574 |
| Exact resume / missing identity | Native refuses resume. Supported branch loads the requested session and refuses an identity outside the bound worker session |
| Disconnect, restart, stale generations | Shared sequence: disconnected and retired pane handles fail, replacement receives input. Native has no event stream; no stale-event injection claim |

The shared ACP fixture follow-up ([#574](https://github.com/Rhovian/niles/issues/574)) must extend this
sequence with protocol-peer stimuli and observations from the production runner/adapter for quiet
work, busy acknowledgments, attention, cancel and stale wire events.
Resume stays bound to the worker's existing conversation: foreign identities are refused locally,
and a missing bound identity must be refused by session/load, never silently create a new conversation.
Successful load alone does not prove conversation-history continuity;
a protocol fixture must also probe retained history after resume.
