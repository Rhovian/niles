# Suggested setup

## Projects and the status bar

Each project is a tmux session, and each worker is a window in it. Project sessions show a
two-line status bar at the top:

- **Projects:** every live project and its lead's state. `●` the lead or one of its workers is
  working; `⚠` the lead is idle with no worker working, so it is waiting for you, with how long
  it has waited.
- **Windows:** the current project's lead and workers, with each agent's model, state (`●`
  working, `○` idle, `⚠` lead waiting for you), token total, and for workers, time since spawn.

## Key bindings

Open the project list from anywhere in tmux with a popup:

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
