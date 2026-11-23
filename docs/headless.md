# Headless agents (ACP)

## Adapters and login

| CLI | Adapter package | Tested adapter version | Login |
| --- | --- | --- | --- |
| Codex | `@agentclientprotocol/codex-acp` | 2.9.0 | `codex-acp cli login` (bundled Codex, ChatGPT) |
| Claude Code | `@agentclientprotocol/claude-agent-acp` | 0.109.1 | `claude-agent-acp --cli auth login` (Claude.ai) |
| pi | `pi-acp` | 0.0.34 | `/login` in pi, select `openai` (ChatGPT OAuth) |

Install the listed adapter. Pair stock `pi-acp` with pi. See [tested CLI versions](../README.md#requirements).

## Launch and billing

Set the workspace manifest's worker launch default.

```yaml
workers: {mode: headless, billing: subscription}
```

Run `niles spawn <id> --skill <skill> <task>` as usual. Each worker uses the manifest's mode and billing unless you override them with `--mode` or `--billing`. Use `--mode native-tui` for one native worker, or remove `workers` (or set `workers: {mode: native-tui}`) to restore the native default. An agent that can't run headless fails at spawn.

`niles start --keep-skills --mode headless --billing subscription` starts the entry agent in headless mode. The entry agent defaults to native mode.

Headless mode requires an explicit `subscription` billing declaration in the manifest or on the command line. Niles removes Codex API-key overrides and refuses account identity mismatches. Workers retain permission bypass.
Claude shares its native login, supports both modes in one workspace, and respects user and project permission restrictions. Pi requires its own login and an explicit `pi:openai/<model>[:thinking]` spec.

## Window commands

- Enter prompts one per line. They queue during turns.
- `:cancel` drops queued input. Wait for an idle `cancelled` response.
- `:resume` reloads the conversation. `niles resume <id>` restarts exited workers in their existing windows.
- `:approve <request-id> <option-id>` selects a printed permission option.
- Use the printed Claude form fields with `:answer <request-id> {"action":"accept","content":{"<field>":"<value>"}}` or cancel with `:answer <request-id> {"action":"cancel"}`.

From another window, use `niles peek <id>` for transcripts, `niles send <id> <text>` for prompts, `niles cancel <id>` for turns, or `niles approve <id> <request-id> <option-id>` for permissions (omit the option to cancel the request). Permissions and questions trigger an `attention:` wake. Explorer state comes from ACP.

## Known limits

Stock pi raises no tool approvals. Extension select and confirm dialogs use ACP permissions. The pi adapter cancels input and editor dialogs. Peek shows messages and tool calls only.
