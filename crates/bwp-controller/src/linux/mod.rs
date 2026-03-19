//! Linux Traffic Control Support (TC HTB API generation placeholder).
//!
//! Exposes identical trait interface to Windows implementation but delegates
//! traffic shaping to IP Route2 `tc htb` constructs wrapped around standard PID assignments.
//! Assumes CAP_NET_ADMIN privileges.

use crate::traits::{ThrottleCommand, TrafficController};
use bwp_core::{BandwidthLimit, BwpError, BwpResult};
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};

pub struct LinuxController {
    command_tx: mpsc::Sender<ThrottleCommand>,
    active_rules: Arc<RwLock<Vec<(u32, BandwidthLimit)>>>,
}

impl LinuxController {
    pub fn new() -> BwpResult<Self> {
        let (command_tx, _command_rx) = mpsc::channel(256);
        let active_rules = Arc::new(RwLock::new(Vec::new()));

        // In Linux, we'd spawn a loop to process commands identically to WinDivert
        // and push netlink `tc` operations per PID dynamically using eBPF maps or cgroups.

        Ok(Self {
            command_tx,
            active_rules,
        })
    }
}

impl TrafficController for LinuxController {
    async fn send_command(&self, command: ThrottleCommand) -> BwpResult<()> {
        self.command_tx
            .send(command)
            .await
            .map_err(|e| BwpError::Controller(format!("Linux Controller Queue closed: {}", e)))
    }

    async fn active_rules(&self) -> BwpResult<Vec<(u32, BandwidthLimit)>> {
        let lock = self.active_rules.read().await;
        Ok(lock.clone())
    }
}
