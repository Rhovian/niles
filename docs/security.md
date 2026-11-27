# Niles threat model

Niles coordinates the agent CLIs you already trust, with your permissions. It is not a sandbox. Report
suspected vulnerabilities according to the [security policy](../SECURITY.md).

## Trust model

Niles treats the operator's choice to clone a repository as the repository trust decision.
Processes already running as your user have authority Niles cannot revoke and are out of scope.

The agent-led `niles` install skill states the planned machine changes before executing them.
`niles setup` accepts explicit model tiers and skill choices without the wizard's confirmation;
invoking it authorizes fetching missing skills and writing the default project manifest.
`--start <project>` also registers the directory and launches the entry agent in detached tmux.
The caller owns consent and CLI authentication; setup does not accept harness trust prompts.

Native and headless prompt delivery use the harness contract. Worker permission bypass
and entry-agent permission prompts apply at the launch boundary. Permission answers, cancel
and resume are gated by the adapter’s negotiated capabilities before dispatch.

What Niles launches:

- `niles update` executes `niles-update` beside the resolved Niles executable, never from
  `PATH` or the working directory. This trusts the installed cargo-dist updater and its release
  source to replace the installation with your permissions. `--channel nightly` opts into
  prereleases from the same source; stable is the default for each invocation. Homebrew and Cargo commands are
  printed for the operator to run.
- Native workers start with approval prompts bypassed: Codex with
  `--dangerously-bypass-approvals-and-sandbox`, Claude with `--dangerously-skip-permissions`.
  The entry agent keeps its prompts, because it is the one you talk to. Pi has no built-in tool approval
  prompts in either role and runs with `PI_OFFLINE=1`.
- All agents share the workspace, your environment and your credentials. Niles adds no sandbox
  or process isolation.
- Niles has no repository trust gate of its own and never accepts an agent CLI's trust prompt.
  Skill text is passed at launch, before that prompt, so declining the prompt can't take it back.
- Reports, pane captures, logs and transcripts may contain secrets. Treat `~/.niles/` as
  sensitive. Code displaying transcript text must escape it and never execute it.

## Boundaries

| Boundary | Source or input | What Niles does | Remaining risk |
| --- | --- | --- | --- |
| Repository code execution | `.niles/setups/<name>.yaml`, when you select it | Runs its `worktree.setup` and `gates.post` only from a setup you name with `--setup`; prints gate argv at spawn. | Selecting a repository setup runs its commands as you. See [Named setups](#named-setups). |
| Workspace skills | `.niles/skills/<name>/SKILL.md` | Lists installed skills by scanning workspace/global skill directories, prints each skill's source at launch, rejects workspace symlinks, and enforces size limits. | Skill text reaches agents at system-prompt privilege. See [Skills](#skills-as-an-instruction-source). |
| Imported skills | The pinned GitHub repository | Pins a full commit SHA, fetches only from github.com with hooks off and symlinks as plain files, caps content at 8 MiB, and never runs imported scripts. | Agents may still choose to run those scripts. |
| Global state | State under `~/.niles/`, outside the clone | Opens state files with `O_NOFOLLOW`; validates worker IDs and labels; uses fixed-length SHA-256 workspace keys with a reserved state leaf to isolate projects. | Parent directories aren't protected; locks don't authenticate writers. |
| Status lines and pane text | Worker status lines and captured pane text | Declares the orchestration contract independently of skills; `niles agent status` rejects invalid kinds and details before appending, and checks reports and staging for `done`. A follow-up dispatch resets the stage requirement and round budget. Escapes control characters in `niles wait`, caps lines at 4,096 bytes, never copies status text into nudges, and sends text with bracketed paste, refusing ESC. | Same-user writers can bypass submission checks by editing the log directly; malformed lines are still skipped by wait. `niles workers`, `report` and `peek` print stored text unsanitized. Any text can still mislead an agent or you. |
| Headless ACP | Adapter JSON-RPC and local command socket | Keeps one runner per state directory under a lock; uses a private per-user socket directory, checks session identity and offered permission options, binds form answers to pending requests, and strips terminal controls from rendered text. Verifies declared subscription identity before opening a session and on subsequent auth updates. | Same-user writers can forge configuration/state or connect to the socket. An approved tool executes with the agent's permissions; worker bypass remains enabled. |
| Agent execution | Launch values and files an agent reads | Passes launch values as separate arguments or shell-quoted; passes models and efforts to the CLI for validation. | Agents act with your full permissions. |
| Remote API | HTTPS browser and proxy traffic with OIDC bearer tokens | Each web listener verifies the signature, issuer and host audience and applies project grants. | Bearer replay, relay compromise and agent execution as the host user. See [Remote API](#remote-api). |
| Remote MCP | HTTPS proxy requests with OIDC bearer tokens | Verifies issuer, audience and signature and applies per-project grants to every request. | Bearer replay and agent execution as the host user. See [Remote MCP](#remote-mcp). |
| Remote PWA | Transcript text shown in the app | Sanitizes rendered Markdown, restricts scripts to the app origin, and keeps push payloads minimal. | No clickjacking protection. See [Remote PWA](#remote-pwa-frontend). |

`niles wait` advances wake cursors when the caller passes the batch token with `--ack`.
The latest batch is saved in workspace state, so a later CLI process can acknowledge it.
Unacknowledged wakes remain eligible for replay. Acknowledgements apply to matching worker
generations and status logs and preserve later valid cursor advances.
The watcher uses published receipts for the matching worker generation to suppress duplicate
nudges.

Headless adapter configuration is generated in agent state, leaving user/project configuration
files untouched. Codex ACP clears API-key and alternate-provider environment overrides and
requests ChatGPT authentication explicitly. Claude ACP also removes API-key/provider environment
overrides and requires a reported Claude.ai subscription identity. Pi ACP requires pi's own
`openai` OAuth readiness and pins the selected model in a private launch wrapper.
A process-local native pi extension rejects API-key authentication at request time,
including after credential changes or session/load; it does not copy credentials.
Pi-acp has no ongoing auth-status notification.

Worker launch, replacement and close reserve IDs outside movable worker directories. Shared project
reservations exclude deregistration; joining through metadata publication and retirement share tree
reservations. Busy lifecycle operations fail. Pane delivery waits for the socket and pane reservation,
then retains it through submit acknowledgement. Trust detection holds the check-in lock through its
baseline update. State publishers stage unique files beside their destinations before atomic publication;
this does not promise fsync durability.

Reservations use hashed, typed keys under `~/.niles/locks/`. These small lock files persist after
worker close, pane exit and project deregistration: unlinking an active flock inode would let
another process bypass it. Managed tree paths resolve HOME symlinks before selecting a key.
Replacement publication replaces destination symlinks instead of following them and publishes
files with mode 0600. This covers JSON/YAML, generated briefs and launch files, archived pane text,
and initial compatibility logs. A user-config or workspace-manifest symlink becomes a regular file;
its former target is unchanged, and previous destination modes are not preserved. Append-only status
logs and lock files retain their existing in-place operations.

The global project manifest at `~/.niles/state/<workspace key>/@state/manifest.yaml` is yours, not the
repository's. Its `worktree.setup` and `gates.post` run with your permissions, so only put
commands there you would run yourself. Model spending approval is project-wide:
only exact agents in the manifest's `tiers` list may be selected by `spawn --agent`, and
`--tier` selects only a listed tier. A skill needs no composition binding to use those agents;
composition bindings grant no additional model access.
The manifest's `workers` setting can select a subscription-backed ACP adapter for worker launches.

Workspace keys are the full SHA-256 digest of the absolute path's UTF-8 bytes, in lowercase
hexadecimal. Their fixed length bounds state paths regardless of workspace path length, and
preserves distinctions between separators, hyphens, case and Unicode. Keys cannot be decoded
back to paths; project registration supplies the path. See [state layout](configuration.md#state-layout).
Existing state is not carried over on upgrade; Niles does not migrate or look up old keys.

### Worktree retirement

After closing the last window using a managed worktree, Niles checks for local changes and
unpushed commits under the tree lock. It renames eligible trees into the managed root's
`.trash` directory and removes the checkout's Git registration before starting background
deletion. `niles prune` previews leftover trash for registered projects. `--apply` deletes
those entries regardless of metadata age.

### Project deregistration

Choosing `deregister` in the explorer's project menu immediately deletes the project's
`~/.niles/state/<workspace key>/@state/` folder (including archives and saved configuration)
and managed worktrees, including uncommitted files. It rechecks the registry link and refuses
while any worker metadata remains live, including workers whose tmux windows were lost. A matching tagged project session is closed; unrelated
sessions and the original checkout are preserved. Git removes only registered worktrees of
that repository under its managed tree root; external worktrees are preserved.

Deletion is not transactional: an error can leave some trees removed, with the registration
retained for retry. Live-worker detection is a snapshot, not exclusion against a concurrent
spawn. Do not launch workers during deregistration. A missing checkout with remaining managed
trees requires restoring the checkout before Git can remove them.

### Named setups

A workspace setup is stored under `.niles/setups/` in the repository. It takes effect only when you
select it with `--setup <name>` or `NILES_SETUP`, and it then shadows a global setup of the same
name; `--setup global:<name>` pins your own. Its `worktree.setup` runs through `sh -c` in each
new worktree, and its `gates.post` commands run each time a worker stages. Both run with your
permissions, credentials and network access. Setup files reject symlinks. Spawn prints the
setup's name, origin and gate commands, but printing them is not approval.

### Skills as an instruction source

A repository can ship `.niles/skills/<name>/SKILL.md`, which overrides a global skill of the
same name. Skill bodies injected at launch reach the agent at system or developer-prompt
privilege, above `AGENTS.md`. `niles skills` scans workspace and global skill directories and
reads all installed skills, including unbound ones, using the same workspace precedence, symlink and
size checks as explicit resolution. Invalid discovered skills are skipped with warnings on
stderr, so valid skills remain available in the listing. Explicit launches still reject invalid
skills. Listing an imported skill may fetch its pinned cache. Listing displays source and
description without injecting skill bodies into an agent. Launches resolve only named skills.
Every launch prints `skill: <name> (<source> <path>)`, so you can see where the text came from.
Any agent, in an entry or worker session, can load any installed skill's text with
`niles agent skill <name>`, resolving workspace before global. Composition bindings configure
injection and staging; they do not restrict skill loading. A skill can ask the
agent to load another by name. Import treats literal `niles agent skill <name>` commands with
actual names filling a Markdown code span or text line as required skills (see
[composition syntax](skills.md#composing-skills)). It recursively imports `skills/<name>` from the
same repository and commit into the same destination. This authorizes that repository to update
existing dependency imports in that destination, including ones previously pinned elsewhere.
Local `SKILL.md` files are never overwritten. Missing or invalid dependencies stop import before
publishing pointers; each pointer publication is atomic, not a transaction across all pointers.
Explicit loads return skill bodies as command output, not system or developer instructions.
They print the resolved source and retain the same symlink and size checks. In any session,
a named load of a `SKILL.ref` may fetch its pinned cache.
Size limits are in [skill workflows](skills.md).

### Remote API

Doctor runs the user service's configured executable directly with `--version`. It trusts that
executable as the service manager does when starting the service. Doctor compares file identity
without changing the service.

The `remote` feature is off in release binaries. The web API binds the specific
`remote.web.bind` IPv4 address. Use the host's tailnet address behind the relay, or
loopback for a same-host HTTPS proxy. Keep the proxy-to-backend hop encrypted or local.
With only MCP configured, the web API is disabled.

Remote navigation reports unreadable worker or current lead metadata individually, without paths.
Its metadata-only reads exclude wait cursors and status logs and do not repeat diagnostics per poll.
Push observation skips a corrupt lead; current-session, historical and telemetry lead reads remain
strict. CLI wait, stop, usage and status retain their explicit failures and recovery diagnostics.
Fatal errors exit immediately, allowing the service manager to restart without waiting for blocking jobs.

Tailscale Serve terminates tailnet HTTPS and forwards to Caddy on localhost. Caddy
serves the PWA and proxies requests to each host. The network grants limit which
peers can connect. The web API requires a bearer token on protected routes and
ignores forwarded identities. It shares token verification, issuer allowlisting,
and subject/project grants with MCP. Hosts verify JWTs locally and send opaque
tokens to one configured issuer’s HTTPS introspection endpoint without following
redirects or caching responses. Each host configures a distinct resource URL as
its audience. The MCP endpoint has its own audience. The shell’s `hosts.json`
publishes the issuer and the public identifiers of the OAuth clients.

With host token exchange enabled, the PWA submits a PKCE authorization code or
refresh token to the selected host. The exchange route requires JSON and grants
no cross-origin browser access. The host fixes the redirect URI and resource
and sends its own confidential client credentials to the discovered HTTPS token
endpoint. Register a distinct client per host. Introspection requires an active,
unexpired token issued to this host’s client and for its exact resource. A compromised
host can see codes and tokens for its own client. Its client secret and those
codes cannot mint tokens accepted by a host that requires a different client.
The host authenticates introspection requests with its own confidential client
credentials over HTTPS.

The PWA requests a token for each host using authorization code with S256 PKCE and
state. Access tokens and confidential-client refresh tokens persist in same-origin
local storage across app launches. The PWA serializes refresh rotation per host. The
pending PKCE exchange stays in tab storage. The app sends bearer tokens in the
Authorization header, including on event streams. The host decides whether a token
is valid. HTTP 401 triggers refresh when credentials allow it. A rejected refresh
grant prompts sign-in. Script running on the PWA origin can read both tokens. A
stolen refresh token can be redeemed through the host even after the access
token expires. tsidp refresh tokens last 30 days from issuance, and successful
rotation issues a fresh token with a new lifetime. The relay can see both tokens
and transcripts or replace the served app. [tsidp token lifetimes](https://github.com/tailscale/tsidp/blob/v0.0.21/server/token.go)

Keep the relay, its asset publisher,
the issuer, and the browser profile trusted. Keep credentials out of logs, URLs, and
shared web caches. The provider should issue short-lived tokens. A live transcript
stream closes on token expiry.

`GET /api/telemetry` applies the same project grants as other read routes. It exposes usage
metadata for registered projects' current and archived sessions: project/session identifiers,
lead/worker role, agent family/model, creation/closure times and token totals in UTC-aligned
15-minute buckets. Collection bounds archived-session reads and exported buckets to the
30 days before collection (including the overlapping start bucket). It returns no transcript
text, prompts, tool output or filesystem paths.
API responses are `no-store`. The app caches the last successful telemetry snapshot per
host in same-origin local storage and discards it after sign-in.
An offline host’s snapshot stays available across launches until the app can reach
it. The cutover moves legacy push subscription files aside.
Devices register again with an issuer identity. The push watcher delivers only for projects with a current read or operate
grant for the stored issuer and subject. Push continues when the browser token expires.

The [remote-setup skill](../skills/remote-setup/SKILL.md) configures network grants,
proxy routes and services for installs and migrations. Keep its configuration files and
secrets outside the served asset directory. An operator can instruct the entry agent and
workers in their granted projects, and so can run commands as the host user.
It can start a registered project only with its saved manifest, stop it, and close workers. No
request names a command, argument, skill or path. Message bodies are capped at 16 KiB and can't
contain ESC or NUL. The API grants no CORS access, and mutations use methods that force a
preflight. See the [deployment guide](remote.md).

### Remote MCP

When configured, `niles remote` opens the MCP listener on loopback. An HTTPS proxy can
publish the MCP and protected resource metadata paths beyond the tailnet.
When both listeners are configured, an MCP listener failure or an unreachable issuer at
startup stops the relay API too.
The proxy terminates TLS and can see bearer tokens and transcript data. Keep tokens out of
proxy logs and restrict access to the proxy host. The MCP transport checks the configured
authority and rejects browser Origin headers. Metadata is public.

Each MCP request requires a token for the canonical resource URL. JWTs are
checked against the issuer’s HTTPS-discovered public keys. Hosts can also configure
an opaque issuer for introspection. Only configured issuers
can select verification keys. A valid issuer can assert any subject within its own grants.
An exact issuer/subject grant limits access to the named registered project. Protect the config
and keep project registrations and workload subjects stable. Restart the service to apply
changes to grants or issuers. The introspection secret is read from the service environment. The MCP server
keeps no bearer tokens or MCP sessions.
Stolen bearer tokens can be replayed while
valid and granted. Signing keys refresh on demand after five minutes. If a refresh fails,
previously loaded keys remain usable for up to 15 minutes from the last successful fetch.

Read grants expose project and worker metadata and recent transcript text, which may contain
secrets. Operate grants let a caller instruct agents that run with the host user's permissions.
Use narrow GitHub Actions subjects and control who can run workflows under those subjects.
The application checks project access before invoking lead or worker operations. Outside callers
can start saved sessions and steer their work through messages. Worker creation, stages and
worker self-reporting remain lead/worker responsibilities. See [remote MCP setup](remote-mcp.md).

### Remote PWA frontend

Whoever controls the app's JavaScript controls an instruction surface for your agents, so serve
it only from the relay, on the API's origin. Transcript Markdown passes through DOMPurify,
links open with `noopener noreferrer`, and the shell CSP restricts scripts to the app origin and allows HTTPS connections for issuer discovery
and token exchange. The deployment's Caddy header narrows connections to the app and issuer endpoints. Workbox caches only the built shell and assets so an installed app can open
without the tailnet. API and relay-host paths, including navigations with query strings, bypass
the shell cache; service-worker caches contain no API responses or outgoing messages.
A meta CSP can't set `frame-ancestors`, and Tailscale
Serve can't add the header, so this deployment has no clickjacking protection.

Push payloads carry a session ID, project name, generic reason and the host ID/name supplied
by the authenticated browser at registration. The browser qualifies session links and notification
tags with that host ID. All hosts share one private VAPID key and receive the same browser
subscription; compromising any host exposes that shared signing identity. Payloads are encrypted
for the subscription and signed with VAPID. Waiting transitions and new operator messages
observed in live lead/solo transcripts notify subscribed devices. Terminal input notifies all devices with a current grant for the project.
The PWA identifies its browser subscription by a SHA-256 endpoint fingerprint, not by sending push
registration metadata with messages. The host compares that typed fingerprint and the authenticated
issuer and subject with each stored device when selecting recipients. It excludes that device
only. An unregistered browser can send messages with an operate grant. Malformed fingerprints
are request errors; there is no fallback from invalid subscription metadata.
In-memory receipts correlate pasted text with transcript echoes after trimming surrounding whitespace
and normalizing CRLF. They are recorded before paste, without holding the observer lock during tmux
I/O, and removed on a failed send, a matching echo or session exit. No receipt text is sent to devices.
Niles watcher nudges and known provider-generated user-role envelopes (including interruptions,
command output, hook metadata and environment context) remain visible in chat but do not count as
operator messages. These reserved envelope forms are not authenticated operator input. A malformed
transcript stops operator observation for that session with a diagnostic; other sessions, waiting
transitions and the remote API continue. Push transcript diagnostics contain only error categories,
parser positions and OS error codes, never session identifiers, paths or malformed input values.
Delivery failures log the operation and device fingerprint and leave other deliveries
and the API running; only dead endpoints are pruned.
Operator pushes contain only a generic reason, never message text.
Historical messages are skipped when a transcript first becomes readable, including late discovery. Both reasons use the same
host/session topic and browser tag, replacing queued and displayed notifications for that session.
Push services see delivery metadata, and a locked phone can show the project and reason.
Subscriptions are accepted only for Google, Mozilla and Apple push endpoints, so registration
can't turn the server into an HTTP client inside your network. Keys and subscriptions are
stored with mode 0600. Push host IDs and names are bounded at registration, including JSON
escaping, to keep host metadata within the push payload budget.

## Non-goals

This model does not protect against:

- hostile local users, or isolation failures on shared or multi-user hosts;
- malicious workers already running as your OS user;
- anything the agent CLIs do once launched. Niles doesn't sandbox or contain them.

Niles runs your existing tools with your permissions in repositories you have chosen to trust.

## First-run setup

Bare `niles` starts with fixed-root directory input, a folder-name default and a path/name
confirmation when no projects are registered. An attended start without a manifest opens the
setup wizard: lead or solo, a lead entry skill (`lead` or Custom; solo uses its own skill),
model tiers and entry agent, and optional starter skills with none checked. Only designated lead
candidates appear; unrelated installed skills are neither inspected nor offered. Custom accepts
the same GitHub reference syntax as `niles skills import`, including an optional `@rev`.
It refuses an already installed name. A custom lead's instructions can come from any repository
the operator names, and that lead runs workers unsandboxed. Review its source as executable
instructions; the final summary shows that custom reference.

One final screen puts the safety warning and this documentation link above the summary of the
project, entry skill, entry agent, tiers and skills. One confirmation accepts the warning, fetches
skills, writes the manifest and starts.
Use ←/Esc to go back with answers retained and `q` to quit; in custom text input, Esc goes back
and F10 quits. Back from the first mode step leaves setup. Registration is a separate confirmation
outside that back stack, and remains committed. Back or quit writes no manifest and fetches no
skills.

Missing selected skills and the entry skill are fetched from `github.com/Rhovian/niles` at Git
HEAD, using the existing import mechanism. This trusts the repository’s current default branch,
rather than the binary’s release tag. Custom leads are fetched from their own reference only after
final confirmation and must have a usable, nonempty body before the manifest is written.
A failed custom setup removes its new skill reference so that the same URL can be retried.
The resolved full commit SHA is recorded in each global
`SKILL.ref`; subsequent runs use that immutable revision. Setup skips installed selections, but
importing a missing skill also installs or updates its required imports. Missing revisions,
invalid skill references and fetch failures stop setup before writing
the manifest. The validated manifest is written with atomic exclusive publication, preserving a manifest
that appeared concurrently. No editor or external setup command is launched by the wizard.

Model discovery resolves the installed CLIs before changing into a fresh temporary directory
outside the caller's repository. Claude's print mode skips its workspace-trust prompt, so running
it inside a repository would execute project hooks and load project MCP configuration without
that prompt. The neutral directory prevents that project discovery, including Pi's project-local
extensions. User and managed CLI configuration remain trusted and can execute startup code.

Discovery disables session persistence, Claude MCP servers (`--strict-mcp-config` without an
MCP config), and Pi extensions (`--no-extensions`). It writes the optional catalogue request,
closes stdin, and collects stdout and stderr through process exit. A query has a 30-second
deadline; exceeding it kills the direct child and fails. Catalogue JSON is data. Launch values
use argument vectors or shell quoting; configuration-backed values are encoded as TOML strings
or JSON, rather than interpolated into configuration syntax.

Catalogue failures are shown in their family or context section, preserving unrelated data.
The catalogue command returns a failure status after printing the working families.
