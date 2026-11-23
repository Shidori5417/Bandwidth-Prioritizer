//! Linux traffic control placeholder.
//!
//! **Not implemented.** Exposes the same trait interface as the Windows controller so
//! the workspace builds and the engine logic can be exercised on Linux, but no traffic
//! is shaped (the planned `tc htb` / cgroup backend does not exist yet).

use crate::traits::{ThrottleCommand, TrafficController};
use bwp_core::{BandwidthLimit, BwpResult};
use tracing::debug;

/// Placeholder Linux controller: accepts commands and ignores them.
#[derive(Debug, Default)]
pub struct LinuxController;

impl LinuxController {
    /// Creates the placeholder controller.
    ///
    /// # Errors
    /// Currently never fails.
    pub fn new() -> BwpResult<Self> {
        Ok(Self)
    }
}

impl TrafficController for LinuxController {
    async fn send_command(&self, command: ThrottleCommand) -> BwpResult<()> {
        debug!(
            ?command,
            "Linux traffic shaping not implemented; command ignored"
        );
        Ok(())
    }

    async fn active_rules(&self) -> BwpResult<Vec<(u32, BandwidthLimit)>> {
        Ok(Vec::new())
    }
}
