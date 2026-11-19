# Headless agents (ACP)

## Adapters and login

| CLI | Adapter package | Tested adapter version | Login |
| --- | --- | --- | --- |
| Codex | `@agentclientprotocol/codex-acp` | 2.6.0 | `codex-acp cli login` (bundled Codex, ChatGPT) |
| Claude Code | `@agentclientprotocol/claude-agent-acp` | 0.107.3 | `claude-agent-acp --cli auth login` (Claude.ai) |
| pi | `pi-acp` | 0.0.34 | `/login` in pi, select `openai` (ChatGPT OAuth) |

Install the listed adapter. Pair stock `pi-acp` with pi. See [tested CLI versions](../README.md#requirements).

## Launch and billing

`niles start --keep-skills --mode headless --billing subscription` starts the entry agent. Pass the mode and billing flags to `niles spawn <id> --skill <skill> ...`. Select manifest-listed agents. Workers retain permission bypass.
Headless mode requires `--billing subscription`. Niles removes Codex API-key overrides and refuses account identity mismatches. API-key billing is unqualified.
Claude shares its native login, supports both modes in one workspace, and respects user and project permission restrictions. Pi requires its own login and an explicit `pi:openai/<model>[:thinking]` spec.

## Window commands

- Enter prompts one per line. They queue during turns.
- `:cancel` drops queued input. Wait for an idle `cancelled` response.
- `:resume` reloads the conversation. `niles resume <id>` restarts exited workers in their existing windows.
- `:approve <request-id> <option-id>` selects a printed permission option.
- Use the printed Claude form fields with `:answer <request-id> {"action":"accept","content":{"<field>":"<value>"}}` or cancel with `:answer <request-id> {"action":"cancel"}`.

From another window, use `niles peek <id>` for transcripts, `niles send <id> <text>` for prompts, `niles cancel <id>` for turns, or `niles approve <id> <request-id> <option-id>` for permissions (omit the option to cancel the request). Permissions and questions trigger an `attention:` wake. Explorer state comes from ACP.

## Known limits

Stock pi raises no tool approvals. Extension select and confirm dialogs use ACP permissions. The pi adapter cancels input and editor dialogs. Adapters receive no ACP MCP parameters. [#495](https://github.com/Rhovian/niles/issues/495) tracks Niles MCP injection. Peek shows messages and tool calls only.
