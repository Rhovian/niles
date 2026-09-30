# Roles

Niles gives each agent one role fragment plus a shared reporting contract. Keeping ownership
separate prevents every participant from rerunning the same work and makes the handoff auditable.

| Role | Owns | Must not do |
| --- | --- | --- |
| Lead | Outcome, plan, delegation, integration, and deciding when work is complete | Implement delegated changes |
| Worker | The change and the project gate, including reporting exact check results | Claim completion without the gate |
| Reviewer | Correctness, idiom, economy, and whether tests prove behavior efficiently | Run the gate or produce hardening findings |
| Security | An adversarial pass for a named, reachable attacker | Run the gate or redo correctness and style review |

## Lead

The foreground lead decides what gets built and how. Before delegating, it reads enough code to
settle foreseeable choices, names exact boundaries and acceptance criteria, and hands the worker a
plan rather than unresolved alternatives. It leaves coding mechanics to the worker. Small reads or
mechanical confirmations can stay inline; implementation, independent judgment, parallel work,
and work needing a fresh context are delegated.

Configured role bindings are authoritative. The lead normally omits `--agent`; changing the
configured family, model, or effort requires operator approval unless already authorized. Effort
follows risk by changing the number and scope of review passes, not by silently substituting an
agent. Re-review should target the fix and nearby regressions rather than repeat the original pass.

Workers that edit overlapping files can overwrite one another without a merge conflict, so
parallel workers need disjoint ownership. Delegation goes through Niles so workers can be watched,
steered, and woken through the same local protocol.

## Worker

The worker implements the scoped change and owns the repository's documented gate. Its report
names the checks and their output. `done:` means work is ready for inspection, not that the agent
should exit; uncertainty is reported as `blocked:` or `needs-decision:` rather than hidden behind a
confident completion line.

Gate ownership is intentionally singular. If every role is told to verify the same change, every
role runs the suite, wasting time while making it unclear whose evidence is authoritative. A
reviewer can challenge a missing or implausible result without quietly rerunning it.

## Reviewer

The reviewer examines the delta in this order: concrete correctness failures, consistency with
surrounding idiom, opportunities to remove code or reuse existing helpers, and whether tests prove
behavior rather than implementation phrasing. It distinguishes defects from preferences and does
not re-derive the whole design.

## Security

The security role is commissioned when the change itself creates or modifies a security boundary,
such as internet-facing, authentication, or untrusted-input forwarding code. It first names the
attacker and reachability. Findings state controlled input, path, outcome, and resource or blast
radius, covering injection, authorization, secrets, exhaustion, and amplification where relevant.

Security remains separate from ordinary review because combining them encourages every small
change to accumulate hardening for an attacker nobody identified. A reviewer flags a
security-relevant boundary in one line; the lead then commissions the dedicated pass.

## Bindings and planning guidance

`.niles/manifest.yaml` binds `lead`, `worker`, `reviewer`, and `security` independently. Security
may be rare, but its tier is still a workspace decision rather than a choice the lead improvises at
spawn time. `niles spawn --role worker` is the default; reviewer and security select their own
briefs. An explicit `--agent` overrides the binding.

Optional `worker_planning` keys are exact `family:model` pairs. Only implementation assignments
consult them; effort is ignored, and reviewer and security assignments are unaffected. The strings
are operator-authored instructions, not built-in judgments or benchmark claims. A missing match or
a binding without a model adds no planning policy. See the valid full example in
[configuration](configuration.md#workspace-manifest).

All spawned roles share the same status-line contract and report location. Worker-authored wake
states are `done:`, `failed:`, `blocked:`, and `needs-decision:`; `working:` is recorded but not
actionable. Niles owns `closed:`.

## Example flow

```sh
niles spawn auth-impl --task auth --agent codex:gpt-5.5:xhigh \
  "Fix the flaky auth test, then run the project's checks."
niles wait auth-impl
niles report auth-impl
niles spawn auth-rev --task auth --role reviewer --agent claude:opus:high \
  "Review auth-impl's fix. Its report says which checks it ran."
niles wait auth-rev
niles report auth-rev
niles close --task auth
```

For a single dispatch, `spawn --wait` or `send --wait` replaces the adjacent explicit `wait`.
Keeping dispatch and wait separate is useful for a labeled fleet because `wait --task auth`
returns whichever worker reports first.
