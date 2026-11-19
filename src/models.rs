use crate::agents::{self, ModelCatalogue};
use crate::util::print_structured_rows;
use anyhow::{Result, ensure};

pub(crate) fn models() -> Result<()> {
    let catalogue = ModelCatalogue::default();
    let mut rows = Vec::new();
    let mut errors = Vec::new();
    for profile in agents::profiles() {
        match catalogue.model_rows(profile.id) {
            Ok(listed) => rows.extend(listed),
            Err(error) => errors.push(error.to_string()),
        }
    }
    print_structured_rows(
        "models",
        ["family", "model", "effort", "context_window"],
        &rows,
    );
    ensure!(errors.is_empty(), "{}", errors.join("\n"));
    Ok(())
}
