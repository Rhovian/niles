pub(crate) const VERSION: &str = env!("CARGO_PKG_VERSION");
pub(crate) const GIT_HASH: &str = env!("NILES_BUILD_GIT_HASH");
pub(crate) const BUILD_TIMESTAMP: &str = env!("NILES_BUILD_TIMESTAMP");
pub(crate) const BUILD_HEAD_TIMESTAMP: &str = env!("NILES_BUILD_HEAD_TIMESTAMP");
pub(crate) const CLAP_VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    " (",
    env!("NILES_BUILD_GIT_HASH"),
    ", built ",
    env!("NILES_BUILD_TIMESTAMP"),
    ")"
);

pub(crate) fn identity() -> String {
    format!("niles {VERSION} ({GIT_HASH}, built {BUILD_TIMESTAMP})")
}
