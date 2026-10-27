use super::{
    Origin, files, global_dir,
    reference::{GithubRef, PinnedRef},
    resolve,
};
use anyhow::{Context, Result, ensure};
use std::fs;

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
        let home = global_dir()?;
        let (root, origin) = if global {
            (home.clone(), Origin::Global)
        } else {
            (
                crate::util::current_dir_utf8()?.join(".niles"),
                Origin::Workspace,
            )
        };
        let dir = root.join("skills").join(reference.name.as_str());
        let markdown = dir.join("SKILL.md");
        let pointer = dir.join("SKILL.ref");
        ensure!(
            !files::exists(&markdown, origin)?,
            "local skill already exists: {markdown}"
        );
        let previous: Option<PinnedRef> = if files::exists(&pointer, origin)? {
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
        fs::create_dir_all(&dir)?;
        // Rust 1.89 std/fs.rs: write follows symlinks and exposes partial pointers; publish a complete
        // sibling with rename, after the same origin checks used by the resolver.
        let temporary = dir.join(format!(".SKILL.ref-{}", uuid::Uuid::new_v4()));
        fs::write(&temporary, format!("{pinned}\n"))?;
        fs::rename(&temporary, &pointer).with_context(|| format!("failed to write {pointer}"))?;
        if let Some(previous) = previous {
            println!("{} → {}", previous.reference().rev, pinned.reference().rev);
        }
        println!("skill: {skill}");
        Ok(())
    }
}
