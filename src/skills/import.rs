use super::reference::{GithubRef, PinnedRef};
use super::{Origin, SkillName, files, global_dir, resolve};
use crate::util;
use anyhow::{Context, Result, ensure};
use camino::{Utf8Path, Utf8PathBuf};
use pulldown_cmark::{Event, Parser};
use std::collections::BTreeSet;
use std::fs;
use std::io::Write;

#[derive(Debug, clap::Subcommand)]
pub(crate) enum SkillsCommand {
    /// Pin a GitHub skill directory for subsequent launches.
    Import {
        reference: GithubRef,
        #[arg(long)]
        global: bool,
    },
}
impl SkillsCommand {
    pub fn run(self) -> Result<()> {
        let Self::Import { reference, global } = self;
        let origin = if global {
            Origin::Global
        } else {
            Origin::Workspace
        };
        import(reference, origin, ImportMode::Replace).map(|_| ())
    }
}

#[derive(Clone, Copy)]
pub(crate) enum ImportMode {
    Replace,
    NewPrimary,
}

pub(crate) fn import(
    reference: GithubRef,
    origin: Origin,
    mode: ImportMode,
) -> Result<Utf8PathBuf> {
    let home = global_dir()?;
    let root = match origin {
        Origin::Global => home.clone(),
        Origin::Workspace => util::current_dir_utf8()?.join(".niles"),
    };
    let primary = prepare(reference, origin, mode, &root, &home)?;
    let pointer = primary.pointer.clone();
    let mut visited = BTreeSet::from([primary.skill.name.clone()]);
    let mut pending = vec![primary];
    let mut prepared = Vec::new();
    // Cycles reuse the root's name even when its imported path is outside skills/.
    while let Some(import) = pending.pop() {
        let reference = import.pinned.reference();
        for name in required_skills(&import.skill.body) {
            if visited.insert(name.clone()) {
                pending.push(prepare(
                    GithubRef {
                        path: format!("skills/{name}"),
                        name,
                        ..reference.clone()
                    },
                    origin,
                    ImportMode::Replace,
                    &root,
                    &home,
                )?);
            }
        }
        prepared.push(import);
    }
    for import in prepared.into_iter().rev() {
        import.publish()?;
    }
    Ok(pointer)
}

struct PreparedImport {
    pointer: Utf8PathBuf,
    previous: Option<PinnedRef>,
    pinned: PinnedRef,
    skill: resolve::Skill,
    mode: ImportMode,
}

fn prepare(
    reference: GithubRef,
    origin: Origin,
    mode: ImportMode,
    root: &Utf8Path,
    home: &Utf8Path,
) -> Result<PreparedImport> {
    let dir = root.join("skills").join(reference.name.as_str());
    let markdown = dir.join("SKILL.md");
    let pointer = dir.join("SKILL.ref");
    ensure!(
        !files::exists(&markdown, origin)?,
        "local skill already exists: {markdown}"
    );
    let previous: Option<PinnedRef> = if files::exists(&pointer, origin)? {
        ensure!(
            matches!(mode, ImportMode::Replace),
            "skill already installed: {pointer}"
        );
        // Invalid persisted state requires explicit removal rather than an
        // implicit repair that cannot report the old commit reliably.
        Some(
            PinnedRef::document(&files::read(&pointer, origin)?).with_context(|| {
                format!("invalid existing pointer; delete {pointer} before re-importing")
            })?,
        )
    } else {
        None
    };
    let pinned = super::cache::pin(reference)?;
    let skill = resolve::imported(&pinned, home)?;
    if matches!(mode, ImportMode::NewPrimary) {
        skill.primary_body()?;
    }
    Ok(PreparedImport {
        pointer,
        previous,
        pinned,
        skill,
        mode,
    })
}

impl PreparedImport {
    fn publish(self) -> Result<()> {
        let Self {
            pointer,
            previous,
            pinned,
            skill,
            mode,
        } = self;
        let dir = pointer.parent().context("skill pointer has no parent")?;
        fs::create_dir_all(dir)?;
        let mut staged = tempfile::NamedTempFile::new_in(dir)?;
        writeln!(staged, "{pinned}")?;
        // tempfile 3.27.0 NamedTempFile::persist and persist_noclobber publish complete files
        // with the requested replacement policy: docs.rs/tempfile/3.27.0/tempfile/struct.NamedTempFile.html.
        match mode {
            ImportMode::Replace => staged.persist(&pointer),
            ImportMode::NewPrimary => staged.persist_noclobber(&pointer),
        }
        .with_context(|| format!("failed to write {pointer}"))?;
        if let Some(previous) = previous {
            println!("{} → {}", previous.reference().rev, pinned.reference().rev);
        }
        println!("skill: {skill}");
        Ok(())
    }
}

fn required_skills(body: &str) -> Vec<SkillName> {
    let mut names = Vec::new();
    // pulldown-cmark 0.13.5 src/lib.rs Event::Code separates inline code from prose.
    // Require a whole command in a code span or text line to avoid treating prose as loads.
    for event in Parser::new(body) {
        if let Event::Code(text) | Event::Text(text) = event {
            for line in text.lines() {
                let words: Vec<_> = line.split_whitespace().collect();
                if let ["niles", "agent", "skill", name] = words.as_slice()
                    && !name.starts_with('-')
                    && (name.bytes().any(|ch| ch.is_ascii_lowercase())
                        || !name.bytes().any(|ch| ch.is_ascii_uppercase()))
                    && let Ok(name) = name.parse()
                {
                    names.push(name);
                }
            }
        }
    }
    names
}
