//! Bandwidth Control Engine.
//!
//! Provides the core abstraction for applying traffic control limits across
//! platforms. The controller layer receives `ThrottleCommand`s and dispatches
//! them to the platform-specific backend.
//!
//! **Status:**
//! - By default no backend shapes traffic: the controllers accept commands and ignore them.
//! - Windows: an **experimental, opt-in** WinDivert packet shaper exists. It needs the
//!   `experimental-windivert` Cargo feature *and* `[shaping] experimental_windivert = true`
//!   in `config.toml`; see [`windows::divert`].
//! - Linux: the `tc` controller is a no-op placeholder.
//!
//! The platform-independent pieces ([`token_bucket`], [`shaper`], [`flow`], [`portmap`])
//! are fully unit tested on every platform.

/// IP/TCP/UDP header parsing for packet attribution.
pub mod flow;
/// Local port to PID attribution.
pub mod portmap;
/// Per-PID token buckets with a delay queue.
pub mod shaper;
/// Token bucket rate limiter.
pub mod token_bucket;
pub mod traits;

/// Windows controller (no-op by default, experimental WinDivert shaper opt-in).
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
