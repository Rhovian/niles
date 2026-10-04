# Suggested setup

## Projects and the status bar

Each project is a tmux session, and each worker is a window in it. Project sessions show a
two-line status bar at the top:

- **Projects:** every live project and its lead's state. `●` the lead or one of its workers is
  working; `⚠` the lead is idle with no worker working, so it is waiting for you, with how long
  it has waited.
- **Windows:** the current project's lead and workers, with each agent's model, state (`●`
  working, `○` idle, `⚠` lead waiting for you), token total, and for workers, time since spawn.

## The home view

Bare `niles` opens the `niles+home` session, creating it on first use: the explorer on the left
and a shell along the bottom, started where you ran `niles`. The explorer lists every registered
project with its lead's state and refreshes every two seconds.

| Key | Action |
| --- | --- |
| ↑ ↓ | Move |
| → ← | Expand or collapse a running project or role folder |
| Enter | Show the project's lead, or the selected window, in the view pane |
| `[` `]` | Show the previous or next running project |
| `;` `'` | Show the previous or next window of the shown project |
| `?` | Show help in the view pane |
| Esc | Show the selected item again, leaving help |
| `r` | Register a directory as a project |
| `q` | Quiet the selected worker's check-in |
| `c` | Close the selected worker or running project, after a y/n confirm |

The view pane on the right is a tmux client nested inside the home session, so typing there goes
to the agent. Enter retargets only that client, never your own. Leave the home view with tmux's
detach or by switching sessions.

## Key bindings

Open the home view from anywhere in tmux with a popup:

```tmux
bind-key -n M-n display-popup -E niles
```

In the explorer, `[` and `]` cycle the view through running projects, and `;` and `'` through
the shown project's windows. To cycle from any pane of the home view, send those keys to the
explorer. tmux has no per-session bindings, so each binding checks the session itself; elsewhere
the keys keep the command you choose (here, switching sessions and windows):

```tmux
bind -n M-[ if -F '#{==:#{session_name},niles+home}' { send-keys -t '=niles+home:{start}.{top-left}' [ } { switch-client -p }
bind -n M-] if -F '#{==:#{session_name},niles+home}' { send-keys -t '=niles+home:{start}.{top-left}' ] } { switch-client -n }
bind -n M-\; if -F '#{==:#{session_name},niles+home}' { send-keys -t '=niles+home:{start}.{top-left}' \; } { previous-window }
bind -n "M-'" if -F '#{==:#{session_name},niles+home}' { send-keys -t '=niles+home:{start}.{top-left}' "'" } { next-window }
```

## Pi with OpenRouter

Install `@mariozechner/pi-coding-agent`, launch `pi`, and use `/login` to store your OpenRouter
key in `~/.pi/agent/auth.json`. Pi never reads `.env` files. An exported `OPENROUTER_API_KEY`
only reaches niles workers if the tmux server started after the export; `/login` avoids that
requirement.

Pi's bundled model catalog lags these models. Add them to `~/.pi/agent/models.json`, which
merges into the built-in OpenRouter provider:

```json
{
  "providers": {
    "openrouter": {
      "models": [
        { "id": "deepseek/deepseek-v4.1-flash", "reasoning": true },
        { "id": "tencent/hy3", "reasoning": true },
        { "id": "z-ai/glm-5.3-flash", "reasoning": true }
      ]
    }
  }
}
```

These entries omit prices, so pi reports their cost as zero. Niles reports token usage and
leaves estimated cost unavailable.
