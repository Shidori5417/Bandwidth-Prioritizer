//! Bandwidth Control Engine.
//!
//! Provides the core abstraction for applying traffic control limits across
//! platforms. The controller layer receives `ThrottleCommand`s and dispatches
//! them to the platform-specific backend.
//!
//! **Status:** no backend shapes traffic yet. The Windows controller (WinDivert) and the
//! Linux controller (`tc`) accept commands but do not limit anything; only the
//! [`token_bucket::TokenBucket`] algorithm itself is implemented.

/// Token bucket rate limiter.
pub mod token_bucket;
pub mod traits;

/// Windows controller (currently a placeholder, see crate docs).
#[cfg(windows)]
pub mod windows;

/// Linux controller (placeholder).
#[cfg(target_os = "linux")]
pub mod linux;

#[cfg(windows)]
pub use windows::WindowsController;

#[cfg(target_os = "linux")]
pub use linux::LinuxController;

pub use traits::{ThrottleCommand, TrafficController};
