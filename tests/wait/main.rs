#![allow(clippy::unwrap_used, clippy::expect_used, reason = "test assertions")]

#[path = "../common/mod.rs"]
mod common;
mod mcp;
mod send;
mod status_lines;
mod support;
mod target_selection;
mod window_liveness;
mod workspace;
