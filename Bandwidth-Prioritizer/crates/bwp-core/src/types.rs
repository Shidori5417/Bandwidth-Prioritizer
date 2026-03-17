//! Core domain types used across the entire workspace.
//!
//! These types form the shared vocabulary between the monitor,
//! controller, and CLI crates.

use serde::{Deserialize, Serialize};
use std::fmt;

// ─── Process Information ────────────────────────────────────────────

/// Metadata about a running system process.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct ProcessInfo {
    /// Operating-system process identifier.
    pub pid: u32,
    /// Human-readable process name (e.g. `discord.exe`).
    pub name: String,
    /// Full path to the executable, if available.
    pub exe_path: Option<String>,
}

impl fmt::Display for ProcessInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[PID {}] {}", self.pid, self.name)
    }
}

// ─── Network Connection ─────────────────────────────────────────────

/// Represents an active network connection tied to a process.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionInfo {
    /// Owning process identifier.
    pub pid: u32,
    /// Transport protocol.
    pub protocol: Protocol,
    /// Local address and port.
    pub local_addr: String,
    /// Remote address and port (if connected).
    pub remote_addr: Option<String>,
    /// Connection state (primarily for TCP).
    pub state: ConnectionState,
}

/// Transport layer protocol.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Protocol {
    /// Transmission Control Protocol.
    Tcp,
    /// User Datagram Protocol.
    Udp,
}

impl fmt::Display for Protocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tcp => write!(f, "TCP"),
            Self::Udp => write!(f, "UDP"),
        }
    }
}

/// TCP connection state (simplified).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ConnectionState {
    /// Connection is established and active.
    Established,
    /// Listening for incoming connections.
    Listening,
    /// Connection is in a transient state.
    Other,
}

// ─── Monitoring Events (Stream Architecture) ─────────────────────────

/// A real-time event representing network activity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MonitorEvent {
    /// A new connection was established (e.g. TCP connect / UDP packet out).
    ConnectionOpened(ConnectionInfo),
    /// An existing connection was torn down.
    ConnectionClosed(ConnectionInfo),
}

// ─── Bandwidth / Throttle ───────────────────────────────────────────

/// A bandwidth limit expressed in **bytes per second**.
///
/// Internally stores the value in bytes for precision.
/// Helper constructors allow creation from Kbps / Mbps.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct BandwidthLimit {
    /// Bytes per second.
    bytes_per_sec: u64,
}

impl BandwidthLimit {
    /// Create a limit from raw bytes per second.
    #[must_use]
    pub const fn from_bytes(bytes_per_sec: u64) -> Self {
        Self { bytes_per_sec }
    }

    /// Create a limit from **kilobits** per second.
    #[must_use]
    pub const fn from_kbps(kbps: u64) -> Self {
        Self {
            bytes_per_sec: kbps * 1_000 / 8,
        }
    }

    /// Create a limit from **megabits** per second.
    #[must_use]
    pub const fn from_mbps(mbps: u64) -> Self {
        Self {
            bytes_per_sec: mbps * 1_000_000 / 8,
        }
    }

    /// Returns the limit in bytes per second.
    #[must_use]
    pub const fn as_bytes_per_sec(&self) -> u64 {
        self.bytes_per_sec
    }

    /// Returns the limit in kilobits per second.
    #[must_use]
    pub const fn as_kbps(&self) -> u64 {
        self.bytes_per_sec * 8 / 1_000
    }

    /// Returns the limit in megabits per second.
    #[must_use]
    pub const fn as_mbps(&self) -> u64 {
        self.bytes_per_sec * 8 / 1_000_000
    }
}

impl fmt::Display for BandwidthLimit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mbps = self.as_mbps();
        if mbps > 0 {
            write!(f, "{mbps} Mbps")
        } else {
            write!(f, "{} Kbps", self.as_kbps())
        }
    }
}

/// The current throttle state of the application.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ThrottleState {
    /// No throttling active — all processes run at full bandwidth.
    Normal,
    /// A critical application is active; non-critical processes are throttled.
    PriorityMode,
    /// Transitioning between states (gradual ramp-up / ramp-down).
    Transitioning,
}

impl fmt::Display for ThrottleState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Normal => write!(f, "Normal"),
            Self::PriorityMode => write!(f, "Priority Mode"),
            Self::Transitioning => write!(f, "Transitioning…"),
        }
    }
}

// ─── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bandwidth_limit_conversions() {
        let limit = BandwidthLimit::from_mbps(100);
        assert_eq!(limit.as_bytes_per_sec(), 12_500_000);
        assert_eq!(limit.as_kbps(), 100_000);
        assert_eq!(limit.as_mbps(), 100);
    }

    #[test]
    fn bandwidth_limit_from_kbps() {
        let limit = BandwidthLimit::from_kbps(500);
        assert_eq!(limit.as_bytes_per_sec(), 62_500);
        assert_eq!(limit.as_kbps(), 500);
    }

    #[test]
    fn bandwidth_limit_display() {
        assert_eq!(BandwidthLimit::from_mbps(10).to_string(), "10 Mbps");
        assert_eq!(BandwidthLimit::from_kbps(500).to_string(), "500 Kbps");
    }

    #[test]
    fn process_info_display() {
        let proc = ProcessInfo {
            pid: 1234,
            name: "discord.exe".to_string(),
            exe_path: None,
        };
        assert_eq!(proc.to_string(), "[PID 1234] discord.exe");
    }

    #[test]
    fn throttle_state_display() {
        assert_eq!(ThrottleState::Normal.to_string(), "Normal");
        assert_eq!(ThrottleState::PriorityMode.to_string(), "Priority Mode");
    }
}
