# Suggested setup

## Projects and the status bar

Each project is a tmux session, and each worker is a window in it. Project sessions show a
two-line status bar at the top:

- The projects line shows every live project and its entry agent's state. `●` the entry agent or
  one of its workers is working; `⚠` the entry agent is idle with no worker working, so it is
  waiting for you, with how long it has waited.
- The windows line shows the current project's entry agent and workers, with each agent's model,
  state (`●` working, `○` idle, `⚠` entry agent waiting for you), token total, and for workers,
  time since spawn.

## The home view

Bare `niles` opens the `niles+home` session, creating it on first use: the explorer on the left
and the view pane on the right, started where you ran `niles`. The explorer lists every registered
project with its entry agent's state and refreshes every two seconds. A header across the top counts
projects, live and lost agents, and their tokens, with the poll interval and time.

| Key | Action |
| --- | --- |
| ↑ ↓ | Move |
| → ← | Expand or collapse a running project or skill folder |
| Enter | Show the project's entry agent, or the selected window, in the view pane |
| `[` `]` | Show the previous or next running project |
| `;` `'` | Show the previous or next window of the shown project |
| `?` | Show help in the view pane |
| Esc | Show the selected item again, leaving help |
| `r` | Register a directory as a project |
| `c` | Close the selected worker or running project, after a y/n confirm |

The view pane on the right is a tmux client nested inside the home session, so typing there goes
to the agent. Enter retargets only that client, never your own. Leave the home view with tmux's
detach or by switching sessions.

Click a pane to focus it, or return to the explorer with tmux's `prefix + ←` (default
`Ctrl-b ←`). The home session turns tmux's `mouse` on for itself alone, so a plain drag selects
in tmux; hold Option (macOS) or Shift (most Linux terminals) for your terminal's own selection.

## Theme

Choose a named palette in `~/.niles/config.yaml`. A missing key or file uses `tokyo-night`.
Unknown keys, unknown theme names, and unreadable files are errors naming the config file.

```yaml
theme: tokyo-night
```

Available themes: `dracula`, `one-dark-pro`, `nord`,
`catppuccin-mocha`, `catppuccin-latte`, `gruvbox-dark`, `gruvbox-light`, `tokyo-night`,
`solarized-dark`, `solarized-light`, `monokai-pro`, `rose-pine`, `kanagawa`, `everforest`,
`cyberpunk`, `midnight-commander`.

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

Bindings are installed in tmux's root table the next time you run `niles`, replacing any existing
bindings for those five keys. Switching the option off installs nothing and removes nothing:
installed bindings stay until tmux restarts.
