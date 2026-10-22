# Niles Threat Model

This document describes Niles's current security boundaries. It is not a claim that running an
agent on an untrusted repository is safe, and it is not a broad audit of Niles or the agent CLIs it
launches.

Report suspected vulnerabilities according to the [security policy](../SECURITY.md).

## Trust model

The important repository attack begins when a malicious repository author convinces an operator
to clone the repository and launch Niles or one of its agents there. Repository files are attacker
input at that point. That is different from an adversary who already controls the host, the
workspace, or another process running as the operator: such an adversary already has authority
outside the boundary Niles can provide.

Niles keeps only operator-added projects as symlinks in `~/.niles/projects/`. Opening a
registered project creates its named tmux session and starts the lead in that path; workers run
in the same session. Niles writes orchestration state, including role manifests, beneath
`~/.niles/state/<mangled-absolute-workspace-path>/`, outside the repository tree.
It never accepts an agent CLI workspace-trust prompt. Niles does not create an isolation boundary
between the operator, agents, and workspace. Agent permissions ultimately depend on the selected
CLI, its launch flags, and operating-system controls.

The operator's `~/.niles/config.yaml` controls the theme and optional tmux bindings; those bindings run only the shell-quoted Niles executable.

The operator's `~/.niles/roster.yaml` can replace model efforts, tiers and metadata before project
model overrides are applied. Registry parsing rejects unknown fields/families, malformed or
normalized duplicate models/efforts, invalid ranks/windows and control-bearing descriptions.
`niles models` JSON-quotes descriptions and restricts displayed pairs to manifest allowances.
This local file is operator-controlled data; Niles does not authenticate its research claims.

## Boundaries

### Maintainer CLI updates

`scripts/cli-versions.sh` reads installed CLI versions and GitHub release metadata. Its explicit
`--update` mode also changes host installations without confirmation. It preflights all four
CLIs before invoking npm or Hermes's native updater, identifies npm ownership by the resolved
executable's location in npm's global tree, and admits Hermes Git updates only for the official
repository. Unsupported ownership fails rather than selecting an installer. `--plan` performs
the same lookups without running updates. Release versions are parsed as numeric stable versions
and commands use argument arrays; feed and CLI output are never evaluated as shell code.
The operator trusts the installed tools and upstream release/package publishers, including npm
install scripts and Hermes's dependency preparation and service restarts. Updates across CLIs
are sequential and are not a transaction; a later failure does not undo earlier updates.

`niles compat` launches built-in CLIs with approval-bypass flags and operator credentials in a
fresh temp directory, using a fixed prompt with no outside input. It tests shipped profiles,
taking only model and effort from the workspace manifest, validated against the effective model
registry. Workspace binary, argument and prompt overrides do not apply; the directory is evidence,
not a sandbox. Release notes and compare text are upstream-controlled inputs to an agent that may
run commands; the maintainer procedure
treats them as data, never executable instructions. Updates require operator approval of the plan.

### Cloned repository manifest and configuration

**Attacker.** A malicious repository author who can choose committed repository content and induce
an operator to launch Niles from the clone.

**Attacker-controlled input.** Niles reads the first present project configuration file,
`niles.yaml` or `.niles.yaml`. An agent entry can replace its executable and argument list. The
global per-project `manifest.yaml` file selects the agent for each role (lead, worker, reviewer, security,
design, and research), and its per-role tiers and ordered secondaries can influence which
listed agent the lead picks. New manifests live in operator state; legacy manifests migrated
from a clone can still carry repository-authored bindings. See
[`src/config/spec.rs`](../src/config/spec.rs),
[`src/workspace_manifest/types.rs`](../src/workspace_manifest/types.rs), and
[`prompts/lead_brief.md`](../prompts/lead_brief.md).

**Existing mitigations.** Spawn defaults to the role's standard primary and rejects unlisted
tiers or exact model/effort pairs. The lead brief requires operator approval for tier and
secondary changes; the CLI does not enforce that conversational approval. Deserializing YAML
does not itself execute shell commands. Unknown
manifest fields are rejected, and built-in agent families reject unsupported model names. Worker
launch scripts quote the selected binary and arguments, while foreground launches pass them as
separate `Command` arguments; these measures prevent incidental shell parsing. See
[`src/agent_window.rs`](../src/agent_window.rs),
[`src/session/foreground.rs`](../src/session/foreground.rs), and
[`src/agents/mod.rs`](../src/agents/mod.rs).

**Known gaps.** Niles has no repository-trust gate of its own. Once the operator launches it, Niles
may intentionally execute the configured binary with the configured arguments, so quoting is not
a trust mechanism for a deliberately selected executable. Role bindings select what is launched,
and planning text is instruction input to the lead. The manifest can also configure a
`worktree.setup` shell command and `worktree.env`; see the execution boundary below.
`niles doctor` also executes repository-chosen agent binaries with `--version`; the `niles-setup`
skill has the agent show a project `agents:` block to the user and get approval before running it.

### Global project state, migration, symlinks, and locks

**Attacker.** A malicious repository author who supplies tracked `.niles` paths or symlinks before
the operator launches Niles. Tampering after launch by another process with the operator's account
is outside the isolation guarantees described here.

**Attacker-controlled input.** New project state is outside the clone, so tracked repository
paths cannot prepopulate it. On first state access, Niles copies an existing `<workspace>/.niles/`
to staging in the global state directory, publishes the complete copy, then removes the source,
including its manifest and artifacts. Migrated content remains
untrusted repository input: migration does not authenticate it. A top-level legacy symlink or
file is rejected, and an existing destination causes an error rather than merging or overwriting
state. The key uses plain slash-to-hyphen replacement: `/Users/j/code/my-app` becomes
`-Users-j-code-my-app`. Separator/hyphen collisions are accepted and can share project state.
There is no escaping or hash fallback; filesystem component length limits apply. The registry remains `~/.niles/projects/`.
Niles subsequently creates and reads manifests, session artifacts, worker directories, metadata,
briefs, launch scripts, reports, status logs, and cursors in the global tree. See
[`src/store/paths.rs`](../src/store/paths.rs) and [`src/store/worker.rs`](../src/store/worker.rs).

**Existing mitigations.** Migration holds a per-project advisory lock under `~/.niles/state/`.
Legacy migration always copies to staging while preserving symlinks without following them.
It publishes the completed copy before removing the source. A failed copy
leaves the source intact and attempts to remove partial staging without replacing the original
copy error if cleanup also fails. Retries use a fresh staging directory. Stop agents before upgrading:
migration cannot update paths already held by running agents or their shell scripts, or
synchronize writes made during the copy.
`niles log` appends timestamped decisions via
Niles, so the lead needs no extra filesystem sandbox grant. It requires the latest lead session's
recorded pane to match `TMUX_PANE`, validates the worker label and rejects control-bearing decision
text; this is session routing, not authentication against same-user processes.

Worker IDs and task labels accept only ASCII letters, digits, hyphens, and
underscores (and reserve `archive`). The final open of `status.log` for `niles wait`, the worker
status CLI, Niles file appends (through `append_line` or the trust watcher's held descriptor),
spawn's log creation, and the opens of `status.cursor`, `wait.lock`, `wait.running`, and `checkin.lock` use
`O_NOFOLLOW`. Wait state lives under `~/.niles/state/<mangled-absolute-workspace-path>/worker/`:
`wait.lock`, `wait.running`, and temporary `wait.running.<uuid>` files are siblings of the
worker directories. No wait state is written inside the repository. A workspace wait reads
every live worker's log, cursor, and metadata from that project's global state tree. Liveness
checks use the same local session-record and tmux/process-activity resolver as workers/watch;
session links may point into agent stores or worktrees. Liveness probes print a fixed gone reason.
An exclusive advisory `flock` on `status.cursor` covers the cursor read, status scan, flushed
output, and cursor advance, serializing delivery against `send`'s cursor skip. Each line is
flushed before its cursor moves: a killed wait cannot consume an unprinted line. A kill after
printing but before advancing can replay that printed line; the operator accepts this boundary.
Wait also records generated `gone:` lines for an unreported worker with a confirmed exited
window/agent or 30 seconds of confirmed idle state. It checks for a real report again after
probing activity and uses the same cursor lock, escaped output and print-before-advance path.
A concurrent worker append can race the final synthetic-status append, as with watcher reports;
its report remains in the log. Send appends a non-actionable dispatch marker so a silent follow-up
can be detected even with check-ins disabled. The short-lived exclusive lock on
`wait.lock` serializes registration with delivery: a newer registration supersedes the
old wait, which checks that its held presence inode is still current before any delivery. The kernel releases locks on death;
no old registration can prevent takeover. A shared `flock` on workspace `wait.running` lets the
watcher suppress report nudges while a wait runs. Registration publishes a newly locked
presence inode by renaming a unique `wait.running.<uuid>` file over `wait.running`; old waits
retain only the unlinked inode, so a suspended stale wait cannot suppress later report nudges.
Temporary presence files use `create_new` and `O_NOFOLLOW`. Every write or removal of a
worker's check-in holds an exclusive `flock` on `checkin.lock`, so the watcher's re-read and the action that follows cannot interleave with a `send`
arming a fresh one. See [`src/worker/validation.rs`](../src/worker/validation.rs) and
[`src/wait.rs`](../src/wait.rs).

`niles prune` previews deletions unless `--apply` is supplied. It removes registry links whose
targets are not directories, and dated archive and session directories, always retaining the
newest session. Archive and session entries are checked with `symlink_metadata`, so symlinks and
files are left alone. Unparseable names are also retained. See
[`src/projects/prune.rs`](../src/projects/prune.rs).

`niles agent status` accepts only worker-owned kinds and non-empty details without control
characters. It requires an existing status log and uses `append_line` with `O_NOFOLLOW`;
`done` also requires a non-empty report. Status text remains unauthenticated input to the lead.

**Known gaps.** `O_NOFOLLOW` protects only those final path components; parent directories and
other filesystem operations are not protected globally against symlink traversal or replacement.
The status CLI checks that the log exists before calling `append_line`, which opens with
creation enabled: concurrent removal between the check and open can recreate that log.
The worker launch script's shell append (`echo closed: ... >> "$STATUS"`) does not use
`O_NOFOLLOW`.
The advisory locks authenticate neither writers nor status content, and any process with the
operator's permissions can hold `wait.lock`'s lock and so block workspace waits, or
hold `wait.running`'s and so silence the watcher's report nudges in the workspace.
Other global project state is not universally locked. Niles does not defend its state from a process that
already has the operator's filesystem permissions.

Name-only project and window status jobs emit tmux conditionals so selection is evaluated on
redraw. Only validated project names and numeric window indexes enter expressions; worker and
plain-window labels stay outside them with `#` escaped, and style commas are escaped inside
conditional branches. Each project's window-list command uses a shell-quoted literal session
name, giving it a separate tmux job cache. These bars read membership and role metadata; the home
header and explorer still resolve telemetry.

Theme config text never reaches tmux formats. The theme name selects a palette; only typed RGB
colors and styling modifiers are rendered into tmux formats.

### Status lines, pane capture, and terminal or lead injection

**Attacker.** A malicious repository author whose content is repeated or acted on by an agent, or
another source able to influence agent output. A deliberately malicious same-user worker is a
non-goal, but its output would reach the same display and prompt paths.

**Attacker-controlled input.** Influenced text can enter worker status lines and reports, appear in
captured tmux panes, or be included in the repository text an agent reads. This creates both
terminal-control risks and semantic prompt-injection risks.

**Existing mitigations.** `niles wait` escapes characters for which Rust's `char::is_control` is
true and limits each escaped rendered line, including its worker id and truncation suffix,
to 4,096 bytes. Worker IDs and task labels have the restricted grammar described above. Watcher nudges identify only the
worker and wake kind; they do not copy status-line content into the lead pane. Text sent through
tmux uses literal-key mode with explicit bracketed-paste delimiters. Niles refuses text containing
ESC so message bytes cannot end the paste early. Framing does not depend on the pane advertising
paste mode or newer tmux format fields. During a new worker's first 30 seconds, while its status
log is still empty, the watcher recognizes specific Claude and Codex workspace-trust prompts
displaying the exact workspace, appends a `blocked:` status, and never accepts the prompt or sends
keys. See
[`src/wait.rs`](../src/wait.rs), [`src/watch/decide.rs`](../src/watch/decide.rs),
[`src/tmux.rs`](../src/tmux.rs), and [`src/watch/trust.rs`](../src/watch/trust.rs).
Before sending a lead nudge, the watcher captures the visible lead pane and queries its cursor.
It holds the nudge only when the cursor is visible on a typed Claude or Codex composer row, or an
indented continuation of one. An absent marker, hidden cursor, or failed query delivers; a
continuous hold ends after five minutes. The recognizer reads terminal output and cursor position
as state only and never submits captured text.

Agent-written session records supply turn state and start times; live process descendants of the
tmux pane can keep an ended turn Working. The shared resolver uses `ps` PID/PPID/state/elapsed
time and executable names, excluding zombies and the querying Niles/probe branch. Only processes
started during the turn that are shells (executable name ending in `sh`) or have a shell ancestor
below the pane root count as work. Directly launched helpers remain standing even if started
mid-turn. Helpers launched through shells can count as work; shell `exec` and reparenting can
hide command ancestry. This is a naming convention, not authenticated shell identity.
Elapsed-time precision gets a one-second tolerance favoring work detection. Reparented processes
outside the pane tree cannot be attributed, and snapshots race with exits. Same-user processes
can forge session records and affect process-tree evidence; this is state detection, not isolation.
A transcript Niles cannot parse is no record: after the initial check-in interval the worker is Unknown and gets `blocked: session state unknown ...`; a read error or uncertain pane
liveness nudges and re-arms instead. A structured terminal failure in a Codex, Claude or pi record
becomes `blocked: provider refused (<label>): <first line>`, never from pane text: the message is
stripped of controls and capped at 1,024 characters, the label at 256; nudges carry only the worker
ID and status kind. Appends use `O_NOFOLLOW` and recheck the check-in under its lock and the log
length; worker status append does not take that lock, so a racing report can interleave (accepted, #348).

`niles panel config` no longer prints raw file bytes: parsed role cells, model names, and load errors
are clamped and stripped of control characters before terminal rendering. The shared roles picker
also clamps repository-authored family names and role values. It probes configured executables
with the same PATH lookup as doctor; it does not execute them.
When a manifest cannot deserialize, the interactive roles picker displays a clamped, control-stripped
parse error and starts from built-in defaults. Only an explicit save replaces the invalid manifest;
quitting leaves it unchanged, and non-interactive launches still fail.

**Known gaps.** The wait renderer is not a global sanitizer. `niles workers` prints stored status
strings, `niles report` prints a report body, and `niles peek` prints tmux capture without a shared
universal sanitizer. See [`src/worker/list.rs`](../src/worker/list.rs),
[`src/worker/report.rs`](../src/worker/report.rs), and
[`src/worker/pane.rs`](../src/worker/pane.rs). Escaping terminal control characters, excluding
status content from nudges, and using tmux bracketed paste do not establish semantic authorization:
ordinary text can still instruct or mislead an agent or operator. Startup trust-prompt detection is
bounded and recognizes only known exact-workspace Claude and Codex prompt shapes; it is not general
prompt or injection detection.
Message delivery assumes the receiving composer understands bracketed paste; tmux mode
advertisement is not checked. Composer recognition depends on the CLIs' current screen layout and
styling. A keystroke between capture and submit can still merge with a nudge.

### Agent CLI execution

With `--tree`, the worker agent runs in a directory chosen by the lead; Niles validates only that it
exists and is a directory.
With `--worktree`, Niles uses git to create a worktree under `~/.niles/trees` and can
remove it on close. The branch name is checked by git before it becomes a path. Close removes only
registered worktrees under that workspace's managed tree root after checking live workers, status,
and remote reachability; it never deletes branches.

The operator-owned global project manifest may supply `worktree.env` and `worktree.setup`. For
`spawn --worktree`, Niles applies literal environment overrides to the worker and executes setup
via `sh -c` in a newly created tree before launch, with those overrides and Niles's inherited
environment. Setup receives EOF on stdin; stdout and stderr both go to Niles's stderr so setup
cannot consume caller input or mix output into spawn results. Setup is arbitrary code with the
operator's permissions, credentials, and network access; it is not sandboxed and runs before any
agent CLI trust prompt. Commands may themselves execute repository content (for example package
install scripts). Review setup and environment entries before spawning, including in migrated
legacy manifests: migration does not confer trust. Niles validates environment names as shell
identifiers and rejects NUL-bearing values; worker exports quote values to prevent incidental
shell evaluation. Setup text is deliberately shell code. Setup never runs when joining an
existing tree or using `--tree`. Failure aborts spawn; Niles forcibly removes the newly created
tree and setup-created files so a retry cannot skip failed setup. Effects outside the tree
cannot be rolled back.

Creation and setup hold a per-branch advisory file lock in global project state;
concurrent joins fail rather than entering a tree under preparation. A `.pending` marker
is created before `git worktree add` and removed only after successful setup or successful
failed-setup cleanup. The lock is released on process death, but the marker persists: failed Git
creation, interrupted creation/setup, and cleanup failure block reuse, including when
configuration changes or an orphan setup later exits. Recovery requires stopping remaining setup
processes, inspecting/saving needed files, removing the incomplete Git worktree, and clearing
the marker named in the error before recreating it. Setup is never rerun on an existing tree.
Existing trees from before this tracking have no marker and are accepted as-is. Locks and
markers are local state, not authentication or protection against another process with the
operator's permissions.

**Attacker.** A malicious repository author who can influence selected configuration, manifest
bindings, briefs, or workspace content and induce the operator to launch an agent.

**Attacker-controlled input.** The selected executable, its configured arguments, role selection,
planning guidance, briefs, and repository content all affect the process Niles launches and the
work it is asked to perform.

**Existing mitigations.** Foreground agents are launched with a process argument vector, and worker
script values are shell-quoted. Static built-in profiles validate their supported model and effort
values. These are launch-integrity measures, not containment. See
[`src/session/foreground.rs`](../src/session/foreground.rs),
[`src/agent_window.rs`](../src/agent_window.rs), and
[`src/agents/families.rs`](../src/agents/families.rs).

Niles reads Claude, Codex, Hermes, and pi session stores to report agent token usage and extract
user/assistant text and structured tool calls. It uses read-only access to those stores and links
sessions through launch identifiers or a Codex brief. Transcript content is untrusted display
data, including tool arguments; consumers must escape it and must never execute it. Tool results,
images, and private reasoning blocks are omitted.
For Hermes, it opens the local `state.db` read-only to read session usage and state.
For pi, Niles chooses transcript directories under its global project worker and lead session
state directories using `--session-dir`, and reads the newest JSONL transcript there.

**Known gaps.** Built-in worker profiles explicitly launch Codex with
`--dangerously-bypass-approvals-and-sandbox`, Claude with `--dangerously-skip-permissions`, and
Hermes `chat` with `--yolo`. Pi runs with no permission prompts in either launch path, like
Hermes with `--yolo`. Niles launches pi with `PI_OFFLINE=1`, which disables its startup update
check and install telemetry only. Foreground defaults differ, and project configuration can replace
default arguments for either launch path. Actual access therefore depends on the agent CLI, launch
flags, credentials, and OS permissions. Niles adds no sandbox or process isolation. Agents share
the workspace tree and inherited environment, and global state artifacts, reports, pane captures, and
logs may contain secrets. Treat them as sensitive.

Retier sends validated same-family model and effort commands to the recorded live worker window,
requires an idle session and empty composer, and records the binding only after fresh
value-specific CLI acknowledgements. Pane text is not authenticated: a malicious local process
can forge acknowledgements. Partial or unconfirmed switches fail loudly without updating metadata.
Codex retier navigates recognized model and reasoning picker rows, including advanced reasoning,
and uses the session-only action. It requires a fresh acknowledgement naming both model and effort
before recording the binding; missing choices or unfamiliar picker layouts fail without recording.

## Non-goals

This model does not promise protection from:

- hostile local users or shared-host and multi-user isolation failures;
- malicious workers already running as the same OS user; or
- sandboxing or otherwise containing the agent CLIs Niles launches.

These non-goals do not make a cloned repository trustworthy. They clarify that Niles coordinates
the operator's existing tools and permissions rather than reducing them.

### Remote API

**Phone access is shell access on the host.** Foreground agents can execute commands
and spawn workers that bypass approval prompts; Niles provides no sandbox. Device
admission is tailnet membership plus ACLs/grants, followed by Niles's user allowlist.
See the [deployment guide](remote.md) for the supported static-root/API-proxy setup.

The optional `remote` Cargo feature is off in default/release binaries and requires
system OpenSSL, without vendoring. It adds an HTTP/SSE listener only while `niles remote`
runs, always on IPv4 loopback. Startup requires a nonempty global `remote.allowed_users`.
Every route, including unknown routes and SSE, requires exactly one matching
`Tailscale-User-Login` header. This is an identity assertion from Tailscale Serve,
not a credential accepted from arbitrary network clients: Serve must terminate tailnet
access and supply the trusted header. Local processes can forge it and already have
access to the same user's plaintext state; hostile local users remain outside this model.
Serve removes client-supplied identity headers and injects verified ones; tagged
clients lack user identity headers and cannot pass this authorization. The API must
never bind a LAN/tailnet address or trust a public proxy's asserted login. No Funnel.
The static shell is not allowlist-gated: tailnet ACLs govern its retrieval, while the
API enforces the allowlist before transcripts/actions. Serve only built shell assets,
not repository/state/key directories. There is no CORS grant. API responses, errors
included, carry `Cache-Control: no-store`.

`GET /api/projects` shares the explorer's registered-project collection and the
same authorization and no-store boundary. It exposes project names/states and
worker IDs/roles/loss, including inactive projects; it excludes filesystem paths,
window targets, briefs, and telemetry. The PWA stores only the last opened project
name in origin-local storage. Worker rows open transcripts and expose a close action.

`niles remote setup` persists this existing host/origin boundary in a launchd/systemd
user service and two Tailscale Serve handlers. It merges explicit allowed identities,
keeps existing VAPID keys, and refuses existing background root/API mounts, unknown
Serve schema fields and existing service files. Tailscale applies its other Serve rules.
Setup invokes local `tailscale`, service-manager,
and `curl` executables from the operator's PATH; these and the selected built web assets
are trusted host code. It does not configure tailnet ACLs, join devices, or enable
HTTPS/MagicDNS. Host-side API verification is advisory: a failed check warns and
prints a check for an allowed tailnet device without undoing setup. The Serve
inspection and mount commands are sequential, not atomic;
do not change Serve concurrently during setup. Partial failures leave completed
configuration in place for inspection and manual cleanup.

Session IDs are resolved only against registered, running project sessions; request
IDs are never used as filesystem paths or tmux targets. Worker IDs are resolved by enumerating live metadata and comparing IDs by equality.
Worker transcript inputs use the recorded agent directory and report path. Session collection and transcript
reads run on blocking workers rather than the async executor. SSE uses the independent
incremental readers, bounded event queues, and a fresh snapshot on each connection.
Snapshots limit turn count to 200, not bytes per turn; initial transcript reads still
scan the available history. Disconnected clients stop polling; vanished sessions close
the stream. Read errors are logged locally and close the stream without exposing
filesystem/error details to the client. Append cursors do not replay in-place edits
or switch transcript files underneath an open reader.

Authenticated message POSTs resolve the opaque ID again against the live catalog and
send only to its recorded foreground agent pane or live worker window through
`tmux::send_line`. Worker targets must be live or recovered through their project/ID tags;
lost workers retain readable transcripts but sending returns 503. No fallback
target, draft/prompt guard, or watcher-shared lock is introduced. JSON bodies are capped
at 16 KiB; text must be nonblank and cannot contain ESC (paste framing) or NUL (process
arguments). Other content, including newlines and shell-looking text, remains literal.
Authorization runs before body parsing or tmux actions. A 200 follows the shared send
path's pane-change acknowledgement; it is not proof that the model acted on the message.
A send failure can follow a paste/submit, so clients must not retry automatically.
Tmux failure details stay in local diagnostics, not HTTP responses. Catalog resolution
and injection are not atomic; concurrent session exit/restart or desk/watcher input can
race with sending, under the operator's no-lock/no-draft-guard ruling. Trusted frontend JavaScript from this host is essential: an
allowed tailnet user can instruct an agent with the host user's execution privileges.

Remote control can start only a saved manifest's default lead for a registered project,
stop a session tagged with that registry path, and close enumerated live/lost workers.
No request chooses an agent, command, args, role or filesystem path. Registration and
manifest creation remain desk-only. Start checks the directory, parses the manifest,
and refuses foreign session names. Its fixed 409 texts distinguish those three
operator-fixable conflicts; other errors expose only status codes. Phone starts bypass
the role picker with `niles lead --keep-roles`. Agents run with the host user's privileges
and the service/tmux server environment; registration already trusts project configuration.
Shell-rc-only credentials may be unavailable. Worker close runs `current_exe` with a
registry cwd and enumerated ID as argv, without a shell. Stop closes workers before
killing the tagged session. These operations are not atomic against desk/lead actions:
concurrent disappearance is rechecked and logged; workers spawned during stop can
remain as lost metadata, closable later. Concurrent crashed-lead starts can respawn
over one another, leaving an orphan session directory for `niles prune`.

Tailscale identity is ambient on requests from an allowed device, including cross-site
requests. Control mutations accept only PUT/DELETE, requiring a CORS preflight that
gets no grant. Message POSTs require JSON content type for the same reason. GETs
have no side effects. No API route grants CORS access.

### Remote PWA frontend

The installable `web/` app is an instruction surface for the foreground agent, which
can execute code with the host user's permissions. Serve its release asset only
from the Niles host through Tailscale, on the API's origin. Whoever controls this
JavaScript effectively controls that instruction surface. The frontend supplies
no authentication; the loopback remote API must authorize trusted Tailscale identity
headers. Public static hosting moves this trust boundary and is unsupported.

Agent transcript text can echo hostile repository or web content. Every markdown
render passes Marked output through DOMPurify; forms, active content, remote images,
and inline style attributes are excluded. Sanitized links open in separate tabs with `noopener noreferrer`. The shell's CSP
restricts scripts, connections and workers to its own origin and permits no inline
script. Meta CSP cannot enforce `frame-ancestors`; the specified direct-directory
Tailscale Serve deployment has no custom response-header setting. Framing protection
is a known clickjacking gap of this deployment; an alternative same-host HTTP server
behind Serve could supply the header.
The service worker precaches only built shell assets, never API traffic; transcripts
and optimistic messages stay in memory. API responses must use `Cache-Control:
no-store` to cover HTTP caching as well. Push payloads contain only session ID, project, and a generic reason, never transcript
text. They are encrypted/signed using `web-push` (RFC 8291/VAPID); OpenSSL generates
the keypair. Push services observe delivery metadata, and OS notification surfaces
can display project/reason while the device is locked or outside the tailnet.
The PWA uses a stable session tag to replace that session's active notification,
without re-alerting while it remains active. Different sessions use different tags.
Clicks encode the ID into a fixed same-origin session hash, ignoring arbitrary URLs.
Malformed pushes still display a generic visible notification.

VAPID and per-device subscription files are private (0600, directories 0700), with
no-follow file reads and atomic subscription replacement. Initialization refuses to
rotate existing signing keys. Subscriptions record their authenticated owner; push
only targets devices whose owner is still in the startup allowlist. Revocation and
key initialization require restart, like other remote configuration changes.
The subscription API accepts only HTTPS/443 endpoints at exactly `fcm.googleapis.com`,
`updates.push.services.mozilla.com`, or `web.push.apple.com` (Google/Mozilla/Apple),
with no credentials, fragments, or redirects. This prevents arbitrary endpoint registration from becoming an HTTP client into the host/tailnet.
Subscription crypto keys are checked through the encryption library before storage.

Push monitoring is server-side and independent of open web clients. It detects
waiting transitions from the shared live catalog, sends only a minimal encrypted
payload, and prunes 404/410. It has no automatic retry; other provider/store/catalog
failures propagate and stop the remote server. Restart can re-notify an already-waiting
session. A five-minute TTL limits stale delivery. Push requires outbound public
internet even when API access is restricted to Tailscale; message access still uses
the tailnet boundary. Directory ancestor replacement and same-user state tampering
remain outside the existing threat model.
