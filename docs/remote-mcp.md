# Remote MCP

Build Niles with `--features remote` and run `niles remote` as the Niles host user.
Use the same user service and `niles doctor` checks as for the PWA. The process enables
MCP when `remote.mcp` is configured and enables the web API when `remote.web`
is configured. MCP-only hosts can omit the relay configuration and Tailscale.

MCP listens on `127.0.0.1:8766` by default. Set `remote.mcp.port` to change it.
The `--port` flag sets the web API port.
Put an HTTPS reverse proxy on the same host and give clients the canonical URL,
for example `https://niles.example.com/mcp`.
Preserve the authority in `Host`, the request paths and the `Authorization` header.
Expose both paths, as in this Caddy configuration.

```caddy
niles.example.com {
    @niles_mcp path /mcp /.well-known/oauth-protected-resource/mcp
    reverse_proxy @niles_mcp 127.0.0.1:8766
}
```

Set `remote.mcp` and the shared `remote.issuers` in `~/.niles/config.yaml`. Move any
existing `remote.mcp.issuers` to `remote.issuers` on upgrade. The resource URL is also the exact token
audience. Use the exact issuer (`iss`) and subject (`sub`) values published by each
provider. Grants match registered project names.

```yaml
remote:
  mcp:
    resource: https://niles.example.com/mcp
    interactive_issuer: https://login.example.com
  issuers:
    https://login.example.com:
      - {subject: 'alice-provider-subject', project: demo, access: read}
    https://token.actions.githubusercontent.com:
      - {subject: 'repo:example/demo:ref:refs/heads/main', project: demo, access: operate}
```

A `read` grant allows project and session listings and transcript reads. An `operate`
grant also allows starting and stopping the saved lead session, messaging the lead or
a worker, and closing a worker. Lead tools take the registered `project` name. Worker
tools also take the `worker_id` returned by `list_projects`. Message tools take `text`.
Call `read_lead_events` or `read_worker_events` again to refresh the bounded recent
transcript snapshot. The MCP endpoint uses stateless Streamable HTTP with JSON responses.

A verified identity with no grants receives HTTP 403. If a grant does not cover the
requested project or operation, the tool returns an error message in its MCP result.
Restart `niles remote` after editing MCP settings, grants or issuers.

Every issuer's discovery document and each JWT issuer’s signing keys must be reachable over
HTTPS with publicly trusted TLS at startup. The process exits if either fetch fails.
Niles refreshes keys on demand after five minutes. On a failed refresh it keeps the last
good keys for up to 15 minutes from the last successful fetch. Keep the host clock
synchronized. Local JWT validation allows 60 seconds of clock skew.

## Interactive clients

Set `interactive_issuer` to a listed issuer to advertise it for sign-in. Leave it unset
for workload-only access. Configure the issuer for the MCP resource and the client's
registration method, typically DCR or CIMD with authorization code and PKCE. For cloud chat clients, both the issuer and endpoint must be reachable from the
vendor's egress. Add the resource URL to the client as a remote MCP server. Its discovery
starts from the 401 Bearer challenge, which links to the public protected resource metadata.
Consult your client's requirements. [Claude connector authentication](https://claude.com/docs/connectors/building/authentication),
[ChatGPT custom MCP servers](https://developers.openai.com/api/docs/guides/custom-mcp-server).

Configure the provider to place the exact resource URL as the sole value in the JWT access token’s
`aud` claim. Common provider defaults can produce a different audience or opaque tokens.
Opaque tokens from a configured introspection issuer such as tsidp use the same
project grants. See the [remote PWA guide](remote.md#configure-tsidp) for issuer setup.
An interactive MCP client must support the issuer’s registration and redemption
requirements. The PWA’s host-side code exchange serves the PWA.

Clients that send an `Origin` header, including browser-based MCP tools, receive 403
with valid credentials. Use an MCP client that connects from its desktop process or backend.

## GitHub Actions

Give the job `id-token: write` and grant the exact job subject configured by GitHub.
For example, this step starts the saved lead session for `demo`. GitHub provides the
token request URL and request credential to the runner. [GitHub Actions OIDC](https://docs.github.com/en/actions/reference/security/oidc).

```yaml
permissions:
  id-token: write
steps:
  - name: Start the Niles lead
    env:
      NILES_MCP_URL: https://niles.example.com/mcp
    shell: bash
    run: |
      token=$(printf 'Authorization: Bearer %s\n' "$ACTIONS_ID_TOKEN_REQUEST_TOKEN" |
        curl -fsS -H @- --get --data-urlencode "audience=$NILES_MCP_URL" "$ACTIONS_ID_TOKEN_REQUEST_URL" |
        jq -er '.value')
      mcp() {
        printf 'Authorization: Bearer %s\n' "$token" |
          curl -fsS -H @- -H 'Accept: application/json, text/event-stream' \
            -H 'Content-Type: application/json' -H 'MCP-Protocol-Version: 2025-11-25' \
            --data "$1" "$NILES_MCP_URL"
      }
      mcp '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"github-actions","version":"1"}}}' |
        jq -e '.result.protocolVersion == "2025-11-25"'
      mcp '{"jsonrpc":"2.0","method":"notifications/initialized"}'
      mcp '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"start_lead","arguments":{"project":"demo"}}}' |
        jq -e '.result.isError == false'
```

Review the [threat model](security.md#remote-mcp) before making this endpoint public.
