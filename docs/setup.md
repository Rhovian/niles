# Interface

## Status bar

Each project is a tmux session, and each worker is a window in it. Project sessions show a
two-line status bar at the top:

- The projects line shows every live project and its entry agent's state: `●` the entry agent or
  one of its workers is working; `⚠` the entry agent is idle with no worker working, so it is
  waiting for you, with how long it has waited.
- The windows line shows the current project's entry agent and workers, with each agent's model,
  state (`●` working, `○` idle, `⚠` waiting for you), token total, and for workers, time since
  spawn.

## Home view

`niles` opens the `niles+home` session: the explorer on the left and a view pane on the right.
The explorer lists every registered project with its entry agent's state, refreshing every two
seconds, under a header that counts projects, agents and tokens.

| Key | Action |
| --- | --- |
| ↑ ↓ | Move |
| → ← | Expand or collapse a running project or skill folder |
| Enter | Show the project's entry agent, or the selected window or panel, in the view pane |
| `[` `]` | Show the previous or next running project |
| `;` `'` | Show the previous or next window of the shown project |
| `?` | Show help in the view pane |
| Esc | Show the selected item again, leaving help |
| `r` | Register a directory as a project |
| `c` | Close the selected worker or running project, after a y/n confirm |

The explorer also has three panels:

- CONFIG shows each setting with its value and where it comes from. ← → change simple values in
  place. Enter on a tier opens the same flat form used at startup: edit primaries and ordered
  secondaries, add absent tiers, or choose `entry_agent` from the listed agents. In that form,
  `a` adds a secondary, `d` removes one, `[`/`]` reorder them, and `s` reviews changes to save;
  other settings are read-only.
- TELEMETRY shows tokens spent today, over 7 days or over 30 days, by project, by model and over
  time, with live sessions' context use.
- HELP shows the key reference.

Typing in the view pane goes to the agent shown there. With `tmux.bindings` enabled,
`M-\` (Alt/Option + backslash, without a tmux prefix) toggles focus between the explorer and
view. You can also click a pane. While the view is focused, your tmux prefix goes directly to
the shown project's session: splits and new shell windows belong to that project and swap out
when you show another project. Shells use the directory selected by your existing tmux bindings.

To detach home from the view, press `M-\` to focus the explorer, then your tmux prefix and `d`
(`Ctrl-b d` by default). The home session turns on tmux's `mouse` for itself only, so a drag
selects in tmux; hold Option (macOS) or Shift (most Linux terminals) for your terminal's own
selection. With `tmux.bindings` disabled, click the explorer before detaching.

## Settings

Change settings in the CONFIG panel or in `~/.niles/config.yaml`. An unknown key or value is an
error that names the file.

```yaml
theme: tokyo-night
tmux:
  bindings: true
```

`theme` picks the palette, `tokyo-night` by default: `dracula`, `one-dark-pro`, `nord`,
`catppuccin-mocha`, `catppuccin-latte`, `gruvbox-dark`, `gruvbox-light`, `tokyo-night`,
`solarized-dark`, `solarized-light`, `monokai-pro`, `rose-pine`, `kanagawa`, `everforest`,
`cyberpunk` or `midnight-commander`. A project's bar picks up a new theme the next time it starts.

`tmux.bindings`, off by default, adds keys that work from any tmux session:

- `M-\` toggles focus between the explorer and view pane in home.
- `M-n` opens the home view in a popup.
- `M-[` and `M-]` show the previous or next running project.
- `M-;` and `M-'` show the previous or next window of the shown project.

In `niles+home` the four cycling keys act like their plain versions in the explorer. Outside
home, the cycling and focus keys pass through to the current pane. Running `niles` installs the
bindings, replacing any of yours on those six keys. Turning the setting off doesn't remove them; they stay until tmux
restarts.
