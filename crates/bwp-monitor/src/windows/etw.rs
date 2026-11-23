//! Event Tracing for Windows (ETW) integration (**not implemented**).
//!
//! The intended design is a real-time `Microsoft-Windows-TCPIP` trace session (via
//! `ferrisetw`) that parses connect/close events into [`bwp_core::MonitorEvent`]s.
//!
//! An earlier skeleton lived here, but it never parsed addresses or the owning
//! process and would have emitted bogus events (`0.0.0.0:0`, tagged with the PID of
//! the ETW worker rather than of the socket owner), so it was removed. Until the real
//! parser exists, starting the listener reports an error so the monitor falls back to
//! the polling path.

use bwp_core::{BwpError, BwpResult, MonitorEvent};
use tokio::sync::mpsc;

/// Attempts to start the ETW trace session.
///
/// # Errors
/// Always returns [`BwpError::Platform`]: ETW monitoring is not implemented yet.
pub fn spawn_etw_listener(_tx: mpsc::Sender<MonitorEvent>) -> BwpResult<()> {
    Err(BwpError::Platform(
        "ETW connection monitoring is not implemented yet".to_string(),
    ))
}
