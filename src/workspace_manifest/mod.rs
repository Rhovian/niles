mod interactive;
mod io;
mod roles_table;
#[cfg(test)]
mod test_support;
mod types;

pub use interactive::ensure_interactive;
pub use io::{load, manifest_path, save};
pub use types::WorkspaceManifest;
