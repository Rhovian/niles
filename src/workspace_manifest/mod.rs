mod interactive;
mod io;
mod roles_table;
mod types;

pub use interactive::ensure_interactive;
pub use io::{load, manifest_path, save};
pub use types::WorkspaceManifest;
