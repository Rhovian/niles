use super::{Directory, directory};
use anyhow::Result;
use camino::{Utf8Path, Utf8PathBuf};
use ratatui::crossterm::event::KeyCode;

pub(crate) enum Prompt {
    Directory(String),
    Name {
        path: Utf8PathBuf,
        default: String,
        input: String,
    },
    Confirm {
        path: Utf8PathBuf,
        name: String,
    },
}
pub(crate) enum Action {
    Pending,
    Cancel,
    Register { path: Utf8PathBuf, name: String },
}
impl Prompt {
    pub fn new() -> Self {
        Self::Directory(String::new())
    }
    pub fn text(&self, root: &Utf8Path) -> String {
        match self {
            Self::Directory(input) => {
                format!("directory: {}/{input}", root.as_str().trim_end_matches('/'))
            }
            Self::Name { default, input, .. } => format!("name [{default}]: {input}"),
            Self::Confirm { path, name } => format!("Path: {path}\nName: {name}\nRegister? y/N"),
        }
    }
    #[expect(clippy::wildcard_enum_match_arm, reason = "ignore unbound prompt keys")]
    pub fn key(&mut self, code: KeyCode, root: &Utf8Path) -> Result<Action> {
        if code == KeyCode::Esc {
            return Ok(Action::Cancel);
        }
        let next = match (&mut *self, code) {
            (Self::Directory(input) | Self::Name { input, .. }, KeyCode::Char(c)) => {
                input.push(c);
                None
            }
            (Self::Directory(input) | Self::Name { input, .. }, KeyCode::Backspace) => {
                input.pop();
                None
            }
            (Self::Directory(input), KeyCode::Enter) => {
                // camino 1.2.6 Utf8Path::join replaces the base for absolute input;
                // this field is strictly the suffix of the displayed root.
                let path = format!("{}/{input}", root.as_str().trim_end_matches('/'));
                match directory(&path, root)? {
                    Directory::Registered(entry) => {
                        anyhow::bail!("already registered as {}", entry.name.as_str())
                    }
                    Directory::Unregistered { path, default } => Some(Self::Name {
                        path,
                        default,
                        input: String::new(),
                    }),
                }
            }
            (
                Self::Name {
                    path,
                    default,
                    input,
                },
                KeyCode::Enter,
            ) => {
                let name = if input.is_empty() {
                    default.clone()
                } else {
                    input.clone()
                };
                super::ProjectName::parse(&name)?;
                Some(Self::Confirm {
                    path: path.clone(),
                    name,
                })
            }
            (Self::Confirm { path, name }, KeyCode::Char('y' | 'Y')) => {
                let action = Action::Register {
                    path: path.clone(),
                    name: name.clone(),
                };
                // A failed registration must reopen name entry, not retry the same confirmation.
                *self = Self::Name {
                    path: path.clone(),
                    default: name.clone(),
                    input: String::new(),
                };
                return Ok(action);
            }
            (Self::Confirm { .. }, _) => return Ok(Action::Cancel),
            _ => None,
        };
        if let Some(next) = next {
            *self = next;
        }
        Ok(Action::Pending)
    }
}
