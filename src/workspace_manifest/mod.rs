mod interactive;
mod io;
mod roles_table;
mod types;
mod worktree;

pub(crate) use interactive::ensure_interactive;
pub use io::{load, manifest_path, save};
pub(crate) use roles_table::{MISSING, clamp, manifest_roles};
pub use types::{RoleBinding, WorkspaceManifest};
pub use worktree::WorktreeConfig;
