# Niles threat model

Niles coordinates the agent CLIs you already trust, with your permissions. It is not a sandbox. Report
suspected vulnerabilities according to the [security policy](../SECURITY.md).

## Trust model

Niles treats the operator's choice to clone a repository as the repository trust decision.
Processes already running as your user have authority Niles cannot revoke and are out of scope.

What Niles launches:

- Workers start with approval prompts bypassed: Codex with
  `--dangerously-bypass-approvals-and-sandbox`, Claude with `--dangerously-skip-permissions`.
  The entry agent keeps its prompts, because it is the one you talk to. Pi has no approval
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
| Repository configuration | `niles.yaml` or `.niles.yaml` | Accepts only model and effort entries; unknown fields fail. It can't change executables or launch arguments. | It can add models and efforts that spawns may select. |
| Repository code execution | `.niles/setups/<name>.yaml`, when you select it | Runs its `worktree.setup` and `gates.post` only from a setup you name with `--setup`; prints gate argv at spawn. | Selecting a repository setup runs its commands as you. See [Named setups](#named-setups). |
| Workspace skills | `.niles/skills/<name>/SKILL.md` | Lists installed skills by scanning workspace/global skill directories, prints each skill's source at launch, rejects workspace symlinks, and enforces size limits. | Skill text reaches agents at system-prompt privilege. See [Skills](#skills-as-an-instruction-source). |
| Imported skills | The pinned GitHub repository | Pins a full commit SHA, fetches only from github.com with hooks off and symlinks as plain files, caps content at 8 MiB, and never runs imported scripts. | Agents may still choose to run those scripts. |
| Global state | State under `~/.niles/`, outside the clone | Opens state files with `O_NOFOLLOW`; validates worker IDs and labels; uses reversible byte-encoded workspace keys with a reserved state leaf to isolate projects. | Parent directories aren't protected; locks don't authenticate writers. |
| Status lines and pane text | Worker status lines and captured pane text | Declares the orchestration contract independently of skills; `niles agent status` rejects invalid kinds and details before appending, and checks reports and staging for `done`. Escapes control characters in `niles wait`, caps lines at 4,096 bytes, never copies status text into nudges, and sends text with bracketed paste, refusing ESC. | Same-user writers can bypass submission checks by editing the log directly; malformed lines are still skipped by wait. `niles workers`, `report` and `peek` print stored text unsanitized. Any text can still mislead an agent or you. |
| Agent execution | Launch values and files an agent reads | Passes launch values as separate arguments or shell-quoted; validates models against the roster. | Agents act with your full permissions. |
| Remote API | Requests forwarded by the relay node | Listens on the host’s tailnet IPv4 address; WhoIs must identify the configured relay, with one forwarded Tailscale login. | Phone access is shell access. See [Remote API](#remote-api). |
| Remote PWA | Transcript text shown in the app | Sanitizes rendered Markdown, enforces a same-origin CSP, and keeps push payloads minimal. | No clickjacking protection. See [Remote PWA](#remote-pwa-frontend). |

The global project manifest at `~/.niles/state/v2/<workspace key>/@state/manifest.yaml` is yours, not the
repository's. Its `worktree.setup` and `gates.post` run with your permissions, so only put
commands there you would run yourself. Model spending approval is project-wide:
only exact agents in the manifest's `tiers` list may be selected by `spawn --agent`, and
`--tier` selects only a listed tier. A skill needs no composition binding to use those agents;
composition bindings grant no additional model access.

Workspace keys encode the full absolute path's UTF-8 bytes as lowercase hex, split into
directory components of at most 240 characters. `@state` is outside that alphabet, so prefix
keys cannot contain each other's state. See [state layout](configuration.md#state-layout).
Legacy slash-to-hyphen state must be migrated by hand before installing the new binary;
there is no legacy lookup that could share another workspace's state.

### Project deregistration

The explorer's `d` action requires confirmation before deleting a project's
`~/.niles/state/v2/<workspace key>/@state/` folder (including archives and saved configuration)
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
same name. A named skill's body reaches the agent at system or developer-prompt privilege,
above `AGENTS.md`. `niles skills` scans workspace and global skill directories and reads all
installed skills, including unbound ones, using the same workspace precedence, symlink and
size checks as explicit resolution. Invalid discovered skills are skipped with warnings on
stderr, so valid skills remain available in the listing. Explicit launches still reject invalid
skills. Listing an imported skill may fetch its pinned cache. Listing displays source and
description without injecting skill bodies into an agent. Launches resolve only named skills.
Every launch prints `skill: <name> (<source> <path>)`, so you can see where the text came from.
Size limits are in [skill workflows](skills.md).

### Remote API

The `remote` feature is off in release binaries. `niles remote` binds the host's IPv4 address
from `tailscale ip -4`, never a wildcard or LAN address. Set `remote.relay_node` in
`~/.niles/config.yaml` to the relay's stable Tailscale node ID (the `Node.StableID` from
`tailscale whois --json <relay-tailnet-ip>`). Missing or invalid configuration stops startup.
Each request authenticates its TCP peer through `tailscale whois --json --proto=tcp` and must
match that node ID. Forwarding headers cannot select the peer. WhoIs failures and malformed
responses fail closed; another node gets 403 even with a forged login header.

On the relay, Tailscale Serve terminates tailnet HTTPS, strips client-supplied identity headers
and injects a verified `Tailscale-User-Login`. Serve proxies to Caddy on localhost only; Caddy
serves the PWA and forwards `/h/<host>/...` to hosts' tailnet addresses, preserving that login.
The host requires exactly one nonempty, well-formed login header on every route, including
unknown routes. It accepts any login the relay forwards: there is no per-host user allowlist;
tailnet ACLs are the user access gate. The old `remote.allowed_users` setting does not restrict
host requests or push delivery. Use Serve, never Funnel.

The relay is now trusted to assert user identity for every host. A compromised relay or a
local process that can make connections as the relay can impersonate any user. Restrict
access to its node credentials and local Caddy listener. A direct connection from a user's
other tailnet device is rejected, even when that device can reach the host through ACLs.
Host-local processes remain outside this model. Existing single-host setup migration is not
provided by this change; the deployment setup must configure the relay and ACLs.

Any user admitted by the relay can instruct the entry agent and workers, and so can run commands as you.
It can start a registered project only with its saved manifest, stop it, and close workers. No
request names a command, argument, skill or path. Message bodies are capped at 16 KiB and can't
contain ESC or NUL. The API grants no CORS access, and mutations use methods that force a
preflight. See the [deployment guide](remote.md).

### Remote PWA frontend

Whoever controls the app's JavaScript controls an instruction surface for your agents, so serve
it only from the relay, on the API's origin. Transcript Markdown passes through DOMPurify,
links open with `noopener noreferrer`, and the CSP allows scripts and connections from the
same origin only. A meta CSP can't set `frame-ancestors`, and Tailscale
Serve can't add the header, so this deployment has no clickjacking protection.

Push payloads carry only a session ID, project name and generic reason, encrypted with VAPID.
Push services see delivery metadata, and a locked phone can show the project and reason.
Subscriptions are accepted only for Google, Mozilla and Apple push endpoints, so registration
can't turn the server into an HTTP client inside your network. Keys and subscriptions are
stored with mode 0600.

## Non-goals

This model does not protect against:

- hostile local users, or isolation failures on shared or multi-user hosts;
- malicious workers already running as your OS user;
- anything the agent CLIs do once launched. Niles doesn't sandbox or contain them.

Niles runs your existing tools with your permissions in repositories you have chosen to trust.

## First-run setup

Bare `niles` offers registration only when no projects are registered. An attended terminal
start of a project without a manifest offers setup in its pane. Before
consent it names GitHub (`Rhovian/niles`), the binary’s release tag, and workers’ bypassed approval
prompts and absent sandbox. Enter accepts. Missing skills are fetched using the existing Git
import mechanism and pinned globally to a commit at `v{CARGO_PKG_VERSION}`; existing skills are
kept. Trust in these agent instructions rests on the release repository and tag. Once installed,
a moved tag cannot change a pinned skill. Missing revisions, malformed references and fetch
failures stop setup before the manifest is written; the registered project can be started again.

Custom setup runs the operator’s `$VISUAL`, `$EDITOR`, or `vi` command on a uniquely named,
0600 draft in the project’s user-owned state directory. The draft path is a separate shell
argument. The file is read once per edit, parsed, and its entry and agents resolved before skill
imports. The exact validated text is written with exclusive creation, preserving a manifest
that appeared concurrently. This matters because manifest worktree setup commands execute with
the user’s permissions. Drafts are removed on success, abort, and error; a crash may leave a draft
in the state directory, which deregistration removes.
