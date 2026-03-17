use crate::traits::{ThrottleCommand, TrafficController};
use bwp_core::{BandwidthLimit, BwpResult, BwpError};
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};

pub mod divert;

/// Windows WinDivert implementation of the Traffic Controller.
/// 
/// Intercepts traffic at the network layer strictly for PIDs requested via `ThrottleCommand`.
/// Bypasses WinDivert overhead entirely for whitelisted applications.
pub struct WindowsController {
    command_tx: mpsc::Sender<ThrottleCommand>,
    active_rules: Arc<RwLock<Vec<(u32, BandwidthLimit)>>>,
}

impl WindowsController {
    /// Instantiate a new traffic controller and launch the background processing thread.
    pub fn new() -> BwpResult<Self> {
        let (command_tx, command_rx) = mpsc::channel(256);
        let active_rules = Arc::new(RwLock::new(Vec::new()));

        let active_rules_clone = active_rules.clone();

        // Launch the specialized engine thread.
        // It's vital that Network intercepts don't run on the Tokio async loop pool.
        std::thread::spawn(move || {
            if let Err(e) = divert::run_windivert_engine(command_rx, active_rules_clone) {
                tracing::error!("WinDivert Engine Terminated: {e}");
            }
        });

        Ok(Self {
            command_tx,
            active_rules,
        })
    }
}

impl TrafficController for WindowsController {
    async fn send_command(&self, command: ThrottleCommand) -> BwpResult<()> {
        self.command_tx.send(command).await.map_err(|e| {
            BwpError::Controller(format!("Traffic engine is down, could not enqueue command: {}", e))
        })
    }

    async fn active_rules(&self) -> BwpResult<Vec<(u32, BandwidthLimit)>> {
        let lock = self.active_rules.read().await;
        Ok(lock.clone())
    }
}
