#![allow(clippy::unwrap_used, clippy::expect_used, reason = "test assertions")]

#[path = "../common/mod.rs"]
mod common;
mod send;
mod spawn_behavior;
mod status_lines;
mod support;
mod target_selection;
mod window_liveness;
mod workspace;
