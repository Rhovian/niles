---
name: remote-setup
description: Use when installing or upgrading Niles phone access through a Tailscale relay, including issuer sign-in, Caddy, tailnet grants and shared push.
---
# Remote setup

Set up an always-on relay that also runs Niles. The request path is phone → Tailscale
Serve HTTPS → localhost Caddy → each Niles host's tailnet IPv4 listener. Serve cannot
proxy directly to other tailnet IPs because its dialer bypasses Tailscale routes.
Keep Niles private to the tailnet. The browser contacts the issuer separately.

## Inventory before changing anything

1. Collect the relay and host SSH/access details, OS users running Niles, host display
   names and routing IDs, tailnet users, issuer and client registrations, per-host
   issuer subjects and project grants, and whether push is wanted. For push collect
   a mailto: or https: contact URI. Include the relay as a host.
   Ask only for information you cannot discover through authorized access.
2. Read the deployment's existing config and service state. Inspect
   `tailscale serve status --json`, Funnel status, Caddy configuration and listeners,
   `~/.niles/config.yaml`, and Niles user services on every machine.
   For an existing Niles deployment, follow the cutover below. Treat paths, listeners and
   Serve mounts owned by unrelated services as conflicts.
   Preserve unrelated config. Do not use `tailscale serve reset`.
   Existing path/Unix-socket mounts gate the whole Serve config: even step 12's HTTP
   proxy may need sudo. Preserve those mounts and report missing privileges.
   [Tailscale #22117](https://github.com/tailscale/tailscale/pull/22117) makes root
   mandatory for configs containing these mounts; operator privileges no longer suffice.
3. Verify `type -a niles tailscale`, `niles --version`, `niles remote --help`,
   `tailscale version --daemon --json`, `tailscale status --json`, and
   `tailscale ip -4` on each machine. Require a remote-enabled Niles binary, matching
   Tailscale CLI/daemon versions >=1.96.2, a running authenticated daemon, and stable
   tailnet IPv4 addresses. Record the relay's DNS name and each host's IPv4 address.
4. On the relay enable MagicDNS and HTTPS certificates in the tailnet DNS settings.
   Verify its DNS name is listed in CertDomains. The phone joins the same tailnet.
   Use open-source Homebrew tailscaled on macOS. Check the LaunchDaemon's PATH
   includes the directory containing the tailscale CLI; Homebrew service regeneration
   can undo this. Keep this PATH check until the relay includes Tailscale #22117,
   which removes the daemon's `sudo --list` check.
   Check Tailscale operator privileges for the service account; if
   needed, give the operator the exact `sudo tailscale set --operator=<user>` fix.
   Report privileged prerequisites that authorized access cannot complete.
5. Install matching remote-enabled Niles binaries on all hosts and built PWA assets
   on the relay. Use the release's `niles-web.tar.gz` with
   `cargo install niles --locked --version <version> --features remote`, or a trusted
   checkout's `scripts/install-remote.sh` (Node 24, pnpm 10.8.1, Rust 1.89+, OpenSSL).
   Confirm `index.html` is at the asset root. Serve only this dedicated built directory
   plus hosts.json; never serve a checkout, home directory or `~/.niles/`.
   Install Caddy on the relay and record `caddy version`.

## Restrict the tailnet

6. Merge tags and grants into the existing tailnet policy. Substitute actual owners
   and users; this fragment is not a replacement policy:

   ```json
   {
     "tagOwners": {
       "tag:niles-relay": ["autogroup:admin"],
       "tag:niles-host": ["autogroup:admin"]
     },
     "grants": [
       {"src": ["you@example.com"], "dst": ["tag:niles-relay"], "ip": ["tcp:443"]},
       {"src": ["tag:niles-relay"], "dst": ["tag:niles-host"], "ip": ["tcp:8765"]},
       {"src": ["tag:niles-relay"], "dst": ["tag:niles-relay"], "ip": ["tcp:8765"]}
     ]
   }
   ```

   Assign only the relay tag to the relay node; assign the host tag to other hosts.
   The third grant lets the relay reach its own backend. Adapt ports if needed.
   Inspect all existing grants/ACLs: permissions are additive, so a broad allow rule defeats these restrictions. Preserve needed
   SSH/admin access; verify policy tests permit chosen phone users on relay:443 and
   relay on every backend port, and deny unwanted users/devices. Apply tags through
   the admin console or `tailscale set --advertise-tags=...`, preserving existing tags.
   Tags and tailnet grants control network reachability. Issuer grants control Niles access.
   Tailnet policy reference: https://tailscale.com/docs/reference/syntax/policy-file

## Configure hosts and push

7. For tsidp, keep its HTTPS issuer on the tailnet. Register a distinct confidential
   client per Niles host. Give each client the exact PWA HTTPS root as its redirect URI. On the tsidp node, grant
   `tailscale.com/cap/tsidp` for the intended phone users with exact `users` and
   `resources` for each Niles host. Allow the phone and backends to reach tsidp on
   HTTPS. The phone uses a user-owned tailnet device, and the PWA uses S256 PKCE.
   Ask the operator to complete issuer changes that require their access.
   [tsidp capability setup](https://github.com/tailscale/tsidp/blob/v0.0.21/README.md#setting-an-application-capability-grant)

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

   Merge the settings into each host’s existing `~/.niles/config.yaml` and set
   its bind address, resource and client ID separately. Use the exact decimal
   Tailscale user ID as the subject. Set only that client’s secret in the named
   Niles service environment variable. Each host uses its credentials for token
   exchange and introspection. Preserve existing JWT issuer grants.
   Operate permits shell access through agents for its project. A public JWT
   OAuth client can omit `remote.introspection` and use browser token exchange.

8. For push, generate one key with
   `niles remote init --subject mailto:you@example.com` on the relay as the Niles user.
   Securely copy that same `~/.niles/remote/vapid.json` to every host user's same path.
   Keep its parent directory mode 0700 and file mode 0600, owned by the service user.
   Compare files without printing private contents. Do not generate independent keys,
   overwrite an existing key, put keys in the web root, or copy subscriptions.
   If push is declined, skip initialization and report push as unconfigured.
9. Create a user service for `niles remote --port 8765` on each host, using the
    absolute binary path and an explicit PATH containing tmux and the
    agent CLIs. Preserve the user's HOME and credential environment; shell startup
    files are not loaded. On Linux use `~/.config/systemd/user/niles-remote.service`,
    with After=network-online.target, Restart=on-failure, RestartSec=5 and
    WantedBy=default.target, then daemon-reload and
    enable --now; enable user lingering if it must survive logout. On macOS use
    `~/Library/LaunchAgents/dev.niles.remote.plist`, Label=dev.niles.remote,
    ProgramArguments as an array, RunAtLoad=true, KeepAlive=true and stdout/stderr logs,
    then `launchctl bootstrap gui/<uid> <plist>`. Escape paths for the service format;
    validate with systemd-analyze verify or plutil -lint before loading.
    Create log directories first. Check startup logs and listener address. Restart after config or key changes.

## Configure the relay origin

10. Write `hosts.json` in the built PWA root. With tsidp, set
    `host_token_exchange` to true and give each host its own `client_id`. For direct
    browser exchange, repeat the public `client_id` on each host entry. Use unique
    lowercase routing IDs matching `[A-Za-z0-9][A-Za-z0-9_-]*`.

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

    For one direct host, use a single host entry with an empty ID. Route
    `/api/*` to the host and use the PWA origin followed by `/api` as its resource.

11. Write a dedicated Caddyfile outside the web root, e.g.
    `~/.config/niles/Caddyfile`. Replace example IPs with the recorded tailnet IPv4s,
    and extend one explicit handle_path per manifest host. NILES_WEB_DIR must be an
    absolute path to the built PWA root, set in Caddy's service environment:

    ```caddyfile
    {
        admin off
        auto_https off
    }
    http://127.0.0.1:8080 {
        bind 127.0.0.1
        handle_path /h/relay/* {
            reverse_proxy 100.64.0.1:8765
        }
        handle_path /h/desktop/* {
            reverse_proxy 100.64.0.2:8765
        }
        @unknown path /api /api/* /h /h/*
        handle @unknown {
            respond 404
        }
        handle {
            root * "{$NILES_WEB_DIR}"
            header /hosts.json Cache-Control no-store
            header Content-Security-Policy "connect-src 'self' https://idp.example.ts.net"
            file_server
        }
    }
    ```

    handle_path strips /h/<id>, retaining /api/... and query strings. Caddy forwards
    Authorization to each backend.
    Replace the example login origin in the CSP with each configured discovery and
    token endpoint origin. Keep the CSP on the shell response.
    Caddy streams text/event-stream automatically. Use its normal HTTP transport,
    not a userspace Tailscale transport. Reference:
    https://caddyserver.com/docs/caddyfile/directives/reverse_proxy
12. With NILES_WEB_DIR set, run `caddy validate --config <Caddyfile> --adapter caddyfile`.
    Install a separate Caddy service as the relay user, with absolute binary/config
    paths and that environment, using the same OS service manager as step 9.
    Check that port 8080 listens only on 127.0.0.1. Start Caddy before Serve.
    Run `tailscale serve --bg --https=443 http://127.0.0.1:8080` as the configured
    operator. Check Serve status and confirm Funnel is disabled. Serve owns TLS and Caddy is reachable only locally.
    Reference: https://tailscale.com/docs/features/tailscale-serve

## Verify and hand back

13. From an allowed tailnet device, require 200 HTML at the relay HTTPS root and a
    valid, noncached hosts.json. Sign in to each host through the issuer. Verify
    session/project listings and the initial transcript stream. After the first tsidp
    access token expires, reopen the PWA and verify it refreshes without sign-in.
    Confirm an unknown
    host route returns 404. Through the relay, submit a request to each host with
    `Tailscale-User-Login` and no bearer token and require 401. Use a short-lived
    token for one host at another host and require 401. Use a read grant to open
    a transcript. Confirm lead messaging returns 404 and starting a session returns 403.
    Keep tokens out of logs and command history. Verify an unauthorized tailnet peer cannot
    reach the relay HTTPS listener. Report any verification that could not be run.
14. Open the relay URL on the phone, send a harmless message and verify it at the desk.
    Install/Add to Home Screen (iOS push requires 16.4+ and the installed app). With all
    hosts awake and push configured, Enable notifications must register one shared
    subscription through every host; fix any named failure and retry. Check each public VAPID key matches,
    trigger a notification per host, and confirm it opens the correct host/session.
    Rerun registration after adding hosts. Opening notifications needs Tailscale.
15. Record exactly which mounts, files and services this run created. Report the relay
    URL, host/route inventory, changed files and policy, service/log
    locations, restart commands, verification results and anything untested. For
    recovery remove only files/services and the Serve mount created in this run
    (`tailscale serve --bg --https=443 --set-path=/ off`), after rechecking ownership;
    preserve keys, config and
    unrelated mounts. Upgrade binary and PWA together and close/reopen app windows.

## Cut over an existing relay-identity deployment

- Inventory the existing Niles services, assets, routes, tailnet policy and shared VAPID key.
  Complete issuer setup from step 7 for every host before changing listeners.
- Stage the matching binary, PWA assets, and config for all hosts. Stop the old Niles
  services during the cutover. Remove `remote.relay_node` and any older
  `remote.allowed_users` key. Set `remote.web` and `remote.issuers` on every host. Move
  any `remote.mcp.issuers` to `remote.issuers` and merge those grants. For tsidp, set
  `remote.introspection` and install each host’s own client secret in its service
  environment. Remove retired wake configuration. Preserve VAPID keys, network
  grants, and unrelated mounts and services. While the old services
  are stopped, move each host’s `~/.niles/remote/subscriptions` directory aside. Niles
  recreates it at startup.
- Update the Caddy asset root, host routes and shell CSP from step 11. Remove
  identity-header injection and any retired wake route. Set the issuer and each
  host’s client ID in `hosts.json` and enable host token exchange for tsidp. Validate the Caddy
  config and restart Caddy and the Niles services. Keep the existing HTTPS Serve mount.
- Reopen the PWA and follow steps 13–14. After signing in to all hosts, enable notifications
  again to bind registrations to issuer identities. Preserve the shared signing key.
