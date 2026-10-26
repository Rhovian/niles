mod interactive;
mod io;
pub(crate) mod setup;
mod skills_table;
mod types;
mod worktree;

pub(crate) use interactive::ensure_interactive;
pub use io::{load, manifest_path, save};
pub(crate) use skills_table::{MISSING, clamp, manifest_skills};
pub use types::{Binding, WorkspaceManifest};
pub use worktree::WorktreeConfig;
