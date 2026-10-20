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
changes follow sends. Push endpoints return 501 pending #318. Mock state disappears
when Vite stops and is never included in the production client.

`pnpm lint`, `pnpm typecheck`, `pnpm test`, and `pnpm build` run in web CI. `pnpm preview` serves a previously built `dist/` without the mock API.
The service worker is registered only in production. Install from the browser's
install/Add to Home Screen control over HTTPS (localhost also works).

The release workflow uploads `niles-web.tar.gz` after cargo-dist announces a release.
Extract it to a dedicated directory on the Niles host. Serve that directory at `/`
with `tailscale serve`, proxying `/api` on the **same origin** to the remote API's
loopback listener. Never host this app on a public frontend service: its JavaScript
can send instructions to an agent with host access. Tailnet authorization and
`Tailscale-User-Login` validation belong to the remote server (#314–316).

Only versioned shell assets are precached. No transcript, API response, or outgoing
message is saved to browser storage. The API must also send `Cache-Control: no-store`
on all responses, including SSE, to prevent HTTP caches retaining transcripts.
The HTML ships a strict CSP without inline scripts/styles. The specified direct-directory `tailscale serve` deployment does not provide a
custom response-header setting. Meta CSP cannot enforce `frame-ancestors`, so
framing protection remains a known deployment gap for #314–316/security review.
A same-host HTTP server behind Serve could supply `Content-Security-Policy:
frame-ancestors 'none'`, but that is an alternative deployment, not a requirement
this static asset can fulfill. Preserve the HTML CSP when deploying.

Updates install in the background and activate when existing app windows close;
close all Niles tabs/app windows, then reopen to load a newer shell. An API mismatch
blocks chat and shows an update banner. Offline mode shows the shell and a tailnet
connection banner; it does not replay transcripts or queue messages for later.

## API v1 contract for #315/#316

`src/api.ts` is the single client contract and runtime JSON parse boundary. All
URLs are same origin. JSON property names use snake_case. IDs are opaque strings;
the client percent-encodes session IDs in URL path segments.

- `GET /api/sessions` → `200 {"api_version":1,"sessions":[Session]}`.
  `Session = {id, project, agent:{family, entry_skill}, waiting, worker_count}`.
  `id`, `project`, `family`, `entry_skill` are strings; `id` is nonempty;
  `waiting` is boolean; `worker_count` is a nonnegative integer. The foreground
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
  full markdown text. A `turn` replaces the same ID or appends a new turn. Status
  replaces selected-session metadata and must match the stream's session ID.
  EventSource owns reconnection; bounded snapshots avoid a cursor retention and
  restart protocol. Unchanged Markdown bubbles are memoized by text.
- `POST /api/sessions/:id/message`, JSON `{text: string}` → **200** when accepted
  (body ignored; mock returns `{}`). Text must be nonempty. Optimistic bubbles
  exist only while POST is in flight and are removed on 200; transcript turns are
  authoritative. No `client_id`, transcript echo/matching, durable idempotency map,
  or automatic retry. A failed/unconfirmed POST restores the draft, preserving new
  text typed during the request. Check the transcript before resending: a network
  failure can occur after server acceptance. There is no 409 draft guard or shared
  watcher lock. A transcript echo arriving before 200 can transiently coexist with
  the pending bubble; no accepted duplicate persists.
- Push endpoints from the epic belong to #318; no unused speculative client types.
  The mock responds 501. Worker hooks show a generic notification and open/focus
  `/`; they ignore payloads and accept no arbitrary target URL.

An incompatible `api_version` is detected before parsing version-specific session
data. Malformed JSON/events surface errors; an invalid stream closes and disables
sending, with an explicit reconnect action. Links open in a separate tab with
`noopener noreferrer`. The server's errors are shown as status codes, never trusted HTML.
