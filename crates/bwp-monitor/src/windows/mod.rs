//! Windows-specific process and network monitoring implementation.
//!
//! The design is hybrid (ETW real-time events with a polling fallback). ETW is **not
//! implemented**: its listener always reports an error, so the monitor always uses the
//! polling path, which snapshots the TCP (IPv4 and IPv6) and UDP tables through
//! `GetExtendedTcpTable` / `GetExtendedUdpTable` and diffs successive snapshots into
//! connection events.

mod etw;
mod poll;

use crate::traits::ProcessMonitor;
use bwp_core::{BwpResult, ConnectionInfo, MonitorEvent, ProcessInfo};
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{info, warn};

/// Windows process monitor (ETW is not implemented, so it polls the IP Helper tables).
#[derive(Debug)]
pub struct WindowsMonitor {
    poll_interval: Duration,
}

impl Default for WindowsMonitor {
    fn default() -> Self {
        Self::new()
    }
}

impl WindowsMonitor {
    /// Creates a monitor polling every 500 ms.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            poll_interval: poll::DEFAULT_POLL_INTERVAL,
        }
    }

    /// Creates a monitor with a custom polling interval (clamped to 100 ms ..= 10 s).
    #[must_use]
    pub fn with_poll_interval(interval: Duration) -> Self {
        Self {
            poll_interval: interval.clamp(Duration::from_millis(100), Duration::from_secs(10)),
        }
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
            poll::spawn_polling_loop(tx, self.poll_interval);
        } else {
            info!("ETW connection monitor successfully started.");
        }

        Ok(rx)
    }

    async fn list_network_processes(&self) -> BwpResult<Vec<ProcessInfo>> {
        // The blocking Win32 calls and process lookups run off the async worker threads.
        run_blocking(poll::snapshot_network_processes).await
    }

    async fn list_connections(&self) -> BwpResult<Vec<ConnectionInfo>> {
        run_blocking(poll::snapshot_connections).await
    }
}

pub use poll::snapshot_connections;

async fn run_blocking<T: Send + 'static>(
    job: impl FnOnce() -> BwpResult<T> + Send + 'static,
) -> BwpResult<T> {
    tokio::task::spawn_blocking(job)
        .await
        .map_err(|e| bwp_core::BwpError::Monitor(format!("snapshot task failed: {e}")))?
}
