# Remote access over Tailscale

Phone access is shell access on the Niles host: the phone can instruct agents that run commands
as you, and workers bypass approval prompts. Serve the app only from the Niles host, never from
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

## Configure

Run setup as your normal user, not root, after stopping any `niles remote` you started by hand:

```sh
niles remote setup --web-dir "$NILES_WEB_DIR" \
  --subject mailto:you@example.com
```

It creates push keys if there are none, installs and starts a user service, and serves the app
at `/` and the API at `/api` through Tailscale Serve on HTTPS 443. Omit `--subject`
for chat without push; add push later with `niles remote init --subject mailto:you@example.com`
and a restart.

Setup first runs the checks in `niles remote doctor`, which you can rerun at any time. Each
failure prints the fix. The common ones:

- The `niles` on your PATH isn't the remote-enabled binary. Put its directory first on PATH and
  run `hash -r`; `type -a niles` shows which one wins.
- The Tailscale CLI and daemon versions differ. Check `type -a tailscale` for a stale GUI wrapper.
- On macOS, the Homebrew `tailscaled` LaunchDaemon can't find the CLI on its PATH. Doctor prints
  the fix; `brew services restart tailscale` can undo it, so rerun doctor afterwards.
- You aren't the Tailscale operator: run `sudo tailscale set --operator="$(id -un)"`. On macOS
  you also need to be an administrator.
- The host has no HTTPS certificate domain: enable HTTPS in the DNS settings.

Setup and doctor never run `sudo`. The service file is
`~/Library/LaunchAgents/dev.niles.remote.plist` on macOS or
`~/.config/systemd/user/niles-remote.service` on Linux; on Linux, run
`loginctl enable-linger "$USER"` to keep it running while you're logged out. Setup refuses to
replace an existing service file or existing `/` and `/api` Serve handlers, and it ends by
checking the API from the host. If that check only warns, run the printed `curl -i` command from
an allowed device. `--port` changes the loopback port.

## Install on the phone

Open `https://<host>.<tailnet>.ts.net/` on the phone, select a live session and send a harmless
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
- 404 at `/api/sessions`, or HTML instead of JSON: the `/api` mapping must target
  `http://127.0.0.1:PORT/api`. Run `niles remote uninstall`, then setup again.
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

To upgrade a release install, install the new binary and web asset as above, then run
`niles remote uninstall` and rerun setup with the new web directory. To upgrade a checkout, rerun
`scripts/install-remote.sh`, restart, then close and reopen every Niles window. Upgrade the binary
and app together: a version mismatch shows an update banner and blocks chat.

`niles remote uninstall` removes the service and the `/` and `/api` Serve mappings, and keeps
your config, push keys and any other Serve mounts. Check `tailscale serve status` first, and
never use `tailscale serve reset`, which removes unrelated Serve configuration too. If uninstall
reports a service-manager error for a stopped service, delete the service file by hand and rerun.
