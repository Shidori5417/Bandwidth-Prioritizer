//! Platform-agnostic monitoring traits.
//!
//! These traits define the contract that platform-specific implementations
//! (Linux, Windows) must satisfy. This enables clean dependency inversion
//! and makes testing with mocks straightforward.

use bwp_core::{BwpResult, ConnectionInfo, ProcessInfo, MonitorEvent};
use tokio::sync::mpsc;

/// Trait for monitoring running processes and their network connections.
///
/// Implementors provide platform-specific logic. A hybrid approach is encouraged:
/// prefer event-driven streams (like ETW on Windows) and fallback to polling.
#[allow(async_fn_in_trait)]
pub trait ProcessMonitor: Send + Sync {
    /// Spawns the monitor task and returns a channel receiver yielding real-time
    /// connection events. If the platform does not support events (or fails), 
    /// it may fall back to generating these events via polling.
    async fn watch_connections(&self) -> BwpResult<mpsc::Receiver<MonitorEvent>>;
    /// Returns a snapshot of all currently running processes with active
    /// network connections.
    async fn list_network_processes(&self) -> BwpResult<Vec<ProcessInfo>>;

    /// Returns all active network connections, each tagged with its owning PID.
    async fn list_connections(&self) -> BwpResult<Vec<ConnectionInfo>>;

    /// Checks whether a specific process (by name, case-insensitive) is
    /// currently running and has active network connections.
    async fn is_process_active(&self, process_name: &str) -> BwpResult<bool> {
        let processes = self.list_network_processes().await?;
        Ok(processes
            .iter()
            .any(|p| p.name.eq_ignore_ascii_case(process_name)))
    }

    /// Returns all processes whose names match any entry in the given list.
    async fn find_matching_processes(
        &self,
        names: &[String],
    ) -> BwpResult<Vec<ProcessInfo>> {
        let processes = self.list_network_processes().await?;
        Ok(processes
            .into_iter()
            .filter(|p| {
                names
                    .iter()
                    .any(|n| p.name.eq_ignore_ascii_case(n))
            })
            .collect())
    }
}
