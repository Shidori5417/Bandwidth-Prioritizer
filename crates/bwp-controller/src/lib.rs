//! Bandwidth Control Engine.
//!
//! Provides the core abstraction for applying traffic control limits across
//! platforms. The controller layer receives `ThrottleCommand`s and dispatches
//! them to the platform-specific backend (WinDivert on Windows, `tc` on Linux).

pub mod token_bucket;
pub mod traits;

#[cfg(target_os = "windows")]
pub mod windows;

#[cfg(target_os = "linux")]
pub mod linux;

#[cfg(target_os = "windows")]
pub use windows::WindowsController;

#[cfg(target_os = "linux")]
pub use linux::LinuxController;

pub use traits::{ThrottleCommand, TrafficController};
