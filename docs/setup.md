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

## Theme

Choose a named palette in `~/.niles/config.yaml`. A missing key or file uses `tokyo-night`.
Unknown keys, unknown theme names, and unreadable files are errors naming the config file.

```yaml
theme: tokyo-night
```

Available names from `ThemeName::all()`: `dracula`, `one-dark-pro`, `nord`,
`catppuccin-mocha`, `catppuccin-latte`, `gruvbox-dark`, `gruvbox-light`, `tokyo-night`,
`solarized-dark`, `solarized-light`, `monokai-pro`, `rose-pine`, `kanagawa`, `everforest`,
`cyberpunk`, `midnight-commander`.

Palette slots: running uses `success`, waiting `warning`, lost `error`, idle/muted/guides
`muted` (guides dim), accents `accent` (bold), selection `selection` (bold background),
active pills `bg` text on `accent` (bold), and the bar `fg` text on `selection`.
Headings stay bold without a color; glyphs are fixed and unstyled text keeps the terminal default.

Bar changes apply when a session's status is configured; there is no live reload.

## Key bindings

Enable the built-in bindings in `~/.niles/config.yaml` (default: false):

```yaml
tmux:
  bindings: true
```

- `M-n` opens the home view in a popup from any session.
- `M-[` and `M-]` show the previous or next running project.
- `M-;` and `M-'` show the previous or next window of the shown project.

The four cycling keys send their plain keys to the explorer in `niles+home`; elsewhere,
they pass the original Meta key through to the current pane.

Bindings are installed in tmux's root table on the next bare `niles`, replacing any existing
bindings for those five keys. Switching the option off installs nothing and removes nothing:
installed bindings stay until tmux restarts.

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
