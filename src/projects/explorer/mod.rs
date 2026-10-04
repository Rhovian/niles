mod tree;

use std::{
    env,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use chrono::Utc;
use ratatui::{
    DefaultTerminal, Frame,
    crossterm::event::{self, Event, KeyCode, KeyEventKind},
    layout::{Constraint, Layout},
    style::Style,
    widgets::{List, ListState, Paragraph, Wrap},
};

use self::tree::{Item, Project, Tree};
use super::{
    register, registry,
    rows::{self, State},
    windows::{self, LEAD_WINDOW, Role},
};
use crate::{
    tmux::{self, TmuxTarget, WindowTarget},
    util::current_dir_utf8,
};

const REFRESH: Duration = Duration::from_secs(2);
const FOOTER_LINES: u16 = 3;
const KEYS: &str = "↵ open · r register · q quiet · c close";

/// Bare `niles`: takes the operator to the home session, creating it on first use.
pub fn home() -> Result<()> {
    let home = tmux::open_home(&current_dir_utf8()?)?;
    tmux::switch_or_attach(&TmuxTarget::session(&home))
}

/// `niles explorer`: the tree in the home session's left pane.
pub fn run() -> Result<()> {
    let pane = env::var("TMUX_PANE").context("niles explorer runs in a tmux pane; run `niles`")?;
    let mut explorer = Explorer {
        pane: TmuxTarget::pane(&pane)?,
        cwd: current_dir_utf8()?,
        tree: Tree::default(),
        mode: Mode::Browse,
        footer: None,
    };
    explorer.tree.replace(collect()?);
    let mut terminal = ratatui::init();
    let result = explorer.run(&mut terminal);
    ratatui::restore();
    result
}

enum Mode {
    Browse,
    Directory(String),
    Name {
        path: Utf8PathBuf,
        default: String,
        input: String,
    },
    Close {
        id: String,
        project: Utf8PathBuf,
    },
}

struct Explorer {
    pane: TmuxTarget,
    cwd: Utf8PathBuf,
    tree: Tree,
    mode: Mode,
    /// The outcome of the last action; the key help shows while there is none.
    footer: Option<String>,
}

impl Explorer {
    fn run(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        let mut collected = Instant::now();
        loop {
            terminal.draw(|frame| self.draw(frame))?;
            if event::poll(REFRESH.saturating_sub(collected.elapsed()))?
                && let Event::Key(key) = event::read()?
                && key.kind == KeyEventKind::Press
            {
                self.key(key.code)?;
            }
            if collected.elapsed() >= REFRESH {
                self.tree.replace(collect()?);
                collected = Instant::now();
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
            (Mode::Browse, KeyCode::Enter) => self.footer = Some(shown(self.open())),
            (Mode::Browse, KeyCode::Char('r')) => self.mode = Mode::Directory(String::new()),
            (Mode::Browse, KeyCode::Char(key @ ('q' | 'c'))) => match self.selected_worker() {
                Some((id, project)) if key == 'q' => {
                    self.footer = Some(shown(niles(&["quiet", &id], &project)));
                }
                Some((id, project)) => self.mode = Mode::Close { id, project },
                None => self.footer = Some("select a worker".to_owned()),
            },
            (Mode::Close { id, project }, KeyCode::Char('y')) => {
                self.footer = Some(shown(niles(&["close", id.as_str()], project)));
                self.mode = Mode::Browse;
                self.tree.replace(collect()?);
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
                        self.tree.replace(collect()?);
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

    fn open(&self) -> Result<String> {
        let Some(item) = self.tree.selected() else {
            return Ok("no projects; r registers one".to_owned());
        };
        let entry = &item.project().row.entry;
        let session = entry.name.session()?;
        let window = match item {
            Item::Project(project) => match project.row.state {
                State::Missing => {
                    return Ok(format!("rm ~/.niles/projects/{}", entry.name.as_str()));
                }
                State::NotRunning => {
                    tmux::open_session(&session, &entry.path)?;
                    tmux::configure_status(&session)?;
                    LEAD_WINDOW
                }
                State::Running | State::Waiting(_) => LEAD_WINDOW,
            },
            Item::Window(_, window) => &window.name,
            Item::Lost(_, id) => return Ok(format!("{id}: window lost")),
        };
        let target = WindowTarget::new(session, window)?;
        tmux::show_in_view(&self.pane, &target)?;
        Ok(format!("showing {target}"))
    }

    fn selected_worker(&self) -> Option<(String, Utf8PathBuf)> {
        let (project, id) = match self.tree.selected()? {
            Item::Window(project, window) => match &window.role {
                Role::Worker(id) => (project, id.as_str()),
                Role::Lead | Role::Plain => return None,
            },
            Item::Lost(project, id) => (project, id),
            Item::Project(_) => return None,
        };
        Some((id.to_owned(), project.row.entry.path.clone()))
    }

    fn draw(&self, frame: &mut Frame) {
        let [list, footer] =
            Layout::vertical([Constraint::Fill(1), Constraint::Length(FOOTER_LINES)])
                .areas(frame.area());
        let now = Utc::now();
        let labels = self
            .tree
            .items()
            .into_iter()
            .map(|item| self.tree.label(item, now));
        let mut state = ListState::default().with_selected(Some(self.tree.cursor()));
        let list_widget = List::new(labels).highlight_style(Style::new().reversed());
        frame.render_stateful_widget(list_widget, list, &mut state);
        let prompt = match &self.mode {
            Mode::Browse => None,
            Mode::Directory(input) => Some(format!("directory [{}]: {input}", self.cwd)),
            Mode::Name { default, input, .. } => Some(format!("name [{default}]: {input}")),
            Mode::Close { id, .. } => Some(format!("close {id}? y/n")),
        };
        let text = match (&self.footer, prompt) {
            (Some(message), Some(prompt)) => format!("{message}\n{prompt}"),
            (Some(message), None) => message.clone(),
            (None, Some(prompt)) => prompt,
            (None, None) => KEYS.to_owned(),
        };
        frame.render_widget(Paragraph::new(text).wrap(Wrap { trim: false }), footer);
    }
}

/// An action's outcome as the footer shows it.
fn shown(outcome: Result<String>) -> String {
    match outcome {
        Ok(message) => message,
        Err(error) => format!("{error:#}"),
    }
}

fn collect() -> Result<Vec<Project>> {
    let now = Utc::now();
    rows::collect(registry::entries()?)?
        .into_iter()
        .map(|row| {
            let agents = match row.state {
                State::Running | State::Waiting(_) => Some(windows::session_agents(
                    &row.entry.name.session()?,
                    Some(&row.entry.path),
                    now,
                )?),
                State::Missing | State::NotRunning => None,
            };
            Ok(Project { row, agents })
        })
        .collect()
}

/// Runs a worker command as its own process in the worker's project, so the explorer writes
/// nothing the CLI wouldn't.
fn niles(args: &[&str], project: &Utf8Path) -> Result<String> {
    let output = Command::new(env::current_exe().context("failed to find niles executable")?)
        .args(args)
        .current_dir(project)
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("failed to run niles {}", args.join(" ")))?;
    let stream = if output.status.success() {
        &output.stdout
    } else {
        &output.stderr
    };
    let text = String::from_utf8_lossy(stream)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if !output.status.success() {
        bail!(text);
    }
    Ok(text)
}
