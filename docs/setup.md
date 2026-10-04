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
| → ← | Expand or collapse a running project |
| Enter | Show the project's lead, or the selected window, in the view pane |
| `r` | Register a directory as a project |
| `q` | Quiet the selected worker's check-in |
| `c` | Close the selected worker, after a y/n confirm |

The view pane on the right is a tmux client nested inside the home session, so typing there goes
to the agent. Enter retargets only that client, never your own. Leave the home view with tmux's
detach or by switching sessions.

## Key bindings

Open the home view from anywhere in tmux with a popup:

```tmux
bind-key -n M-n display-popup -E niles
```

Switch projects and windows with tmux's own keys (`switch-client -n/-p`, `next-window`,
`previous-window`), or bind them, for example:

```tmux
bind -n M-[ switch-client -p
bind -n M-] switch-client -n
bind -n M-\; previous-window
bind -n "M-'" next-window
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
