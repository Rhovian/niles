# Remote access over Tailscale

Phone access is shell access on the Niles host: the phone can instruct agents that run commands
as you, and workers bypass approval prompts. Serve the app only from your relay, never from
a public site. Read the [threat model](security.md#remote-api) first.

## Prerequisites

- A Niles binary built with `--features remote`; release binaries omit it. Building needs Rust
  1.89+ and system OpenSSL development files ([OpenSSL build instructions](https://docs.rs/openssl/0.10.81/openssl/#automatic)).
  Building the app from a checkout also needs Node 24 and pnpm 10.8.1
  ([web prerequisites](../web/README.md)).
- Tailscale 1.96.2 or newer on the host and phone, both on your tailnet, with tailnet ACLs or
  grants restricting the host's TCP 443 to your users and devices. The phone must sign in as a
  user, not a tagged device, because tagged clients get no identity header.
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
`NILES_WEB_DIR="$PWD/web/dist"`.

The web directory must contain only the built app, with `index.html` at its top. Never point it
at the repository, your home directory or `~/.niles/`. Treat the app and wherever you get it
from as code running on your host.

## Configure with the remote-setup skill

Use the [remote-setup skill](../skills/remote-setup/SKILL.md) with an agent that can access
these machines. It is an ordinary disk skill: copy its directory to
`~/.niles/skills/remote-setup/` to use it across projects, or follow the linked instructions
in an attended agent session. Give it the relay and host inventory, permitted phone users,
push contact URI, and any Wake-on-LAN targets. It handles fresh installations; migration of
existing remote deployments is out of scope.

The relay stays awake and also runs Niles. Tailscale Serve terminates HTTPS and forwards to
Caddy on `127.0.0.1:8080`. Caddy serves the built PWA and `hosts.json`, strips `/h/<id>` from
host API requests, and forwards them to each host's tailnet IPv4 port (default 8765).
Serve cannot proxy directly to other tailnet IPs because its dialer bypasses Tailscale routes.
The phone uses one HTTPS origin; no CORS configuration is needed. Use Serve, never Funnel.

Every host, including the relay, needs this in its existing `~/.niles/config.yaml`:

```yaml
remote:
  relay_node: nYOUR_RELAY_STABLE_NODE_ID
```

Use `Node.StableID` from `tailscale whois --json <relay-tailnet-ip>`. Caddy must use the relay's
own tailnet IP for its backend too, so WhoIs can authenticate its connection. Hosts require
exactly one forwarded Tailscale login and the configured relay as their TCP peer. There is
no `remote.allowed_users`: tailnet policy restricts phone users to relay TCP 443 and the relay
to host backend ports. Anyone admitted through the relay has shell authority on every host.

The skill configures tags and grants, explicit Caddy routes, a noncached host manifest,
shared VAPID keys, optional server-side WoL targets, and OS user services. It verifies the
API through the relay and tests denied access. Its Caddyfile and service files live outside
the public asset directory. Never put private keys or Niles state in the PWA directory.

## Install on the phone

Open `https://<relay>.<tailnet>.ts.net/` on the phone, select a live session and send a harmless
message; check it arrives at the desk. Then use the browser's Add to Home Screen. On iOS 16.4+,
push needs the installed app: choose Enable notifications there. Notifications arrive even with
Tailscale off, but opening one needs Tailscale. See [notification behavior](../web/README.md#notifications).

## Using it

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

- 403: check the request comes through the configured relay and the phone isn't a tagged device.
- 404 at `/h/<id>/api/sessions`, or HTML instead of JSON: check the manifest ID and Caddy
  `handle_path /h/<id>/*` route. Strip only `/h/<id>`, retaining `/api/...`.
- 502: `niles remote` isn't running, or the proxy port doesn't match `--port`.
- No sessions: only registered, running projects appear.
- Message 503: the session has no recorded agent pane. Push 503: run `niles remote init`, then
  restart.
- A device stops getting notifications: a 404 or 410 from its push service removes it. Other push
  errors stop the server: read its log, fix the cause and restart.

## Restart, upgrade and uninstall

Restart after any config or key change:

```sh
launchctl kickstart -k gui/$(id -u)/dev.niles.remote   # macOS
systemctl --user restart niles-remote                   # Linux
```

To upgrade a release install, install matching new binary and web assets as above, update
Caddy's asset directory, validate its config and restart the affected services. To upgrade a
checkout, rerun `scripts/install-remote.sh` and restart. Close and reopen every Niles app
window. Upgrade the binary and app together: a version mismatch blocks chat.

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
registration names the host; retry after fixing it. Existing registrations remain valid.
Enable notifications again after adding a host. Notifications display the originating host
and open its session even when other hosts use the same session ID.

## Wake-on-LAN through the relay

Keep the relay awake on the same LAN as the sleeping hosts and enable Wake-on-LAN
in each host's firmware and OS. Add targets to the relay's `~/.niles/config.yaml`:

```yaml
remote:
  relay_node: nYOUR_RELAY_STABLE_NODE_ID
  wake_hosts:
    desktop:
      mac: "01:23:45:67:89:ab"
      broadcast: 192.168.1.255
```

Use the same IDs as `/hosts.json`; MAC addresses have six colon-separated hex
bytes. Restart `niles remote` after editing. Invalid targets stop startup.
Caddy must route `/api/hosts/*/wake` to the relay's own Niles tailnet listener,
preserving Tailscale Serve's login header. Configure the relay's `relay_node` as
its own stable node ID so its connection passes the same WhoIs check as host
requests. Keep Caddy bound to localhost behind Tailscale Serve.

`POST /api/hosts/desktop/wake` with `Content-Type: application/json` and body `null`
sends one standard magic packet to the configured broadcast address on UDP port 9.
It returns 204 after sending, 404 for an unknown ID, and 500 on a socket failure.
It requires the same relay identity as other API routes. MAC and destination
addresses never come from the phone. An asleep host's Wake button uses this relay
route; successful sending does not guarantee the host will wake. Normal polling
recognizes it when its API becomes reachable.
