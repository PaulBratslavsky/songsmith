//! Songsmith Studio — Rust core.
//!
//! Owns all state: models, libSQL persistence, the tool registry, and the stage
//! agent loop. The Tauri shell and (later) the MCP server are thin layers over
//! this crate.

pub mod ableton;
pub mod agent;
pub mod db;
pub mod engine;
pub mod freeze;
pub mod midi;
pub mod models;
pub mod render;
pub mod spine;
pub mod tools;

pub use models::*;
