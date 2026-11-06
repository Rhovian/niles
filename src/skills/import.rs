use super::{
    Origin, files, global_dir,
    reference::{GithubRef, PinnedRef},
    resolve,
};
use anyhow::{Context, Result, ensure};
use camino::Utf8PathBuf;
use std::{fs, io::Write};

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
        Origin::Workspace => crate::util::current_dir_utf8()?.join(".niles"),
    };
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
    let skill = resolve::imported(&pinned, &home)?;
    if matches!(mode, ImportMode::NewPrimary) {
        skill.primary_body()?;
    }
    fs::create_dir_all(&dir)?;
    let mut staged = tempfile::NamedTempFile::new_in(&dir)?;
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
    Ok(pointer)
}
