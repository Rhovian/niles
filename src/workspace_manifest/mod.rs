mod interactive;
mod io;
mod roles_table;
mod types;

pub use interactive::ensure_interactive;
pub use io::{load, manifest_path, save};
pub(crate) use roles_table::{MISSING, clamp, manifest_roles};
pub use types::{AgentGroup, ReviewerBinding, RoleBinding, WorkspaceManifest};
#[cfg(test)]
pub use types::{PlanningGroup, WorkerPlanning};
