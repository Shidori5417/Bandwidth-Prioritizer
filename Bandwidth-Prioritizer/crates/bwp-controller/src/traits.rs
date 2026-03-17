//! Platform-agnostic traffic control traits.
//!
//! [`TrafficController`] is the core abstraction for applying and removing
//! bandwidth limits on a per-process basis.

use bwp_core::{BandwidthLimit, BwpResult};

/// Trait for applying traffic shaping rules at the OS level.
///
/// Platform-specific implementations handle the actual kernel or
/// driver interaction (WFP on Windows, tc/eBPF on Linux).
#[allow(async_fn_in_trait)]
pub trait TrafficController: Send + Sync {
    /// Apply a bandwidth limit to a specific process (by PID).
    ///
    /// If a rule already exists for this PID, it should be updated.
    async fn throttle_process(&self, pid: u32, limit: BandwidthLimit) -> BwpResult<()>;

    /// Remove the bandwidth limit for a specific process.
    ///
    /// No-op if no rule exists for this PID.
    async fn unthrottle_process(&self, pid: u32) -> BwpResult<()>;

    /// Remove all active throttling rules (e.g. on shutdown).
    async fn clear_all_rules(&self) -> BwpResult<()>;

    /// Returns the list of PIDs currently being throttled.
    async fn active_rules(&self) -> BwpResult<Vec<(u32, BandwidthLimit)>>;
}
