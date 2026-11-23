//! # bwp-core
//!
//! Core types, traits, and shared abstractions for **Bandwidth-Prioritizer**.
//!
//! This crate defines the foundational building blocks used across all other
//! crates in the workspace. It contains:
//!
//! - **Domain types**: Process metadata, bandwidth limits, connection info
//! - **Error types**: Unified error hierarchy via `thiserror`
//! - **Traits**: Platform-agnostic abstractions for monitoring and traffic control
//! - **Configuration models**: Strongly-typed config structures

pub mod config;
pub mod error;
pub mod ipc;
pub mod manager;
pub mod pid;
pub mod types;

pub use config::AppConfig;
pub use error::{BwpError, BwpResult};
pub use manager::ConfigManager;
pub use types::{
    BandwidthLimit, ConnectionInfo, ConnectionState, MonitorEvent, ProcessInfo, Protocol,
    ThrottleState,
};
