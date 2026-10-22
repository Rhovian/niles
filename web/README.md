# Niles web

Installable Preact PWA, built independently of Cargo. Requires Node 24 and pnpm
10.8.1 (pinned in package.json).

```sh
cd web
pnpm install --frozen-lockfile
pnpm dev
```

Open the printed localhost URL. Development uses in-memory mock sessions (including
`solo`); messages are echoed, SSE snapshots are sent on every connection, and status
changes follow sends. Push endpoints return 501 in the local mock; real push requires the remote server. Mock state disappears
when Vite stops and is never included in the production client.

`pnpm lint`, `pnpm typecheck`, `pnpm test`, and `pnpm build` run in web CI. `pnpm preview` serves a previously built `dist/` without the mock API.
The service worker is registered only in production. Install from the browser's
install/Add to Home Screen control over HTTPS (localhost also works).

For installation from `niles-web.tar.gz`, same-host Tailscale Serve configuration,
allowlist and VAPID initialization, follow the [remote setup guide](../docs/remote.md).
Only the built shell directory is served; Cargo never builds this frontend.
The [threat model](../docs/security.md#remote-pwa-frontend) covers sanitized markdown,
shell-only caching, CSP and the known framing gap. API responses must use no-store;
no transcript or outgoing message is saved in browser storage.

Updates install in the background and activate when existing app windows close;
close all Niles tabs/app windows, then reopen to load a newer shell. An API mismatch
blocks chat and shows an update banner. Offline mode shows the shell and a tailnet
connection banner; it does not replay transcripts or queue messages for later.

## API v1 contract

`src/api.ts` is the single client contract and runtime JSON parse boundary. All
URLs are same origin. JSON property names use snake_case. IDs are opaque strings;
the client percent-encodes session IDs in URL path segments.

- `GET /api/projects` → `200 {"api_version":1,"projects":[Project]}`. Additive v1
  endpoint using the explorer's registered-project snapshot. `Project` contains
  `name`, `state` (`missing`, `not running`, `running`, `starting`, `waiting`), and
  `workers: [{id, role, lost}]`. Roles are worker, reviewer, security, research,
  design. Filesystem paths, window targets, and agent prompts are excluded.
- `GET /api/sessions` → `200 {"api_version":1,"sessions":[Session]}`.
  `Session = {id, project, agent:{family, entry_skill}, waiting, worker_count}`.
  `id`, `project`, `family`, `entry_skill` are strings; `id` is nonempty;
  `waiting` reflects the lead alone, independently of worker activity; `worker_count` is a nonnegative integer. The foreground
  **agent** can have any entry skill (including `lead` and `solo`). The list refreshes
  every 15 seconds and on window focus; waiting sessions sort first.
- `GET /api/sessions/:id/events` → `200 text/event-stream`. Named events:
  `snapshot` with `{"turns":[Turn],"truncated":boolean}`, `turn` with `Turn`,
  `status` with `Session`. Send a snapshot of at most the **200 most recent turns**
  in order on every connection/reconnection, before deltas. `truncated` means older
  turns were omitted; the UI tells users to read earlier history at the desk. The
  client stays “catching up” until a snapshot arrives and retains at most 200 turns
  during live updates. This bounds turn count, not the byte size of a single turn.
  `Turn = {id, role, text}`: stable nonempty string ID, role `agent` or `user`, and
  markdown text. The server omits tool calls and assistant Markdown code blocks
  (fenced or indented) before sending snapshots or live turns. Empty filtered turns
  are omitted; surrounding prose, inline code, and operator messages remain intact.
  Tool results were already excluded by the shared transcript reader. A `turn` replaces the same ID or appends a new turn. Status
  replaces selected-session metadata and must match the stream's session ID.
  EventSource owns reconnection; bounded snapshots avoid a cursor retention and
  restart protocol. Unchanged Markdown bubbles are memoized by text.
- `POST /api/sessions/:id/message`, JSON `{text: string}` → **200** when accepted
  (body ignored; mock returns `{}`). Text must be nonempty. Optimistic bubbles
  remain visible as “Sent” after 200 until a new user transcript turn with matching
  text replaces one local bubble, in send order. Previously received turn IDs are
  remembered within the recent-turn window so reconnect snapshots do not consume
  another identical send. An echo before 200 also replaces its local bubble.
  There is no client ID/cross-device correlation, durable idempotency map, or
  automatic retry. A failed/unconfirmed POST restores the draft, preserving new
  text typed during the request. Check the transcript before resending: a network
  failure can occur after server acceptance. Local sent bubbles are memory-only;
  unmatched bubbles remain until leaving the chat. There is no 409 draft guard or
  shared watcher lock.
- `GET /api/push/vapid` → `200 {public_key: string}`, base64url uncompressed
  application server key. `POST /api/push/subscribe` accepts browser subscription
  JSON (`endpoint`, `keys.p256dh`, `keys.auth`; expirationTime may be present),
  returns 200 with empty body. Both require the remote server's authorization;
  503 means initialize push and restart the server first. Subscription JSON is
  capped at 4096 bytes. The local mock responds 501.
- Push payload: `{session_id: string, project: string, reason: string}`; no
  transcript. Notifications contain plain text, and clicks construct only the
  same-origin `/#/sessions/<encoded-id>` URL. Existing windows are navigated/focused;
  new windows use that URL. Invalid payloads still show a generic visible notification.


- `GET /api/projects/:project/workers/:id/events` and `POST .../message` use the
  lead chat schemas and limits; worker SSE emits snapshot/turn, without status.
  Lost workers have readable transcripts; sending returns 503. Unknown IDs return 404.
- `PUT /api/projects/:project/session` starts the saved manifest's default lead,
  bypassing the role picker. Already running returns 200. Start returns 409 with one
  fixed plain-text body: `Project directory missing`, `No saved manifest; start once
  at the desk`, or `Tmux session name held by a foreign session`.
- `DELETE /api/projects/:project/session` closes workers then stops the tagged project
  session; `DELETE /api/projects/:project/workers/:id` closes a live or lost worker.
  Both return 200 on success, 404 when already gone/unregistered. Stop/Close ask for
  confirmation. Actions refetch the menu; they do not update it optimistically.

An incompatible `api_version` is detected before parsing version-specific session
data. Malformed JSON/events surface errors; an invalid stream closes and disables
sending, with an explicit reconnect action. Links open in a separate tab with
`noopener noreferrer`. The server's errors are status codes except fixed plain-text 409 start conflicts;
all are rendered as text, never trusted HTML.

## Notifications

After [server initialization](../docs/remote.md#install-the-backend-and-shell-on-the-host),
choose **Enable notifications** in this app; permission is requested
only in response to that button. Existing browser subscriptions are re-registered
with the host; a failed save is visible and can be retried. The UI never claims
success before the server accepts registration. Reopening the app shows the enable
button again so the server can recover registration if necessary.

On iOS 16.4+, add the PWA to the home screen first. Every push displays a notification,
including a malformed/empty payload. Valid notifications show the session ID in the
title and project/reason in the body. A new push replaces the active notification
for that session without alerting again; different sessions remain separate. Once
dismissed, the next push can alert again. Delivery can arrive with Tailscale off via the
public push service; opening the session and all API operations still need Tailscale.
An ended session link reports that it has ended. No transcript is stored by push or
in offline caches. Notification clicks navigate the app, so finish any draft first.
Dismiss any notifications left by the older, untagged app version once after upgrading.

The hamburger menu lists all registered projects and workers grouped by role,
including lost workers. Selecting a project opens its live lead chat; the project
name is remembered in local storage across visits and lead restarts. Notification
session links still open the corresponding project. With no selection, the main
screen shows the Niles logo and invites opening projects on the sidebar. Worker
rows open direct chat; Start launches the default lead and Stop/Close end sessions/workers.
The local mock refuses control and worker chat with 501.
