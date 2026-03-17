//! Windows-specific process and network monitoring implementation.
//!
//! This module uses a hybrid approach:
//! 1. **ETW (Event Tracing for Windows)**: Primary fast-path providing real-time
//!    connection events via `ferrisetw` and the `Microsoft-Windows-TCPIP` provider.
//! 2. **Polling (GetExtendedTcpTable)**: Graceful fallback if the ETW session
//!    fails to start (e.g. lack of admin privileges or Group policies).

mod etw;
mod poll;

use crate::traits::ProcessMonitor;
use bwp_core::{BwpError, BwpResult, ConnectionInfo, MonitorEvent, ProcessInfo};
use tokio::sync::mpsc;
use tracing::{info, warn};

/// Windows process monitor, implementing the hybrid ETW/Polling strategy.
pub struct WindowsMonitor {
    // We could store configuration here, like poll_interval overrides,
    // ETW session names, etc.
}

impl WindowsMonitor {
    /// Creates a new Windows monitor.
    pub fn new() -> Self {
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
