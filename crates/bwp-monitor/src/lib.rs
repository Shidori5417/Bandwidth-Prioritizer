//! # bwp-monitor
//!
//! Process and network connection monitoring for **Bandwidth-Prioritizer**.
//!
//! Provides platform-agnostic traits and platform-specific implementations
//! for detecting active processes and their network connections.
//!
//! On Windows the monitor polls `GetExtendedTcpTable` / `GetExtendedUdpTable` and turns
//! the difference between successive snapshots into connection events (ETW is not
//! implemented). The snapshot diffing and table parsing helpers are platform independent
//! and unit tested everywhere.

pub mod diff;
pub mod table;
pub mod traits;

/// Windows monitor implementation (IP Helper polling).
#[cfg(windows)]
pub mod windows;

#[cfg(windows)]
pub use windows::WindowsMonitor;
#[cfg(windows)]
pub use windows::snapshot_connections;

pub use traits::ProcessMonitor;
