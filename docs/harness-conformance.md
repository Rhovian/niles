# Harness conformance

Every adapter uses the acceptance runner in `src/harness/conformance.rs`. Its `Fixture`
creates an adapter and controls the outside CLI or protocol peer. `received` reads the peer's
receipt; `observe` reads the adapter's lifecycle evidence at the supplied clock time. Neither
may manufacture the expected answer. Add an ACP fixture here when its runner lands (#533);
do not duplicate the assertions in an adapter-specific suite.

CI runs `native_families` through the ordinary Rust test gate. It enumerates the family
registry, starts a private tmux server and runs fake CLIs through the shipped worker launch
script and `NativeTui`. A subprocess isolates PATH and TMUX from concurrent unit tests.
Receipts publish atomically after the CLI reads each input; consuming one waits for that
publication, independently of terminal echo. Startup/receipt waits and the child process have
deadlines, so a crashed fake fails the test. No installed agent CLI or credentials are used.
Each family prints its adapter, protocol, CLI and auth path (`fixture/no-auth`). Transcript stimuli use each family's real reader.

The shared sequence covers:

| Input or event | Required observation |
| --- | --- |
| First input with newlines, quotes and Unicode | Exact opening argument at the CLI |
| Multiline follow-up, idle and busy | Exact receipt; accepted message ID versus queued disposition when available; native reports Unconfirmed in both cases |
| Running with no new output, including a much later observation | Still running; quiet alone never completes a turn |
| Provider failure and quota exhaustion | Error stop with original timestamp and diagnostic; quota must not become normal completion |
| Attention left unanswered, then answered | Remains RequiresAction until response; response returns to running |
| Cancel requested, then a cancelled completion | Requested stays unconfirmed during running; only cancelled completion confirms it |
| Exact resume and missing identity | Same conversation identity; missing identity fails, never starts a fresh conversation |
| Disconnect, restart and stale generation | Old handle fails; replacement works; a late event cannot change replacement state |

An adapter's unavailable capabilities require explicit refusal. Native declares attention,
cancel and resume unavailable: both permission dispatch and capability checks refuse attention,
and cancel/resume refuse before dispatch. Native has no streamed event transport; its stale-event
case checks that refusal and the old pane's inability to receive input after restart. Native
input delivery is unconfirmed: a changing pane proves neither acceptance nor queuing. Quota is
currently an Error carrying the provider diagnostic, without a separate quota stop variant.
The v1 stop-reason table separately checks every terminal reason's cancellation semantics.

## Live compatibility procedure

Run this manually during [Agent CLI compatibility](../CONTRIBUTING.md#agent-cli-compatibility),
after headless compat and before raising tested versions. Fake evidence is not a live pass.
Use a disposable workspace and existing signed-in subscription logins; never store credentials
in evidence. Install mismatched CLI pins under a temporary prefix and isolate writable config;
leave global installations and config unchanged. Record any subscription-auth handoff between CLIs.

1. Record the Niles adapter commit, CLI exact version before and after, protocol version
   (`none` for native TUI), model/effort, OS/tmux version, and auth path actually used
   (subscription provider/login and any token-environment or credential-store handoff).
   An environment variable's name is evidence; its value is secret. Pin installations using the
   approved CLI update procedure. Version drift invalidates the run.
2. Launch with the shipped worker arguments and run the table above except failure/quota cases.
   Use unique sample labels and capture input receipts, pane captures/transcripts and lifecycle observations. For quiet
   work, request a tool that stays silent beyond the normal check-in interval; record elapsed
   time and process liveness. For attention, test both leaving the request open and answering it.
3. Failure and quota classification are fake-only checks: run the shared fixture suite and
   record its result alongside live evidence. Do not force either condition against live
   subscriptions. Check unavailable operations for the refusals above; native attention,
   cancel, resume and streamed events are refusals, not live CLI interactions.
4. For resumable adapters, record the exact identity before disconnect and after resume, and
   attempt a nonexistent identity. Disconnect during work, restart, then deliver a delayed event
   from the old generation through the test peer; it must not alter the new turn. Native checks
   the retired pane handle and event-transport refusal instead.
5. Save a record with one row per table case: versions, auth path, stimulus, observed result,
   pass/refusal/fake-only/untested, and evidence path. A pass requires every live-supported case,
   every declared refusal, and the fake failure/quota checks. Report gaps explicitly alongside
   headless compat; update tested versions only after both gates pass. No live pass is implied by the versions already in the registry.

## Recorded native live run

2026-11-16 UTC: native adapter at `ebd93fb`, using the #567 acceptance spec, on macOS 26.6.2
arm64 with tmux 3.8; protocol `none`. Claude/Codex used temporary pinned installations and
all families used isolated writable config. Versions matched before and after the run.

| Family | CLI pin | Model/effort | Auth path | Result |
| --- | --- | --- | --- | --- |
| Claude | 2.1.312 | sonnet/low | Existing Max OAuth via token environment, isolated config | Pass |
| Codex | 0.165.0 | gpt-6.1-sol/low | Existing ChatGPT OAuth copied to isolated CODEX_HOME | Pass |
| pi | 1.4.0 | openai-codex/gpt-6.1-sol/low | Same ChatGPT OAuth copied to isolated Pi auth store | Pass |

Each passed exact opening and idle/busy multiline receipts, 315 seconds of silent tool work
(still `working` after 305 seconds), disconnect during work, restart delivery, and retired-pane
refusal. Native attention/cancel/resume/event refusals and failure/quota classification passed
in the shared fake suite. Native structured acknowledgments and supported attention/cancel/resume
flows remain unavailable; ACP live behavior was not tested. Per-case evidence is in worker w567's
run report; no global CLI installation or config was changed by the live runs.
