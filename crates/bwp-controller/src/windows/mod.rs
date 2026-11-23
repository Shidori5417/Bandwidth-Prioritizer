use crate::traits::{ThrottleCommand, TrafficController};
use bwp_core::{BandwidthLimit, BwpError, BwpResult};
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};

/// Placeholder packet-shaping engine (the default).
pub mod divert;

/// Experimental, opt-in WinDivert packet shaper.
#[cfg(feature = "experimental-windivert")]
pub mod windivert;

/// Windows implementation of the Traffic Controller.
///
/// By default this is a **no-op**: commands are accepted and ignored and no packet is
/// ever captured. The experimental WinDivert shaper only runs when the crate is built with
/// the `experimental-windivert` feature *and* [`with_shaping`](Self::with_shaping) is
/// called with `true`.
pub struct WindowsController {
    command_tx: mpsc::Sender<ThrottleCommand>,
    active_rules: Arc<RwLock<Vec<(u32, BandwidthLimit)>>>,
}

impl WindowsController {
    /// Instantiate a controller that does not shape traffic (the safe default).
    ///
    /// # Errors
    /// Currently never fails.
    pub fn new() -> BwpResult<Self> {
        Self::with_shaping(false)
    }

    /// Instantiate a new traffic controller and launch the background processing thread.
    ///
    /// With `experimental_windivert == true` and a build with the `experimental-windivert`
    /// feature, throttle commands are enforced by the experimental WinDivert shaper (see
    /// [`windivert`]). Without the feature the flag is ignored with a warning.
    ///
    /// # Errors
    /// Currently never fails.
    pub fn with_shaping(experimental_windivert: bool) -> BwpResult<Self> {
        let (command_tx, command_rx) = mpsc::channel(256);
        let active_rules = Arc::new(RwLock::new(Vec::new()));

        let active_rules_clone = active_rules.clone();

        // Launch the specialized engine thread.
        // It's vital that Network intercepts don't run on the Tokio async loop pool.
        std::thread::spawn(move || {
            if let Err(e) = run_engine(experimental_windivert, command_rx, &active_rules_clone) {
                tracing::error!("Shaping engine terminated: {e}");
            }
        });

        Ok(Self {
            command_tx,
            active_rules,
        })
    }
}

type ActiveRules = Arc<RwLock<Vec<(u32, BandwidthLimit)>>>;

fn run_engine(
    experimental_windivert: bool,
    command_rx: mpsc::Receiver<ThrottleCommand>,
    active_rules: &ActiveRules,
) -> BwpResult<()> {
    if experimental_windivert {
        #[cfg(feature = "experimental-windivert")]
        return windivert::run_shaping_engine(command_rx, active_rules);
        #[cfg(not(feature = "experimental-windivert"))]
        tracing::warn!(
            "[shaping] experimental_windivert is set, but this build does not include the \
             `experimental-windivert` feature: packet shaping stays disabled."
        );
    }
    divert::run_placeholder_engine(command_rx, active_rules)
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
