use super::{
    registry::{self, Entry, ProjectName},
    rows::{self, Row, State},
};
use crate::{
    tmux,
    util::{current_dir_utf8, utf8_path},
};
use anyhow::{Context, Result, bail};
use camino::Utf8PathBuf;
use chrono::Utc;
use std::{
    fs,
    io::{self, ErrorKind, IsTerminal, Write},
};

pub fn run() -> Result<()> {
    if !io::stdin().is_terminal() {
        bail!("the project list requires a terminal on stdin");
    }
    loop {
        let rows = rows::collect(registry::entries()?)?;
        print_rows(&rows);
        let Some(line) = prompt("number to open · n new project · q quit")? else {
            return Ok(());
        };
        match line.trim() {
            "q" => return Ok(()),
            "n" => {
                if let Some(entry) = new_project()? {
                    open(&entry)?;
                    return Ok(());
                }
            }
            value => match value.parse::<usize>() {
                Ok(n) if n > 0 && n <= rows.len() => {
                    let row = &rows[n - 1];
                    if matches!(row.state, State::Missing) {
                        println!("rm ~/.niles/projects/{}", row.entry.name.as_str());
                    } else {
                        open(&row.entry)?;
                        return Ok(());
                    }
                }
                _ => println!("Invalid choice. Enter a listed number, n, or q."),
            },
        }
    }
}

fn open(entry: &Entry) -> Result<()> {
    let session = entry.name.session()?;
    tmux::open_session(&session, &entry.path)?;
    tmux::switch_or_attach(&session)
}

fn prompt(label: &str) -> Result<Option<String>> {
    print!("{label}: ");
    io::stdout().flush()?;
    let mut line = String::new();
    if io::stdin().read_line(&mut line)? == 0 {
        return Ok(None);
    }
    Ok(Some(line.trim().to_owned()))
}

fn new_project() -> Result<Option<Entry>> {
    let cwd = current_dir_utf8()?;
    let path = loop {
        let Some(input) = prompt(&format!("directory [{cwd}]"))? else {
            return Ok(None);
        };
        let candidate = if input.is_empty() {
            cwd.clone()
        } else {
            Utf8PathBuf::from(input)
        };
        match fs::canonicalize(&candidate) {
            Ok(path) if path.is_dir() => break utf8_path(path, "project path")?,
            Ok(_) => println!("Directory does not exist: {candidate}"),
            Err(error) => println!("Cannot open directory {candidate}: {error}"),
        }
    };
    if let Some(existing) = registry::entries()?
        .into_iter()
        .find(|entry| entry.path == path)
    {
        println!("already registered as {}", existing.name.as_str());
        return Ok(None);
    }
    let default = path
        .file_name()
        .context("project directory has no basename")?;
    loop {
        let Some(input) = prompt(&format!("name [{default}]"))? else {
            return Ok(None);
        };
        let name = match ProjectName::parse(if input.is_empty() { default } else { &input }) {
            Ok(name) => name,
            Err(error) => {
                println!("{error}");
                continue;
            }
        };
        match registry::register(&name, &path) {
            Ok(()) => return Ok(Some(Entry { name, path })),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                println!("Name {} is already taken.", name.as_str())
            }
            Err(error) => return Err(error).context("failed to register project"),
        }
    }
}

fn print_rows(rows: &[Row]) {
    println!("niles — projects");
    for (index, row) in rows.iter().enumerate() {
        let status = match row.state {
            State::Missing => format!("missing · rm ~/.niles/projects/{}", row.entry.name.as_str()),
            State::NotRunning => "not running".to_owned(),
            State::Running => "running".to_owned(),
            State::Waiting(Some(since)) => {
                format!("waiting {}m", (Utc::now() - since).num_minutes().max(0))
            }
            State::Waiting(None) => "waiting".to_owned(),
        };
        let tokens = match row.lead_tokens {
            Some(n) => format!(" · lead {}", rows::abbreviate(n)),
            None => String::new(),
        };
        println!(
            "{:>2} {}  {}  {status}{tokens} · {} workers",
            index + 1,
            row.entry.name.as_str(),
            row.entry.path,
            row.workers
        );
    }
}
