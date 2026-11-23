use crate::traits::{ThrottleCommand, TrafficController};
use bwp_core::{BandwidthLimit, BwpError, BwpResult};
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};

/// Placeholder packet-shaping engine.
pub mod divert;

/// Windows implementation of the Traffic Controller.
///
/// **Placeholder:** commands are accepted and queued, but the WinDivert packet
/// shaping behind them is not implemented yet, so no traffic is limited.
pub struct WindowsController {
    command_tx: mpsc::Sender<ThrottleCommand>,
    active_rules: Arc<RwLock<Vec<(u32, BandwidthLimit)>>>,
}

impl WindowsController {
    /// Instantiate a new traffic controller and launch the background processing thread.
    ///
    /// # Errors
    /// Currently never fails.
    pub fn new() -> BwpResult<Self> {
        let (command_tx, command_rx) = mpsc::channel(256);
        let active_rules = Arc::new(RwLock::new(Vec::new()));

        let active_rules_clone = active_rules.clone();

        // Launch the specialized engine thread.
        // It's vital that Network intercepts don't run on the Tokio async loop pool.
        std::thread::spawn(move || {
            if let Err(e) = divert::run_placeholder_engine(command_rx, &active_rules_clone) {
                tracing::error!("Shaping engine terminated: {e}");
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
            BwpError::Controller(format!(
                "Traffic engine is down, could not enqueue command: {}",
                e
            ))
        })
    }

    async fn active_rules(&self) -> BwpResult<Vec<(u32, BandwidthLimit)>> {
        let lock = self.active_rules.read().await;
        Ok(lock.clone())
    }
}
