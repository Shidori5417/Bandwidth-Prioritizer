//! Platform-agnostic traffic control traits.
//!
//! [`TrafficController`] is the core abstraction for applying and removing
//! bandwidth limits on a per-process basis.

use bwp_core::{BandwidthLimit, BwpResult};

/// An explicit command dictating how a specific PID should be handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThrottleCommand {
    /// Throttle this process with a specified limit.
    ApplyLimit {
        /// Process to throttle.
        pid: u32,
        /// Bandwidth cap to enforce.
        limit: BandwidthLimit,
    },
    /// Allow this process free flow. Removes from throttle list.
    RemoveLimit {
        /// Process to release.
        pid: u32,
    },
    /// Clear all limits across the system. Fully unblock all flows.
    ClearAll,
}

/// Trait for applying traffic shaping rules at the OS level.
///
/// The intended design is a WinDivert filter on Windows and `tc htb` classes on Linux.
/// Neither backend is implemented yet; see the crate docs.
#[allow(async_fn_in_trait)]
pub trait TrafficController: Send + Sync {
    /// Send a throttle command to the engine. This pushes the update
    /// as an event to the underlying networking thread without blocking.
    async fn send_command(&self, command: ThrottleCommand) -> BwpResult<()>;

    /// Returns the list of PIDs currently being throttled, and their limits.
    async fn active_rules(&self) -> BwpResult<Vec<(u32, BandwidthLimit)>>;
}
