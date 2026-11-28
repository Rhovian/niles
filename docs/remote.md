# Remote access over Tailscale

An `operate` grant gives shell authority on the host through its agents. Serve the PWA
from the relay on your tailnet. Each Niles host verifies issuer tokens and applies project
grants. Read the [threat model](security.md#remote-api) first.

For chat apps, CI and other MCP clients, see [remote MCP](remote-mcp.md).

## Prerequisites

- A Niles binary built with `--features remote`; release binaries omit it. Building needs Rust
  1.89+ and system OpenSSL development files ([OpenSSL build instructions](https://docs.rs/openssl/0.10.81/openssl/#automatic)).
  Building the app from a checkout also needs Node 24 and pnpm 10.8.1
  ([web prerequisites](../apps/pwa/README.md)).
- Tailscale 1.96.2 or newer on the host and phone, both on your tailnet, with tailnet ACLs or
  grants restricting the relay's TCP 443 and the backend ports to the intended peers.
- An HTTPS issuer that supports browser-based OAuth authorization code with S256 PKCE,
  OpenID Connect discovery, and bearer access tokens with host-specific audiences. Niles
  supports JWTs and opaque tokens with RFC 7662 introspection.
- MagicDNS and HTTPS enabled in the [Tailscale DNS settings](https://login.tailscale.com/admin/dns).
- On macOS, the open-source `tailscaled` from Homebrew instead of the GUI app, which can't
  serve a directory. See [Serve's requirements](https://tailscale.com/docs/features/tailscale-serve).

## Install the backend and shell on the host

From a release, replace `X.Y.Z` with the version and install its matching web asset:

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

From a checkout, run `scripts/install-remote.sh` in its root, then use
`NILES_WEB_DIR="$PWD/apps/pwa/dist"`.

The web directory must contain only the built app, with `index.html` at its top. Never point it
at the repository, your home directory or `~/.niles/`. Treat the app and wherever you get it
from as code running on your host.

## Configure with the remote-setup skill

Use the [remote-setup skill](../skills/remote-setup/SKILL.md) with an agent that can access
these machines. It is an ordinary disk skill: copy its directory to
`~/.niles/skills/remote-setup/` to use it across projects, or follow the linked instructions
in an attended agent session. Give it the relay and host inventory, issuer and client details,
per-host project grants and push contact URI. The skill covers
fresh installs and upgrades from relay identity.

The relay stays awake and also runs Niles. Tailscale Serve terminates HTTPS and forwards to
Caddy on `127.0.0.1:8080`. Caddy serves the built PWA and `hosts.json`, strips `/h/<id>` from
host API requests, and forwards them to each host's tailnet IPv4 port (default 8765).
Serve cannot proxy directly to other tailnet IPs because its dialer bypasses Tailscale routes.
The phone uses one HTTPS origin for the shell and all Niles API requests. Use Serve on the
tailnet. The browser also contacts the issuer, where discovery and token exchange must
allow CORS from the PWA origin when made by the browser. Set a Caddy response header on the shell with
`Content-Security-Policy: connect-src 'self' https://login.example.com`. Substitute the
actual discovery and token endpoint origins.

### Configure tsidp

Keep tsidp on the tailnet and use its HTTPS issuer URL. Register a separate confidential client for
each Niles host in the tsidp admin UI. Give each client the exact PWA origin
followed by `/` as its redirect URI. Use a user-owned tailnet device for phone sign-in.
Configure the `tailscale.com/cap/tsidp` application capability on tsidp for the
intended phone users. Set `users` to their login names and `resources` to each
exact Niles API resource, including the relay if it runs Niles. Use a separate
admin grant for client registration. Allow each backend to reach tsidp over HTTPS.
[tsidp capability setup](https://github.com/tailscale/tsidp/blob/v0.0.21/README.md#setting-an-application-capability-grant)

Serve a noncached `hosts.json` beside the shell:

```json
{
  "issuer": "https://idp.example.ts.net",
  "host_token_exchange": true,
  "hosts": [
    {"id": "relay", "name": "Relay", "client_id": "relay-client-id"},
    {"id": "desktop", "name": "Desktop", "client_id": "desktop-client-id"}
  ]
}
```

On each host, including the relay, merge this into its `~/.niles/config.yaml`.
Use the host’s own bind address, resource, and client ID from the manifest.
Put only that client’s secret in the named variable in its Niles service environment.

```yaml
remote:
  web:
    bind: 100.64.0.2
    resource: https://relay.example.ts.net/h/desktop/api
  introspection:
    issuer: https://idp.example.ts.net
    client_id: desktop-client-id
    client_secret_env: NILES_PWA_CLIENT_SECRET
  issuers:
    https://idp.example.ts.net:
      - {subject: '123456', project: demo, access: operate}
      - {subject: '654321', project: demo, access: read}
```

Use each user’s exact decimal Tailscale user ID as the grant subject. The PWA
uses PKCE and sends the authorization code and verifier to the selected host.
That host redeems the code and refreshes tokens using its own secret and fixed
resource. The PWA keeps each host’s access and refresh tokens across launches.
When an access token expires, the app uses that host’s refresh token and saves
the replacement returned by tsidp. An invalid or expired refresh grant requires
sign-in again. Each host authenticates to HTTPS introspection with its own client credentials.
Niles requires the introspected client ID to match the host’s registration and
the audience to name exactly that host resource. tsidp also includes the client
ID in the audience. [tsidp token implementation](https://github.com/tailscale/tsidp/blob/v0.0.21/server/token.go)

### Other issuers

For a JWT issuer with a public browser client, put its `client_id` on every host
entry and omit `host_token_exchange` and `remote.introspection`. Register
the same redirect URI and use authorization code with S256 PKCE. Issue signed access tokens with the exact host resource as their
sole audience. Hosts can configure one opaque issuer alongside JWT issuers. Other
opaque issuers must advertise HTTPS introspection and token endpoints and support
`client_secret_post` on both. An active introspection result must include `active`, `iss`,
`sub`, `exp`, `aud`, and the issuing `client_id`. The host verifies JWT issuers locally, including GitHub Actions.

For a direct single-host setup, keep the same manifest fields and set `hosts` to
one entry with an empty `id` and the same client fields as above. Route `/api/*`
to the host and use `https://<pwa-origin-host>/api` as its resource. Projects and transcripts are visible
with either grant. Starting and stopping sessions, messaging and closing workers require
`operate` on that project. The web and MCP listeners on a host share `remote.issuers`,
and each listener requires its own resource audience. Tailnet policy continues to restrict
network reachability. Caddy preserves the `Authorization` header on every proxied API route.

The skill configures tags and grants, explicit Caddy routes, a noncached host manifest,
shared VAPID keys, and OS user services. It verifies the
API through the relay and tests denied access. Its Caddyfile and service files live outside
the public asset directory. Never put private keys or Niles state in the PWA directory.

## Install on the phone

Open `https://<relay>.<tailnet>.ts.net/` on the phone. Choose **Sign in to** for each host
you want to open and complete sign-in in the same app window. The app reuses each host’s
credentials across launches and refreshes through the selected host when available. Select a live session and send
a harmless message with an `operate` grant. Check that it arrives at the desk. Use the
browser's Add to Home Screen to install the app. On iOS 16.4+, use **Enable notifications**
from the installed app after signing in to all hosts. Notifications arrive even with
Tailscale off, but opening one needs Tailscale. See [notification behavior](../apps/pwa/README.md#notifications).

## Using it

Open **Telemetry** in the PWA menu for token usage across the machines in `hosts.json`, grouped
by machine, project, session and model over rolling windows of 1, 7 or 30 days. These are
24-hour days, unlike the CLI panel’s local-midnight calendar ranges. The overlapping 15-minute
bucket at the beginning of a window is included. **Refresh telemetry** fetches
each machine independently. An unreachable machine retains its last successful snapshot with an
“As of” timestamp; relay gateway failures (502/503/504) and request timeouts are classified as unreachable.
A network failure reaching the relay shows one connection notice suggesting reconnecting Tailscale,
while other API failures are shown as Failed alongside any cached usage. Totals include these stale
snapshots, so check collection times when comparing machines. The browser keeps each
snapshot across launches until sign-in clears it.
An outdated or malformed cache is discarded with a notice and refreshed from the server.
Browser storage failures are reported explicitly while fresh fetched usage remains visible.
The CLI TELEMETRY panel remains local-only.

Each host serves `GET /api/telemetry` through the existing relay route. The versioned JSON contains
`collected_at` and `sessions`; each session has `project`, `id`, `entry`, `family`, nullable `model`,
`created_at`, nullable `closed_at`, and `buckets` mapping UTC 15-minute bucket starts to token counts.
Token accounting follows the local TELEMETRY panel, including input, output and cache tokens.
The endpoint bounds closed-session reads and exported buckets to the last 30 days, including
the overlapping start bucket, rather than exporting all-time history. The `created_at` field
distinguishes incarnations when a worker ID is reused. Historical coverage is limited to
registered projects and retained Niles state/transcripts; no transcript
text is returned. Run matching web/backend releases on every host.

- Messages are sent even mid-turn. A failed send can still have been delivered, so check before
  resending.
- Phone transcripts hide tool calls and code blocks, including diffs and file contents; the full
  transcript stays at the desk. This is a display filter, not secret redaction.
- Tap a worker to read its transcript and message it directly. The entry agent isn't told.
- Start runs a stopped project with its saved manifest, skipping the skill picker; start it once
  at the desk first to save one. Stop and Close ask for confirmation. Stopping a project with many
  workers can take tens of seconds.
- Agents started from the phone get the service's environment, so credentials set only in your
  shell profile may be missing. A start that fails shows the project as not running.

## Troubleshooting

- For HTTP 401, sign in to the host again and check the issuer, token expiry and audience.
- For HTTP 403, check the issuer, subject and project access in the host’s grants.
- 404 at `/h/<id>/api/sessions`, or HTML instead of JSON: check the manifest ID and Caddy
  `handle_path /h/<id>/*` route. Strip only `/h/<id>`, retaining `/api/...`.
- 502: `niles remote` isn't running, or the proxy port doesn't match `--port`.
- No sessions: only registered, running projects appear.
- Message 503: the session has no recorded agent pane. Push 503: run `niles remote init`, then
  restart.
- A device stops getting notifications: a 404 or 410 from its push service removes it. Other push
  errors stop the server: read its log, fix the cause and restart.

## Restart, upgrade and uninstall

Run `niles doctor` to see the documented user service's configured executable, whether that
file includes `remote`, and whether the running service uses it. Older binaries that omit
features from `--version` report `unknown`. If doctor reports a different
running binary, first make sure the configured executable includes `remote`, then restart the
service. For a remote-enabled binary, `niles update` preserves the feature in Cargo instructions
and refuses to run the release updater.

Restart after any config or key change:

```sh
launchctl kickstart -k gui/$(id -u)/dev.niles.remote   # macOS
systemctl --user restart niles-remote                   # Linux
```

To upgrade a release install, install matching new binary and web assets as above. For a
checkout, rerun `scripts/install-remote.sh`. Close and reopen every app window after
restarting the services.

When moving from relay identity, prepare the client registrations, resources and grants first.
Stop the old Niles services during the cutover. On every host, remove `remote.relay_node`
and any older `remote.allowed_users` key and apply the `remote.web` and `remote.issuers`
settings above. For tsidp, configure `remote.introspection` with a separate client ID
and secret on each host. Remove any legacy `remote.wake_hosts` entry. Move existing
`remote.mcp.issuers` up to `remote.issuers` and merge its grants.
While the old services are stopped, move `~/.niles/remote/subscriptions` aside on each
host. Niles recreates it for issuer-backed registrations. Keep the existing tailnet policy and
VAPID key. Update Caddy to serve the matching new assets and forward bearer headers.
For tsidp, set the issuer and per-host client IDs in the manifest and enable host
token exchange. For a public client, move any top-level client ID into each host
entry. Remove retired wake routes and manifest entries. Update the shell CSP
with the issuer origins. Validate the Caddy config and restart
Caddy and the Niles services. Test issuer sign-in, a rejected forged-header request,
and cross-host audience rejection. Re-enable notifications
from the upgraded PWA after sign-in to bind each registration to an issuer identity.

To uninstall, inspect the files and services recorded by the setup skill. Stop and remove
only the Niles and dedicated Caddy services it created, then remove only its Serve root mount
with `tailscale serve --bg --https=443 --set-path=/ off`. Preserve unrelated mounts and services,
config and push keys. Never use `tailscale serve reset`. Inspect tailnet policy before removing
Niles tags or grants so unrelated access remains intact.

## Shared push through a relay

Generate the VAPID key once with `niles remote init --subject mailto:you@example.com`.
Securely copy `~/.niles/remote/vapid.json` to that same path on every host (including the relay
if it runs Niles), preserving mode 0600 and a private mode 0700 parent directory. Never put
this private file in the PWA directory. Restart every host after installing the key; do not
initialize independent keys per host.

With the relay's `/hosts.json` listing all hosts, Enable notifications verifies that every
host uses the same key, creates or reuses one browser subscription, and registers it through
each `/h/<id>/api/push/subscribe`. All hosts must be awake during registration. A failed
registration names the host; retry after fixing it. Enable notifications again after migrating from relay identity or adding a host. Notifications display the originating host
and open its session even when other hosts use the same session ID.
