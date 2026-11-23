//! Windows-specific process and network monitoring implementation.
//!
//! **Status: not implemented.** The design is hybrid (ETW real-time events with a
//! `GetExtendedTcpTable` polling fallback), but the ETW listener always reports an
//! error and the polling fallback returns empty snapshots, so no connection events
//! are produced yet.

mod etw;
mod poll;

use crate::traits::ProcessMonitor;
use bwp_core::{BwpResult, ConnectionInfo, MonitorEvent, ProcessInfo};
use tokio::sync::mpsc;
use tracing::{info, warn};

/// Windows process monitor, implementing the hybrid ETW/Polling strategy.
#[derive(Debug, Default)]
pub struct WindowsMonitor {}

impl WindowsMonitor {
    /// Creates a new Windows monitor.
    #[must_use]
    pub const fn new() -> Self {
        Self {}
    }
}

impl ProcessMonitor for WindowsMonitor {
    async fn watch_connections(&self) -> BwpResult<mpsc::Receiver<MonitorEvent>> {
        let (tx, rx) = mpsc::channel(1024);

        // Attempt to spawn the ETW event listener.
        if let Err(e) = etw::spawn_etw_listener(tx.clone()) {
            warn!(
                %e,
                "Failed to start ETW listener. Falling back to polling mode."
            );

            // Fallback to polling.
            poll::spawn_polling_loop(tx);
        } else {
            info!("ETW connection monitor successfully started.");
        }

        Ok(rx)
    }

    async fn list_network_processes(&self) -> BwpResult<Vec<ProcessInfo>> {
        // Delegate to the polling engine for static snapshots.
        poll::snapshot_network_processes()
    }

    async fn list_connections(&self) -> BwpResult<Vec<ConnectionInfo>> {
        // Delegate to the polling engine for static snapshots.
        poll::snapshot_connections()
    }
}

// Ensure the module is properly structured and fails fast if not on windows.
#[cfg(not(target_os = "windows"))]
compile_error!("bwp-monitor's windows module should only be compiled on Windows targets.");
