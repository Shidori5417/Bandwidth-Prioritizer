//! The default (no-op) packet-shaping engine.
//!
//! It never opens a WinDivert handle, so no packet is ever captured: capturing without
//! re-injecting would black-hole the traffic of every throttled process. It only drains the
//! command queue (blocking, so no CPU is spent) and logs a clear warning.
//!
//! The opt-in experimental shaper lives in `windows::windivert` and is only compiled with
//! the `experimental-windivert` feature.

use bwp_core::{BandwidthLimit, BwpResult};
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};
use tracing::{debug, warn};

use crate::traits::ThrottleCommand;

/// Runs the (no-op) shaping engine on the current thread until the command
/// channel is closed.
///
/// No traffic is intercepted or limited. the active rules list stays empty because no rule is
/// actually being enforced.
pub fn run_placeholder_engine(
    mut command_rx: mpsc::Receiver<ThrottleCommand>,
    _active_rules: &Arc<RwLock<Vec<(u32, BandwidthLimit)>>>,
) -> BwpResult<()> {
    warn!(
        "Packet shaping is disabled (default): throttle commands are accepted but \
         no traffic is intercepted or limited. See the README to opt into the \
         experimental WinDivert shaper."
    );

    // `blocking_recv` parks the thread until a command arrives: no busy loop.
    while let Some(command) = command_rx.blocking_recv() {
        debug!(
            ?command,
            "Ignoring throttle command (packet shaping disabled)"
        );
    }
    Ok(())
}
