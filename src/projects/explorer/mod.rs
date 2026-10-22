mod collection;
mod scroll;
mod tree;

use std::{
    env, fs,
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::Command,
    sync::mpsc::TryRecvError,
    time::{Duration, Instant, SystemTime},
};

use anyhow::{Context, Result, bail};
use camino::Utf8PathBuf;
use chrono::Utc;
use ratatui::{
    DefaultTerminal, Frame,
    crossterm::{
        event::{self, DisableMouseCapture, EnableMouseCapture, KeyCode},
        execute,
    },
    layout::{Constraint, Layout},
    text::{Line, Text},
    widgets::{Block, Borders, List, ListState, Paragraph, Wrap},
};

use self::{
    collection::collector,
    tree::{Item, Member, Tree},
};
use super::{
    panels::{self, Panel},
    register,
    registry::ProjectName,
    rows::State,
    windows::{LEAD_WINDOW, Role},
};
use crate::{
    theme::{StyleKey, Theme},
    tmux::{self, TmuxTarget, WindowTarget},
    util::current_dir_utf8,
};

const REFRESH: Duration = Duration::from_secs(2);
const KEYS: [(&str, &str); 3] = [("↵", "open"), ("r", "register"), ("c", "close")];

fn key_hints(theme: &Theme) -> Line<'_> {
    let hints = KEYS.map(|(key, label)| format!("{key} {label}"));
    Line::styled(hints.join(" · "), theme.style(StyleKey::Muted))
}

/// Bare `niles`: takes the operator to the home session, creating it on first use.
pub fn home() -> Result<()> {
    let config = crate::config::user::UserConfig::load()?;
    let home = tmux::open_home(&current_dir_utf8()?)?;
    tmux::configure_home(&home, &Theme::load()?, REFRESH)?;
    if config.tmux.bindings {
        tmux::install_home_bindings()?;
    }
    tmux::switch_or_attach(&TmuxTarget::session(&home))
}

/// `niles explorer`: the tree in the home session's left pane.
pub fn run() -> Result<()> {
    let pane = env::var("TMUX_PANE").context("niles explorer runs in a tmux pane; run `niles`")?;
    // Taken now: once the binary is replaced, Linux reports this process's path as "(deleted)".
    let binary = env::current_exe().context("failed to find niles executable")?;
    let mut explorer = Explorer {
        pane: TmuxTarget::pane(&pane)?,
        cwd: current_dir_utf8()?,
        tree: Tree::default(),
        list_state: ListState::default(),
        list_area: ratatui::layout::Rect::default(),
        collected: Instant::now(),
        theme: Theme::load()?,
        mode: Mode::Browse,
        footer: None,
        collection_error: None,
        installed: modified(&binary)?,
        binary,
    };
    explorer.refresh();
    let mut terminal = ratatui::init();
    let result = execute!(std::io::stdout(), EnableMouseCapture)
        .map_err(anyhow::Error::from)
        .and_then(|()| explorer.run(&mut terminal));
    let mouse_restored = execute!(std::io::stdout(), DisableMouseCapture);
    ratatui::restore();
    result?;
    mouse_restored?;
    // Replacing the process keeps its pane, so the operator's layout survives an upgrade.
    Err(Command::new(&explorer.binary).arg("explorer").exec())
        .context("failed to restart the explorer")
}

fn modified(binary: &Path) -> Result<SystemTime> {
    fs::metadata(binary)
        .and_then(|metadata| metadata.modified())
        .with_context(|| format!("failed to read {}", binary.display()))
}

enum Mode {
    Browse,
    Directory(String),
    Name {
        path: Utf8PathBuf,
        default: String,
        input: String,
    },
    Close(Closing),
}

enum Closing {
    Worker {
        id: String,
        project: Utf8PathBuf,
    },
    /// A running project: its workers, then its view and session.
    Project {
        name: ProjectName,
        path: Utf8PathBuf,
        workers: Vec<String>,
    },
}

struct Explorer {
    pane: TmuxTarget,
    cwd: Utf8PathBuf,
    tree: Tree,
    list_state: ListState,
    list_area: ratatui::layout::Rect,
    /// When the latest refresh began; older results must not undo its data or error.
    collected: Instant,
    theme: Theme,
    mode: Mode,
    /// The outcome of the last action; the key help shows while there is none.
    footer: Option<String>,
    collection_error: Option<String>,
    binary: PathBuf,
    /// When `binary` was written; a later write is an upgrade this process doesn't run yet.
    installed: SystemTime,
}

impl Explorer {
    /// Returns once the binary on disk has been replaced and no prompt is open.
    fn run(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        let collections = collector();
        loop {
            terminal.draw(|frame| self.draw(frame))?;
            if event::poll(tree::SPINNER_FRAME)? {
                self.event(event::read()?)?;
            }
            match collections.try_recv() {
                Ok((started, projects)) => {
                    if matches!(self.mode, Mode::Browse)
                        && modified(&self.binary)? != self.installed
                    {
                        return Ok(());
                    }
                    self.accept_collection(started, projects);
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => bail!("the explorer's collector stopped"),
            }
        }
    }

    /// Every key clears the footer; a key that does something reports its outcome there.
    fn key(&mut self, code: KeyCode) -> Result<()> {
        self.footer = None;
        match (&mut self.mode, code) {
            (Mode::Browse, KeyCode::Up) => self.tree.up(),
            (Mode::Browse, KeyCode::Down) => self.tree.down(),
            (Mode::Browse, KeyCode::Right) => self.tree.expand(),
            (Mode::Browse, KeyCode::Left) => self.tree.collapse(),
            // Help leaves the selection alone, so Esc reopens what the view showed before it.
            (Mode::Browse, KeyCode::Enter | KeyCode::Esc) => self.footer = shown(self.open()),
            (Mode::Browse, KeyCode::Char('?')) => {
                self.footer = shown(
                    panels::open(Panel::Help)
                        .and_then(|window| tmux::show_in_view(&self.pane, &window))
                        .map(|()| None),
                );
            }
            (Mode::Browse, KeyCode::Char(key @ ('[' | ']' | ';' | '\''))) => {
                let steps = if matches!(key, ']' | '\'') { 1 } else { -1 };
                let landed = if matches!(key, '[' | ']') {
                    self.tree.cycle_projects(steps)
                } else {
                    self.tree.cycle_windows(steps)
                };
                if landed {
                    self.footer = shown(self.open());
                }
            }
            (Mode::Browse, KeyCode::Char('r')) => self.mode = Mode::Directory(String::new()),
            (Mode::Browse, KeyCode::Char('c')) => match self.selected_closing() {
                Some(closing) => self.mode = Mode::Close(closing),
                None => self.footer = Some("select a worker or running project".to_owned()),
            },
            (Mode::Close(closing), KeyCode::Char('y')) => {
                // Onto the next running project first, so the view moves there once this one is
                // gone. When it was the only one, its row stays selected and the view stays
                // on help.
                let project = matches!(closing, Closing::Project { .. });
                if project {
                    self.tree.cycle_projects(1);
                }
                self.footer = shown(close(closing, &self.pane));
                self.mode = Mode::Browse;
                self.refresh();
                if project
                    && matches!(self.tree.selected(), Some(Item::Project(project)) if project.agents.is_some())
                    && let Some(error) = shown(self.open())
                {
                    self.footer = Some(error);
                }
            }
            (Mode::Directory(input) | Mode::Name { input, .. }, KeyCode::Char(c)) => input.push(c),
            (Mode::Directory(input) | Mode::Name { input, .. }, KeyCode::Backspace) => {
                input.pop();
            }
            (Mode::Directory(input), KeyCode::Enter) => match register::directory(input, &self.cwd)
            {
                Ok((path, default)) => {
                    self.mode = Mode::Name {
                        path,
                        default,
                        input: String::new(),
                    }
                }
                Err(error) => self.footer = Some(format!("{error:#}")),
            },
            (
                Mode::Name {
                    path,
                    default,
                    input,
                },
                KeyCode::Enter,
            ) => {
                let name = if input.is_empty() { default } else { input };
                match register::register(name, path) {
                    Ok(()) => {
                        self.footer = Some(format!("registered {name}"));
                        self.mode = Mode::Browse;
                        self.refresh();
                        if let Some(error) = shown(panels::open(Panel::Help).map(|_| None)) {
                            self.footer = Some(error);
                        }
                    }
                    Err(error) => self.footer = Some(format!("{error:#}")),
                }
            }
            (Mode::Close { .. }, _) | (Mode::Directory(_) | Mode::Name { .. }, KeyCode::Esc) => {
                self.mode = Mode::Browse;
            }
            (Mode::Browse | Mode::Directory(_) | Mode::Name { .. }, _) => {}
        }
        Ok(())
    }

    fn open(&self) -> Result<Option<String>> {
        let Some(item) = self.tree.selected() else {
            return Ok(None);
        };
        let target = match item {
            Item::Header | Item::Folder(..) | Item::ProjectScopes => return Ok(None),
            Item::Panel(heading) => panels::open(heading.panel())?,
            Item::Scope(project) => panels::open(Panel::Config {
                project: project.map(|project| project.row.entry.name.clone()),
            })?,
            Item::Range(range) => panels::open(Panel::Telemetry { range })?,
            Item::Project(project) => {
                let entry = &project.row.entry;
                let session = entry.name.session()?;
                match project.row.state {
                    State::Missing => {
                        return Ok(Some(format!(
                            "rm ~/.niles/projects/{}",
                            entry.name.as_str()
                        )));
                    }
                    State::NotRunning => {
                        tmux::open_session(&session, &entry.path, tmux::LeadStart::Attended)?;
                        tmux::configure_status(&session, &self.theme)?;
                    }
                    State::Running | State::Starting | State::Waiting(_) => {}
                }
                WindowTarget::new(session, LEAD_WINDOW)?
            }
            Item::Member(project, _, Member::Window(window)) => {
                WindowTarget::new(project.row.entry.name.session()?, &window.name)?
            }
            Item::Member(_, _, Member::Lost(id)) => return Ok(Some(format!("{id}: window lost"))),
        };
        tmux::show_in_view(&self.pane, &target)?;
        Ok(None)
    }

    fn selected_worker(&self) -> Option<(String, Utf8PathBuf)> {
        let (project, id) = match self.tree.selected()? {
            Item::Member(project, _, Member::Window(window)) => match &window.role {
                Role::Worker(id, _) => (project, id.as_str()),
                Role::Lead | Role::Plain => return None,
            },
            Item::Member(project, _, Member::Lost(id)) => (project, id),
            Item::Project(_)
            | Item::Folder(..)
            | Item::Header
            | Item::Panel(_)
            | Item::Scope(_)
            | Item::ProjectScopes
            | Item::Range(_) => return None,
        };
        Some((id.to_owned(), project.row.entry.path.clone()))
    }

    fn selected_closing(&self) -> Option<Closing> {
        let Some(Item::Project(project)) = self.tree.selected() else {
            let (id, project) = self.selected_worker()?;
            return Some(Closing::Worker { id, project });
        };
        let agents = project.agents.as_ref()?;
        let workers = agents
            .windows
            .iter()
            .filter_map(|window| match &window.role {
                Role::Worker(id, _) => Some(id.clone()),
                Role::Lead | Role::Plain => None,
            })
            .chain(agents.lost.iter().map(|(id, _)| id.clone()))
            .collect();
        Some(Closing::Project {
            name: project.row.entry.name.clone(),
            path: project.row.entry.path.clone(),
            workers,
        })
    }

    fn draw(&mut self, frame: &mut Frame) {
        let prompt = match &self.mode {
            Mode::Browse => None,
            Mode::Directory(input) => Some(format!("directory [{}]: {input}", self.cwd)),
            Mode::Name { default, input, .. } => Some(format!("name [{default}]: {input}")),
            Mode::Close(Closing::Worker { id, .. }) => Some(format!("close {id}? y/n")),
            Mode::Close(Closing::Project { name, .. }) => {
                Some(format!("close {} and its workers? y/n", name.as_str()))
            }
        };
        let mut text = match (&self.footer, prompt) {
            (Some(message), Some(prompt)) => Text::raw(format!("{message}\n{prompt}")),
            (Some(message), None) => Text::raw(message.as_str()),
            (None, Some(prompt)) => Text::raw(prompt),
            (None, None) => Text::from(key_hints(&self.theme)),
        };
        if let Some(error) = &self.collection_error {
            text.lines.insert(0, Line::raw(error.as_str()));
        }
        let rule = Block::new()
            .borders(Borders::TOP)
            .border_style(self.theme.style(StyleKey::Guide));
        let footer_widget = Paragraph::new(text).wrap(Wrap { trim: false }).block(rule);
        #[expect(
            clippy::disallowed_methods,
            reason = "a footer past u16::MAX lines is clipped to the frame either way"
        )]
        let footer_lines =
            u16::try_from(footer_widget.line_count(frame.area().width)).unwrap_or(u16::MAX);
        let [list, footer] =
            Layout::vertical([Constraint::Fill(1), Constraint::Length(footer_lines)])
                .areas(frame.area());
        frame.render_widget(footer_widget, footer);
        let now = Utc::now();
        let labels = self
            .tree
            .items()
            .into_iter()
            .map(|item| self.tree.label(item, now, &self.theme, list.width));
        self.list_area = list;
        // ratatui-widgets 0.3.2 list/state.rs: retain state so rendering preserves its offset.
        self.list_state.select(Some(self.tree.cursor()));
        let list_widget = List::new(labels).highlight_style(self.theme.style(StyleKey::Selection));
        frame.render_stateful_widget(list_widget, list, &mut self.list_state);
    }
}

/// An action's outcome as the footer shows it.
fn shown(outcome: Result<Option<String>>) -> Option<String> {
    match outcome {
        Ok(message) => message,
        Err(error) => Some(format!("{error:#}")),
    }
}

fn close(closing: &Closing, explorer: &TmuxTarget) -> Result<Option<String>> {
    match closing {
        Closing::Worker { id, project } => {
            super::lifecycle::niles_in(project, &["close", id]).map(Some)
        }
        Closing::Project {
            name,
            path,
            workers,
        } => {
            for id in workers {
                super::lifecycle::niles_in(path, &["close", id])?;
            }
            // Before the session goes, so its client never moves to another session.
            tmux::close_view(explorer)?;
            tmux::kill_session(&name.session()?)?;
            Ok(Some(format!("closed {}", name.as_str())))
        }
    }
}

#[cfg(test)]
mod tests;
