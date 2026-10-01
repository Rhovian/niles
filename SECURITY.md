# Security Policy

## Supported versions

Niles supports the latest release and current `main`. Security fixes are made on `main` and ship
as a new release. Older releases do not receive backports.

| Version | Supported |
| ------- | --------- |
| Latest release | Yes |
| Current `main` | Yes |
| Older releases | No |

## Reporting a vulnerability

Please report suspected vulnerabilities privately through a
[GitHub security advisory](https://github.com/Rhovian/niles/security/advisories/new). Do not open a
public issue for a vulnerability before the project has had a chance to investigate it.

Include, when available:

- the affected commit or version;
- steps or a minimal example that reproduce the issue;
- the security impact, including which input an attacker controls; and
- sanitized logs or diagnostics, with credentials, tokens, personal data, and other secrets
  removed.

The project will make a best-effort initial response within seven calendar days. This is an
initial-response target, not a resolution deadline or a guarantee. The project lead may update the
timeline as the investigation develops, particularly if the operator or reporter responds with
additional information.

For the security assumptions and known gaps that help determine whether behavior is a
vulnerability, see the [Niles threat model](docs/security.md).
