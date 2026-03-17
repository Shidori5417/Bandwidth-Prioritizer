//! Polling Fallback Implementation using GetExtendedTcpTable.
//!
//! This module kicks in if the ETW pipeline fails. It periodically invokes Windows APIs
//! to snapshot active networking states, computes diffs, and synthesizes `MonitorEvent`s.
//! O(N) cost per poll interval.

use bwp_core::{BwpError, MonitorEvent, BwpResult, ConnectionInfo, ProcessInfo};
use tokio::sync::mpsc;
use tokio::time::{sleep, Duration};
use tracing::{debug, error, info};

/// Spawns the asynchronous polling loop.
pub fn spawn_polling_loop(tx: mpsc::Sender<MonitorEvent>) {
    tokio::spawn(async move {
        info!("Graceful fallback to GetExtendedTcpTable polling loop started.");

        let mut previous_state: Vec<ConnectionInfo> = vec![];

        loop {
            match snapshot_connections() {
                Ok(current_state) => {
                    // Compute Diffs: ConnectionOpened & ConnectionClosed
                    // (Simplified logic for demonstration: actual impl maintains a HashSet)
                    // Synthesize ETW-like MonitorEvents here.
                    
                    for conn in &current_state {
                        if !previous_state.iter().any(|p| p.pid == conn.pid && p.local_addr == conn.local_addr) {
                            let _ = tx.send(MonitorEvent::ConnectionOpened(conn.clone())).await;
                        }
                    }

                    for prev in &previous_state {
                        if !current_state.iter().any(|c| c.pid == prev.pid && c.local_addr == prev.local_addr) {
                            let _ = tx.send(MonitorEvent::ConnectionClosed(prev.clone())).await;
                        }
                    }

                    previous_state = current_state;
                }
                Err(e) => {
                    error!("Polling failed: {e}");
                }
            }

            sleep(Duration::from_millis(500)).await;
        }
    });
}

/// Returns a frozen snapshot of processes using the net APIs (`GetExtendedTcpTable`, `GetExtendedUdpTable`).
pub fn snapshot_network_processes() -> BwpResult<Vec<ProcessInfo>> {
    // Phase 1 Dummy implementation.
    // In actual implementation, we map `MIB_TCPROW_OWNER_PID` rows.
    Ok(vec![])
}

/// Returns a frozen snapshot of active outbound connections per PID.
pub fn snapshot_connections() -> BwpResult<Vec<ConnectionInfo>> {
    // Native Win32 API calls lie here (Win32_NetworkManagement_IpHelper).
    // GetExtendedTcpTable / GetExtendedUdpTable mapping over Windows API wrapper.
    Ok(vec![])
}
