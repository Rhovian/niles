mod interactive;
mod io;
mod roles_table;
mod types;

pub use interactive::ensure_interactive;
pub use io::{load, manifest_path, save};
pub(crate) use roles_table::{clamp, manifest_roles};
pub(crate) use types::DEFAULT_REVIEWER_AGENT;
#[cfg(test)]
pub use types::{AgentGroup, PlanningGroup, WorkerPlanning};
pub use types::{ReviewerBinding, RoleBinding, WorkspaceManifest};
