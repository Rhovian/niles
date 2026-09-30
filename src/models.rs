use anyhow::Result;

use crate::{
    config::spec::load_project_config_from,
    util::{current_dir_utf8, print_structured_rows},
};

pub(crate) fn models() -> Result<()> {
    let config = load_project_config_from(&current_dir_utf8()?)?;
    print_structured_rows(
        "models",
        ["family", "model", "efforts"],
        &config.models.rows(),
    );
    Ok(())
}
