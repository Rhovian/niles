# Remote access over Tailscale

Phone access is shell access on the Niles host: the foreground agent can run commands
and spawn workers that bypass approval prompts. Niles adds no sandbox. Serve this
PWA **from the Niles host, on the API's origin**, never from a public frontend/CDN.
Whoever controls its JavaScript can send instructions to that agent. Same-origin
serving keeps this trust boundary at the host/tailnet and needs no CORS grant.
See the [threat model](security.md#remote-api).

## Prerequisites

Use a release containing the remote feature. Prebuilt release binaries omit it;
build with `--features remote`. The host needs Rust 1.89+ build prerequisites and system
OpenSSL development files (no vendoring); see the [OpenSSL build instructions](https://docs.rs/openssl/0.10.81/openssl/#automatic).
Node/pnpm are only needed if [building the PWA yourself](../web/README.md).

Install Tailscale on the host and phone, join the tailnet, and restrict access to
this host's TCP 443 to the intended users/devices with tailnet ACLs/grants. Tailnet
membership plus these rules register the device; Niles adds no device password.
Tailscale handles roaming networks and DERP relay connections. The phone must have
a user identity: tagged clients do not get the identity header Niles requires.

Enable MagicDNS/HTTPS for the tailnet; Serve's setup flow provides an enablement
link if needed. It provisions HTTPS at `https://<host>.<tailnet>.ts.net`.
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

For a checkout, install with `cargo install --path . --locked --features remote`,
build the [PWA separately](../web/README.md) with `pnpm build` in `web/`, and set
`NILES_WEB_DIR` to its absolute `web/dist` path instead of downloading an asset.

The archive contains the contents of `dist/`, not a surrounding `dist` directory;
`$NILES_WEB_DIR/index.html` must exist. Serve only this dedicated shell directory,
never the repository, home directory, or `~/.niles/` state/keys. Treat the release
asset and its update source as executable host code. The web build is independent
of Cargo; the API version is separate from the CLI JSON schema.

Merge an explicit allowlist into `~/.niles/config.yaml`, preserving other settings:

```yaml
remote:
  allowed_users: [you@example.com]
```

Use the exact `Tailscale-User-Login` identity, not a device name. Niles refuses an
empty/invalid list. Authorization applies to every API route; the static shell is
available to devices permitted by the tailnet rules. Config changes require restart.

If using notifications, initialize once before starting the server:

```sh
niles remote init --subject mailto:you@example.com
```

This writes private VAPID/device state under `~/.niles/remote/`; repeated init fails
instead of rotating existing keys. Skip it to use chat without push. Now run:

```sh
niles remote --port 8765
```

It prints `remote listening on 127.0.0.1:8765`. Keep this process running in tmux or
your service manager. The listener is **IPv4 loopback only**; do not expose it on
`0.0.0.0`, the LAN, or a tailnet IP. Only the local Serve proxy should reach it:
otherwise clients could forge the trusted identity header.

## Serve static `/` and proxy `/api`

Inspect existing mappings first; these commands replace `/` and `/api` on port 443.
Run as a user permitted to configure `tailscaled` (use sudo if required).
Keep `NILES_WEB_DIR` from the installation shell, or set it to that absolute path.

```sh
tailscale serve status
tailscale serve --bg --https=443 --set-path=/ "$NILES_WEB_DIR"
tailscale serve --bg --https=443 --set-path=/api http://127.0.0.1:8765/api
tailscale serve status
```

The `/api` suffix on the **target** is required: Serve removes the mount prefix,
then joins the remaining path to the target URL. Thus `/api/sessions` reaches
`127.0.0.1:8765/api/sessions`; a target ending at `:8765` would reach `/sessions`
and return 404. The more specific API mount wins over the static `/` handler.
This behavior is in [Tailscale 1.96.2's proxy implementation](https://github.com/tailscale/tailscale/blob/v1.96.2/ipn/ipnlocal/serve.go#L1097);
see also the [Serve CLI reference](https://tailscale.com/docs/reference/tailscale-cli/serve).

Serve supplies the verified identity header to the proxy and removes caller-supplied
identity headers. Do not add an untrusted proxy or forward public identity headers.
Use **Serve, never Funnel**: Funnel publishes to the internet and does not supply
the user identity this API requires. `--bg` persists Serve across restarts; it does
not start or supervise `niles remote`. No public frontend, port-forward or CORS setup
is needed. Tailscale must remain available on the host.

## Verify and install

From a user-owned device connected to the tailnet, replace the hostname below:

```sh
curl -i https://HOST.TAILNET.ts.net/api/sessions
```

Expect HTTP 200, `Cache-Control: no-store`, and JSON with `api_version: 1` and a
`sessions` array (possibly empty). Do not supply an identity header yourself; this
checks Serve's identity injection and path mapping. Direct unauthenticated requests
to `http://127.0.0.1:8765/api/sessions` return 403 by design.

Open `https://HOST.TAILNET.ts.net/` on the phone. Select a live session; its agent
family and entry skill (including `solo`) appear together. Chat becomes live after
the SSE snapshot. Send a harmless message and verify it in the desktop transcript.
There is no draft/prompt guard or watcher send lock: mid-turn input is sent anyway.
A failed acknowledgement can still mean delivery; inspect before resending.

Install using the browser's install/Add to Home Screen control. On iOS 16.4+, push
requires the installed home-screen app. Choose **Enable notifications** there.
Push arrives over the public internet even with Tailscale off; opening its session
link still needs Tailscale. See [PWA notification behavior](../web/README.md#notifications).

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
  Startup can notify already-waiting sessions again; device-owner revocation takes
  effect after restarting with the changed allowlist.
- Upgrade backend and trusted web asset together. Extract the new shell into its own
  version directory and repoint static `/`. Close all Niles windows, then reopen so
  the installed worker can activate. API mismatch blocks chat with an update banner.

To remove only these Serve handlers, preserving unrelated mounts:

```sh
tailscale serve --bg --https=443 --set-path=/api off
tailscale serve --bg --https=443 --set-path=/ off
```

Stop `niles remote` separately. Avoid `tailscale serve reset` on a host with other
mappings; it removes the whole Serve configuration.

## Known framing gap

The shell carries strict **meta CSP**, with no inline scripts; markdown is sanitized
on every changed render. Meta CSP cannot enforce `frame-ancestors`, and direct
`tailscale serve` directory hosting cannot add that response header. Clickjacking
protection is therefore a known gap of this deployment, not a solved requirement.
A same-host HTTP server behind Serve could add the header, but is an alternative
deployment. Keep the shell's CSP and the host/origin trust boundary intact.
