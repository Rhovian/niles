# Harness conformance

`src/harness/conformance.rs` runs the shared spec against real harness adapters in CI.
The native fixture enumerates every family through
`agents::invocation` → `launch_worker_window` in a private tmux server. It uses
`tests/harness_exec.py` to record launch argv and selected environment values.
The ACP fixture launches the production runner and uses the same one-request socket
connections as worker commands. The fake peers in `tests/harness_acp.py` exercise
Codex's launch config, Claude's session metadata and mode, and pi's native readiness
command and generated child command. Codex and Claude advertise and send auth status.
The pi-acp-shaped peer omits auth status from both initialization and notifications.
All three peers exercise resume and restart. The Claude peer also supplies the
stimuli for the shared ACP runner's history, queue, permission, and cancel checks.

Both fixtures use complete input receipts from the fake CLI or peer. The ACP peer uses
a distinct completion reason so the opening-turn assertion cannot match the Idle state
written during initialization. The fixtures require Python 3 and use no live credentials.
Installed CLI checks use `niles compat`. See
[Agent CLI compatibility](../CONTRIBUTING.md#agent-cli-compatibility).

| Guarantee | Coverage |
| --- | --- |
| First and multiline input | Shared sequence compares the exact bytes received by the CLI or ACP peer. Native also checks actual launch argv. |
| Input acknowledgment and queueing | Both return Unconfirmed for idle dispatch. ACP returns Queued while a permission is pending and sends that input after the active turn ends. |
| Quiet work | The resolver test `session_lifecycle_requires_turn_evidence_deadline_and_liveness` covers elapsed time and liveness. |
| Provider failure and quota | Existing Claude/Codex error-frame tests and pi's state-reader test cover provider failures. Distinct quota-shaped fixtures remain open. |
| Attention and response | Native refuses attention and permission dispatch. ACP surfaces a pending request and returns the selected option to the peer. |
| Cancel requested and confirmed | Native refuses cancel. ACP stays Running while the peer withholds confirmation, automatically cancels a late permission, and reports Cancelled after the peer confirms. |
| Exact resume, missing identity, and retained history | Native refuses resume. ACP loads the bound session, refuses a foreign identity and a peer-side missing session, and probes retained history after load. Replay leaves one copy in the transcript. |
| Disconnect, restart, and stale generations | The old pane or open socket connection fails after restart. ACP recovers a stale socket path, loads the bound session from the same worker directory, and receives new input without resending the opening assignment. |
