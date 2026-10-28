# Remote access over Tailscale

Phone access is shell access on the Niles host: the foreground agent can run commands
and spawn workers that bypass approval prompts. Niles adds no sandbox. Serve this
PWA **from the Niles host, on the API's origin**, never from a public frontend/CDN.
Whoever controls its JavaScript can send instructions to that agent. Same-origin
serving keeps this trust boundary at the host/tailnet and needs no CORS grant.
See the [threat model](security.md#remote-api).

With a remote-enabled binary and matching built web assets installed, configure the
host with:

```sh
niles remote setup --web-dir /absolute/path/to/web --allow-user you@example.com \
  --subject mailto:you@example.com
```

This merges authorization, initializes push if needed, starts a user service, mounts
the same-origin shell/API through Tailscale Serve, and attempts API verification.
Installation and tailnet prerequisites follow; see [setup details](#configure-with-niles-remote-setup).

## Prerequisites

Use a release containing the remote feature. Prebuilt release binaries omit it;
build with `--features remote`. The host needs Rust 1.89+ build prerequisites and system
OpenSSL development files (no vendoring); see the [OpenSSL build instructions](https://docs.rs/openssl/0.10.81/openssl/#automatic).
The checkout installer needs Node 24 and pnpm 10.8.1; see the [web prerequisites](../web/README.md).

Install Tailscale on the host and phone, join the tailnet, and restrict access to
this host's TCP 443 to the intended users/devices with tailnet ACLs/grants. Tailnet
membership plus these rules register the device; Niles adds no device password.
Tailscale handles roaming networks and DERP relay connections. The phone must have
a user identity: tagged clients do not get the identity header Niles requires.

Enable MagicDNS/HTTPS at https://login.tailscale.com/admin/dns before setup. It provisions HTTPS at `https://<host>.<tailnet>.ts.net`.
On macOS, serving directories requires the open-source `tailscaled` variant;
sandboxed GUI clients cannot serve the directory in this setup. These commands use
the modern Serve CLI, checked with 1.96.2. See [Serve prerequisites/limitations](https://tailscale.com/docs/features/tailscale-serve).

## Install the backend and shell on the host

Replace `X.Y.Z` with the chosen release version. Use its matching web asset:

```sh
NILES_VERSION=X.Y.Z
cargo install niles --locked --version "$NILES_VERSION" --features remote
NILES_WEB_DIR="$HOME/.local/share/niles/web/$NILES_VERSION"
mkdir -p "$NILES_WEB_DIR"
curl --fail --location \
  "https://github.com/Rhovian/niles/releases/download/v${NILES_VERSION}/niles-web.tar.gz" \
  --output "$NILES_WEB_DIR/niles-web.tar.gz"
tar -xzf "$NILES_WEB_DIR/niles-web.tar.gz" -C "$NILES_WEB_DIR"
rm "$NILES_WEB_DIR/niles-web.tar.gz"
```

For a checkout, build and install the remote-enabled binary and matching shell in
one step from the checkout root:

```sh
scripts/install-remote.sh
```

Then use `--web-dir "$PWD/web/dist"` for setup (`web/dist` also works), or set
`NILES_WEB_DIR="$PWD/web/dist"` for the commands below. Serve only the built assets;
treat this checkout and its build commands as executable host code.

The archive contains the contents of `dist/`, not a surrounding `dist` directory;
`$NILES_WEB_DIR/index.html` must exist. Serve only this dedicated shell directory,
never the repository, home directory, or `~/.niles/` state/keys. Treat the release
asset and its update source as executable host code. The web build is independent
of Cargo; the API version is separate from the CLI JSON schema.

## Configure with `niles remote setup`

After installing the backend and matching web asset above, run the setup command
at the top of this guide with `--web-dir "$NILES_WEB_DIR"`. Use your ordinary user
with permission to configure `tailscaled`, not root. The directory must contain
only trusted built web assets; setup checks for index.html, not directory contents.
Stop any manually running backend before installing the user service.

Repeat `--allow-user` to add more exact `Tailscale-User-Login` identities. Setup
merges them into `~/.niles/config.yaml`, preserving other settings and existing users.
It rejects empty/invalid logins. Omit `--subject` for chat without push; when supplied,
setup reuses `niles remote init` only if keys are absent and keeps existing keys and
subscriptions. `niles remote init --subject mailto:you@example.com` remains available
for adding push later; restart the service afterward.

Run `niles remote doctor` at any time to repeat the read-only prerequisite checks;
it needs no web directory or allowlist and works after setup. Setup runs the same
checks before writing config, keys, service files, or Serve state. The checks stop
at the first failure with commands or an admin-console link to fix it:

- The `niles` selected by PATH must resolve to the running executable. If an older
  installation shadows it, put the new binary's directory first on PATH and run
  `hash -r`. If `niles remote doctor` itself reports “built without remote”, use the
  absolute path of the binary installed with `--features remote`; inspect `type -a niles`.
- CLI and daemon releases must match. Release build hashes in the daemon version
  are accepted. Repair stale GUI wrappers using `type -a tailscale` and reinstall
  the Homebrew CLI if needed.
- On macOS, remove the GUI app before using Homebrew `tailscaled`. The Homebrew
  LaunchDaemon plist must give PATH access to the selected CLI directory. Doctor
  prints the exact PlistBuddy and bootout/bootstrap commands when PATH is missing.
  `brew services restart tailscale` regenerates this plist and can revert the fix;
  rerun doctor afterward. Plist inspection describes the on-disk configuration;
  bootstrap it after editing so the running daemon uses it.
- The current user must be the Tailscale operator and, on macOS, an administrator.
  Set the operator with `sudo tailscale set --operator="$(id -un)"`.
- The host DNS name must have an HTTPS certificate domain in Tailscale status.

Setup and doctor never run sudo. Custom sudoers policy cannot be proven by these
read-only checks: doctor prints `sudo --other-user="$(id -un)" --list tailscale`
for manual confirmation. The daemon performs that check when serving a directory;
its environment must resolve `tailscale` too.

Setup requires Tailscale 1.96.2 or newer with a three-component numeric version
(suffixed/development versions are unsupported). It rejects unknown Serve schema
fields and existing background `/`, `/api`, or `/api/...` handlers on HTTPS 443.
Other mounts remain in place; Tailscale applies its own remaining Serve rules.
Setup installs and starts a user service using this executable's absolute path and
current HOME/PATH:

- macOS: `~/Library/LaunchAgents/dev.niles.remote.plist` in the logged-in GUI domain.
- Linux: `~/.config/systemd/user/niles-remote.service`. To run without login, enable
  lingering with `loginctl enable-linger "$USER"` (administrator permission may be needed).

An existing service file is never overwritten. If service activation fails, setup
removes the file it just created so activation can be retried. Config/key changes
remain in place. `--port 9123` selects another nonzero loopback port for both service
and proxy. The service manager keeps `niles remote` running; no tmux pane is needed.

Setup mounts static `/` and proxies `/api` to `http://127.0.0.1:PORT/api`. The target's
`/api` suffix matters: Serve strips the mount prefix before joining the target URL.
The API stays on IPv4 loopback; exposing it on a LAN/tailnet IP would allow forged
identity headers. Serve injects verified identities and strips caller-supplied ones.
Use **Serve, never Funnel**, and keep Tailscale available on the host.

Setup attempts HTTPS `/api/sessions` verification with curl 7.84+: HTTP 200,
`Cache-Control: no-store`, expected API version and a sessions array. It sends no
identity header and follows no redirects. **Verification failure is a warning**;
installed configuration stays active and setup succeeds. Host self-resolution,
a tagged/disallowed host identity, an older curl, or service startup delay can
prevent this check even when the phone can connect. Follow the printed `curl -i`
command from an allowed user-owned device on the tailnet. Expect the same response
contract; do not supply an identity header yourself. Direct unauthenticated requests
to the loopback API return 403 by design.

Setup does not join devices, change tailnet policies, build the PWA, or install the
binary. Complete these manual steps before setup: join host and phone to the tailnet,
enable MagicDNS/HTTPS, restrict TCP 443 with ACLs/grants, and on macOS install the
open-source `tailscaled` variant. After setup, open the printed URL on the phone and
install/Add to Home Screen; enable notifications in the installed app for push.

Failures other than advisory verification stop setup and leave completed steps in
place. Diagnostics name the failed command and recommend `niles remote uninstall`
before retrying the same setup command. Inspect `tailscale serve status` and the
service first: uninstall removes the service and exact `/` and `/api` mappings on
HTTPS 443, so use it only when those belong to this setup. To upgrade, run
`niles remote uninstall`, then rerun setup with the newly installed executable and
matching dedicated web directory.
Config changes require a service restart (`launchctl kickstart -k gui/$(id -u)/dev.niles.remote`
on macOS; `systemctl --user restart niles-remote` on Linux).

## Install on the phone

Open `https://HOST.TAILNET.ts.net/` on the phone. Select a live session; its agent
family and entry skill (including `solo`) appear together. Chat becomes live after
the SSE snapshot. Send a harmless message and verify it in the desktop transcript.
There is no draft/prompt guard or watcher send lock: mid-turn input is sent anyway.
A failed acknowledgement can still mean delivery; inspect before resending.

Install using the browser's install/Add to Home Screen control. On iOS 16.4+, push
requires the installed home-screen app. Choose **Enable notifications** there.
Push arrives over the public internet even with Tailscale off; opening its session
link still needs Tailscale. See [PWA notification behavior](../web/README.md#notifications).

## Chat and project control

Phone transcripts omit tool calls and assistant Markdown code blocks on the server,
including diffs and file contents inside those blocks. Replies left empty are omitted;
surrounding prose, inline code, and operator messages remain. Complete transcripts
remain available at the desk. This is a display filter, not secret redaction.

Tap a worker in the menu to read its live transcript and send directly to its recorded
window. The lead is not notified and its worker wait is not re-armed. Lost workers
remain readable but sending returns 503. Close works for live and lost workers.

Start appears for registered projects that are not running. It requires a saved,
parsing manifest: start once at the desk to choose skills. Phone starts always use
that manifest's default lead and skip the picker (`niles start --keep-skills`); desk
starts still show it. Stop and Close require confirmation. The API routes are:

- `GET /api/projects/:project/workers/:id/events` and `POST .../message` use the
  lead chat schemas and limits; worker SSE emits snapshot/turn, without status.
  Lost workers have readable transcripts; sending returns 503. Unknown IDs return 404.
- `PUT /api/projects/:project/session` starts the saved manifest's default lead,
  bypassing the skill picker. Already running returns 200. Start returns 409 with one
  fixed plain-text body: `Project directory missing`, `No saved manifest; start once
  at the desk`, or `Tmux session name held by a foreign session`.
- `DELETE /api/projects/:project/session` closes workers then stops the tagged project
  session; `DELETE /api/projects/:project/workers/:id` closes a live or lost worker.
  Both return 200 on success, 404 when already gone/unregistered. Stop/Close ask for
  confirmation. Actions refetch the menu; they do not update it optimistically.

A launch that fails afterwards appears as not running; the API does not diagnose
agent login/trust dialogs or missing credentials. Agents inherit the service/tmux
server environment, so shell-rc-only credentials may be missing. Stop with many workers
can take tens of seconds; proxy timeout behavior is unverified. The server continues
an in-flight operation after disconnect, and retrying is safe. Concurrent desk/lead
changes can leave lost workers after stop; close those from the menu.

## Operations and troubleshooting

- 403: confirm the exact allowed login, restart after config edits, check tailnet
  access and use a user-owned rather than tagged client. The backend requires one
  matching identity header; a local/public reverse proxy cannot invent that identity.
- 404 at `/api/sessions`: check the API target ends in `/api`. HTML instead of JSON
  means the request reached the static handler. 502: check `niles remote` is running
  and the proxy port matches `--port`.
- No sessions: only registered running sessions appear. A 404 for a particular ID
  means it ended/changed. SSE reconnects fetch a fresh snapshot of up to 200 turns;
  earlier history remains at the desk, and transcripts are never cached offline.
- Message 503: the live session lacks a recorded agent pane. Push 503: initialize
  VAPID and restart. Blank/ESC/NUL text, extra message fields or a JSON message body
  over 16 KiB are rejected; subscription bodies are limited to 4096 bytes.
- Notifications: endpoints are restricted to Google/FCM, Mozilla and Apple at the
  exact hosts listed in the [threat model](security.md#remote-pwa-frontend), HTTPS/443,
  no redirects. Payload is session ID/project/generic reason, never transcript.
  Five-minute TTL; 404/410 prunes a device. Other provider/store/catalog errors stop
  the server: inspect its diagnostics, fix the cause and restart. No automatic retry.
  The PWA shows the project name in the title and project/reason in the body. Repeated
  pushes replace the session's active notification without alerting again; different
  sessions remain separate. After dismissal, the next push can alert again.
  Startup can notify already-waiting sessions again, replacing any active notification
  for that session. Device-owner revocation takes effect after restarting with the
  changed allowlist.
- Upgrade backend and trusted web asset together. Extract the new shell into its own
  version directory and repoint static `/`. For a checkout serving `web/dist`, rerun
  `scripts/install-remote.sh`, then restart the service using the commands above.
  This replaces the live shell in place before the new binary starts. Close all
  Niles windows, then reopen so the installed worker can activate. API mismatch
  blocks chat with an update banner, including during the checkout upgrade.

To stop and remove the generated user service and its two Serve mappings:

```sh
niles remote uninstall
```

This leaves other Serve mounts (including other ports and `/api/...` handlers),
`~/.niles/config.yaml` authorization, and push keys/subscriptions in place. It skips
missing service files and missing mounts, so it also handles partial setup. It
reads Serve status, stops/removes the service, then removes each existing mount.
Other failures stop uninstall and name the failed command; completed removals stay
in place. If a service file exists but the service is already stopped, service-manager
errors are reported rather than ignored; inspect the file and remove it manually
before retrying (`~/Library/LaunchAgents/dev.niles.remote.plist` on macOS,
`~/.config/systemd/user/niles-remote.service` on Linux).

Uninstall needs no web assets, allowlist, or setup preflight. Keep Tailscale available
until it completes. Never use `tailscale serve reset` for this: it removes unrelated
Serve configuration too.

## Known framing gap

The shell carries strict **meta CSP**, with no inline scripts; markdown is sanitized
on every changed render. Meta CSP cannot enforce `frame-ancestors`, and direct
`tailscale serve` directory hosting cannot add that response header. Clickjacking
protection is therefore a known gap of this deployment, not a solved requirement.
A same-host HTTP server behind Serve could add the header, but is an alternative
deployment. Keep the shell's CSP and the host/origin trust boundary intact.
