//! # bwp-monitor
//!
//! Process and network connection monitoring for **Bandwidth-Prioritizer**.
//!
//! Provides platform-agnostic traits and platform-specific implementations
//! for detecting active processes and their network connections.

pub mod traits;

#[cfg(target_os = "windows")]
pub mod windows;

#[cfg(target_os = "windows")]
pub use windows::WindowsMonitor;

pub use traits::ProcessMonitor;
