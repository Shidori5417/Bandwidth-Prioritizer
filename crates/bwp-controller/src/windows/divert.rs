//! Placeholder for the WinDivert packet-shaping engine.
//!
//! **Status: not implemented.** Packet capture, token-bucket shaping and re-injection
//! are not written yet. Capturing packets without re-injecting them would black-hole
//! the traffic of every throttled process, so this engine deliberately never opens a
//! WinDivert handle. It only drains the command queue (blocking, so no CPU is spent)
//! and logs a clear warning.
//!
//! Design notes for the real implementation:
//! - The WinDivert `NETWORK` layer has no `pid` filter field. Mapping packets to
//!   processes needs the `SOCKET` layer (flow events carry the process id), keyed by
//!   the connection 5-tuple, combined with a `NETWORK` layer handle that is filtered
//!   on those tuples.
//! - Every captured packet has to be either re-injected (`send`) or intentionally
//!   dropped, after a [`crate::token_bucket::TokenBucket`] decision.

use bwp_core::{BandwidthLimit, BwpResult};
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};
use tracing::{debug, warn};

use crate::traits::ThrottleCommand;

/// Runs the (placeholder) shaping engine on the current thread until the command
/// channel is closed.
///
/// No traffic is intercepted or limited. the active rules list stays empty because no rule is
/// actually being enforced.
pub fn run_placeholder_engine(
    mut command_rx: mpsc::Receiver<ThrottleCommand>,
    _active_rules: &Arc<RwLock<Vec<(u32, BandwidthLimit)>>>,
) -> BwpResult<()> {
    warn!(
        "Packet shaping is not implemented yet: throttle commands are accepted but \
         no traffic is intercepted or limited."
    );

    // `blocking_recv` parks the thread until a command arrives: no busy loop.
    while let Some(command) = command_rx.blocking_recv() {
        debug!(
            ?command,
            "Ignoring throttle command (shaping not implemented)"
        );
    }
    Ok(())
}
