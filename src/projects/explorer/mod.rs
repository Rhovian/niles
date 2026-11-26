use self::collection::collector;
use self::menu::{Menu, MenuInput};
use self::tree::{Item, Member, Project, Tree};
use super::panels::{self, Panel};
use super::register;
use super::registry::{self, ProjectName};
use super::rows::State;
use super::windows::LEAD_WINDOW;
use crate::config::user::UserConfig;
use crate::theme::{StyleKey, Theme};
use crate::tmux::{self, TmuxTarget, WindowTarget};
use crate::util::current_dir_utf8;
use anyhow::{Context, Result, bail};
use camino::Utf8PathBuf;
use chrono::Utc;
use ratatui::crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, KeyCode};
use ratatui::crossterm::execute;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Text};
use ratatui::widgets::{Block, Borders, List, ListState, Paragraph, Wrap};
use ratatui::{DefaultTerminal, Frame};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::TryRecvError;
use std::time::{Duration, Instant, SystemTime};
use std::{env, fs, io};

mod collection;
mod menu;
mod scroll;
mod tree;

const REFRESH: Duration = Duration::from_secs(2);

/// Bare `niles`: takes the operator to the home session, creating it on first use.
pub fn home() -> Result<()> {
    let config = UserConfig::load()?;
    let cwd = current_dir_utf8()?;
    let theme = Theme::load()?;
    let target = super::onboard::first_run(&cwd)?
        .map(|entry| start_project(&entry, &theme))
        .transpose()?;
    let home = tmux::open_home(&cwd)?;
    tmux::configure_home(&home, &theme, REFRESH)?;
    if config.tmux.bindings {
        tmux::install_home_bindings()?;
    }
    if let Some(target) = target {
        tmux::show_in_view(&TmuxTarget::pane(&tmux::explorer_pane())?, &target)?;
    }
    tmux::switch_or_attach(&TmuxTarget::session(&home))
}

fn start_project(entry: &registry::Entry, theme: &Theme) -> Result<WindowTarget> {
    super::lifecycle::start_entry(entry, tmux::LeadStart::Attended, theme)?;
    entry.window(LEAD_WINDOW)
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
        list_area: Rect::default(),
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
    let result = execute!(io::stdout(), EnableMouseCapture)
        .map_err(anyhow::Error::from)
        .and_then(|()| explorer.run(&mut terminal));
    let mouse_restored = execute!(io::stdout(), DisableMouseCapture);
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
    Register(register::prompt::Prompt),
    Menu(Menu),
    Close(Closing),
}

#[derive(Clone, PartialEq, Eq)]
struct Closing {
    name: ProjectName,
    path: Utf8PathBuf,
    workers: Vec<String>,
    next_worker: usize,
}

impl Closing {
    fn project(project: &Project) -> Option<Self> {
        let agents = project.agents.as_ref()?;
        let workers = agents
            .windows
            .iter()
            .filter_map(|window| window.kind.worker_id())
            .chain(agents.lost.iter().map(|(id, _)| id.as_str()))
            .map(str::to_owned)
            .collect();
        Some(Self {
            name: project.row.entry.name.clone(),
            path: project.row.entry.path.clone(),
            workers,
            next_worker: 0,
        })
    }
}

struct Explorer {
    pane: TmuxTarget,
    cwd: Utf8PathBuf,
    tree: Tree,
    list_state: ListState,
    list_area: Rect,
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
            if !self.advance_close() && event::poll(tree::SPINNER_FRAME)? {
                self.event(event::read()?);
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
    fn key(&mut self, code: KeyCode) {
        self.footer = None;
        if code == KeyCode::Enter && self.pick_menu(MenuInput::Enter) {
            return;
        }
        let view = |target| tmux::show_in_view(&self.pane, &target);
        match (&mut self.mode, code) {
            (Mode::Browse, KeyCode::Up) => self.tree.up(),
            (Mode::Browse, KeyCode::Down) => self.tree.down(),
            (Mode::Browse, KeyCode::Right) => self.tree.expand(),
            (Mode::Browse, KeyCode::Left) => self.tree.collapse(),
            (Mode::Browse, KeyCode::Enter) => self.activate(None),
            // Help leaves the selection alone, so Esc reopens what the view showed before it.
            (Mode::Browse, KeyCode::Esc) => self.footer = shown(self.open()),
            (Mode::Browse, KeyCode::Char('?')) => {
                self.footer = shown(panels::open(Panel::Help).and_then(view).map(|()| None));
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
            (Mode::Browse, KeyCode::Char('r')) => {
                self.mode = Mode::Register(register::prompt::Prompt::new())
            }
            (Mode::Menu(menu), KeyCode::Up | KeyCode::Down) => {
                menu.move_highlight(code == KeyCode::Down);
            }
            (Mode::Menu(_), KeyCode::Esc) => self.mode = Mode::Browse,
            (Mode::Register(prompt), code) => {
                use register::prompt::Action;
                match prompt.key(code, &self.cwd) {
                    Ok(Action::Pending) => {}
                    Ok(Action::Cancel) => self.mode = Mode::Browse,
                    Ok(Action::Register { path, name }) => match register::register(&name, &path) {
                        Ok(entry) => {
                            self.mode = Mode::Browse;
                            let started = start_project(&entry, &self.theme).and_then(view);
                            self.footer =
                                shown(started.map(|()| Some(format!("registered {name}"))));
                            self.refresh();
                        }
                        Err(error) => self.footer = Some(format!("{error:#}")),
                    },
                    Err(error) => self.footer = Some(format!("{error:#}")),
                }
            }
            (Mode::Browse | Mode::Menu(_) | Mode::Close(_), _) => {}
        }
    }

    fn advance_close(&mut self) -> bool {
        let Mode::Close(closing) = &mut self.mode else {
            return false;
        };
        let outcome = match closing.workers.get(closing.next_worker) {
            Some(id) => super::lifecycle::niles_in(&closing.path, &["close", id]).map(|_| None),
            None => finish_project_close(&closing.name, &self.pane).map(Some),
        };
        if matches!(outcome, Ok(None)) {
            closing.next_worker = closing.next_worker.saturating_add(1);
            return true;
        }
        self.footer = shown(outcome);
        self.mode = Mode::Browse;
        self.refresh();
        if matches!(self.tree.selected(), Some(Item::Project(project)) if project.agents.is_some())
            && let Some(error) = shown(self.open())
        {
            self.footer = Some(error);
        }
        true
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
            Item::Project(project) => match project.row.state {
                State::Missing => return Ok(Some("project directory missing".to_owned())),
                State::NotRunning => start_project(&project.row.entry, &self.theme)?,
                State::Running | State::Starting | State::Waiting(_) => {
                    project.row.entry.window(LEAD_WINDOW)?
                }
            },
            Item::Member(project, _, Member::Window(window)) => {
                project.row.entry.window(&window.name)?
            }
            Item::Member(_, _, Member::Lost(id)) => return Ok(Some(format!("{id}: window lost"))),
        };
        tmux::show_in_view(&self.pane, &target).map(|()| None)
    }

    fn draw(&mut self, frame: &mut Frame) {
        let hints = |keys| Text::styled(keys, self.theme.style(StyleKey::Muted));
        let mut text = match (&self.mode, &self.footer) {
            (Mode::Browse, None) => hints("↵ actions/open · r register"),
            (Mode::Browse, Some(message)) => Text::raw(message.as_str()),
            (Mode::Menu(_), _) => hints("↑↓ choose · ↵ run · esc dismiss"),
            (Mode::Close(closing), _) => {
                Text::raw(match closing.workers.get(closing.next_worker) {
                    Some(id) => format!(
                        "closing {} {}/{} {id}",
                        closing.name.as_str(),
                        closing.next_worker.saturating_add(1),
                        closing.workers.len()
                    ),
                    None => format!("closing {}", closing.name.as_str()),
                })
            }
            (Mode::Register(prompt), message) => {
                let mut text = Text::raw(prompt.text(&self.cwd));
                if let Some(message) = message {
                    text.lines.insert(0, Line::raw(message.as_str()));
                }
                text
            }
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
        if let Mode::Menu(menu) = &mut self.mode {
            let row = self.tree.cursor().saturating_sub(self.list_state.offset());
            menu.draw(frame, list, row, &self.theme);
        }
    }
}

/// An action's outcome as the footer shows it.
fn shown(outcome: Result<Option<String>>) -> Option<String> {
    match outcome {
        Ok(message) => message,
        Err(error) => Some(format!("{error:#}")),
    }
}

fn finish_project_close(name: &ProjectName, explorer: &TmuxTarget) -> Result<String> {
    // Before the session goes, so its client never moves to another session.
    tmux::close_view(explorer)?;
    tmux::kill_session(&name.session()?)?;
    Ok(format!("closed {}", name.as_str()))
}

#[cfg(test)]
mod tests;
