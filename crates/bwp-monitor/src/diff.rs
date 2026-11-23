//! Platform-independent connection snapshot diffing.
//!
//! The Windows poller takes periodic snapshots of the TCP/UDP tables. This module holds
//! the pure logic that turns two successive snapshots into
//! [`MonitorEvent::ConnectionOpened`] / [`MonitorEvent::ConnectionClosed`] events, so it can
//! be unit tested on any platform.
//!
//! Every connection that appears in a snapshot produces exactly one `Opened` event and,
//! once it disappears, exactly one `Closed` event. The engine counts connections per PID,
//! so the two must stay balanced.

use bwp_core::{ConnectionInfo, ConnectionState, MonitorEvent, ProcessInfo, Protocol};
use std::collections::{BTreeMap, HashMap};
use std::net::SocketAddr;

/// Identity of a connection across snapshots.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ConnKey {
    /// Owning process.
    pub pid: u32,
    /// `true` for TCP, `false` for UDP.
    pub tcp: bool,
    /// Local `ip:port`.
    pub local: String,
    /// Remote `ip:port`, if any.
    pub remote: Option<String>,
}

impl ConnKey {
    /// Builds the key of a connection.
    #[must_use]
    pub fn of(conn: &ConnectionInfo) -> Self {
        Self {
            pid: conn.pid,
            tcp: conn.protocol == Protocol::Tcp,
            local: conn.local_addr.clone(),
            remote: conn.remote_addr.clone(),
        }
    }
}

fn is_loopback(addr: &str) -> bool {
    addr.parse::<SocketAddr>()
        .is_ok_and(|a| a.ip().is_loopback())
}

/// Whether a connection should generate events.
///
/// Only connections that can carry traffic to or from the network count: established TCP
/// connections and UDP endpoints. Listening and half-closed TCP sockets, rows without an
/// owner (PID 0) and loopback-only sockets are ignored.
#[must_use]
pub fn is_trackable(conn: &ConnectionInfo) -> bool {
    if conn.pid == 0 {
        return false;
    }
    match conn.protocol {
        Protocol::Tcp => {
            conn.state == ConnectionState::Established
                && !conn.remote_addr.as_deref().is_some_and(is_loopback)
        }
        Protocol::Udp => !is_loopback(&conn.local_addr),
    }
}

/// Remembers the previous snapshot and diffs it against the next one.
#[derive(Debug, Default)]
pub struct ConnectionTracker {
    known: HashMap<ConnKey, ConnectionInfo>,
}

impl ConnectionTracker {
    /// Creates an empty tracker (the first snapshot reports everything as opened).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of connections currently tracked.
    #[must_use]
    pub fn len(&self) -> usize {
        self.known.len()
    }

    /// Whether nothing is tracked.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.known.is_empty()
    }

    /// Feeds a new snapshot and returns the events since the previous one.
    ///
    /// `Opened` events come first, in snapshot order; `Closed` events follow, ordered by key.
    /// Duplicate rows in one snapshot are collapsed.
    pub fn update(&mut self, snapshot: &[ConnectionInfo]) -> Vec<MonitorEvent> {
        let mut current: HashMap<ConnKey, &ConnectionInfo> = HashMap::new();
        let mut order = Vec::new();
        for conn in snapshot.iter().filter(|c| is_trackable(c)) {
            let key = ConnKey::of(conn);
            if current.insert(key.clone(), conn).is_none() {
                order.push(key);
            }
        }

        let mut events = Vec::new();
        for key in &order {
            if !self.known.contains_key(key) {
                if let Some(conn) = current.get(key) {
                    events.push(MonitorEvent::ConnectionOpened((*conn).clone()));
                }
            }
        }

        let closed: BTreeMap<&ConnKey, &ConnectionInfo> = self
            .known
            .iter()
            .filter(|(key, _)| !current.contains_key(*key))
            .collect();
        events.extend(
            closed
                .into_values()
                .map(|conn| MonitorEvent::ConnectionClosed(conn.clone())),
        );

        self.known = current
            .into_iter()
            .map(|(key, conn)| (key, conn.clone()))
            .collect();
        events
    }
}

/// Groups connections by PID into process records, using `resolve` to look up the process
/// name and executable path. PIDs `resolve` does not know are skipped, and the result is
/// sorted by PID.
pub fn processes_from_connections(
    connections: &[ConnectionInfo],
    mut resolve: impl FnMut(u32) -> Option<(String, Option<String>)>,
) -> Vec<ProcessInfo> {
    let mut pids: Vec<u32> = connections
        .iter()
        .filter(|c| c.pid != 0)
        .map(|c| c.pid)
        .collect();
    pids.sort_unstable();
    pids.dedup();
    pids.into_iter()
        .filter_map(|pid| {
            resolve(pid).map(|(name, exe_path)| ProcessInfo {
                pid,
                name,
                exe_path,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tcp(pid: u32, local: &str, remote: &str) -> ConnectionInfo {
        ConnectionInfo {
            pid,
            protocol: Protocol::Tcp,
            local_addr: local.to_string(),
            remote_addr: Some(remote.to_string()),
            state: ConnectionState::Established,
        }
    }

    fn udp(pid: u32, local: &str) -> ConnectionInfo {
        ConnectionInfo {
            pid,
            protocol: Protocol::Udp,
            local_addr: local.to_string(),
            remote_addr: None,
            state: ConnectionState::Other,
        }
    }

    fn opened_pids(events: &[MonitorEvent]) -> Vec<u32> {
        events
            .iter()
            .filter_map(|e| match e {
                MonitorEvent::ConnectionOpened(c) => Some(c.pid),
                MonitorEvent::ConnectionClosed(_) => None,
            })
            .collect()
    }

    fn closed_pids(events: &[MonitorEvent]) -> Vec<u32> {
        events
            .iter()
            .filter_map(|e| match e {
                MonitorEvent::ConnectionClosed(c) => Some(c.pid),
                MonitorEvent::ConnectionOpened(_) => None,
            })
            .collect()
    }

    #[test]
    fn first_snapshot_opens_everything() {
        let mut tracker = ConnectionTracker::new();
        let events = tracker.update(&[
            tcp(10, "192.168.1.5:50000", "1.1.1.1:443"),
            udp(20, "0.0.0.0:5353"),
        ]);
        assert_eq!(opened_pids(&events), [10, 20]);
        assert!(closed_pids(&events).is_empty());
        assert_eq!(tracker.len(), 2);
    }

    #[test]
    fn unchanged_snapshot_produces_no_events() {
        let mut tracker = ConnectionTracker::new();
        let snap = [tcp(10, "192.168.1.5:50000", "1.1.1.1:443")];
        let _ = tracker.update(&snap);
        assert!(tracker.update(&snap).is_empty());
    }

    #[test]
    fn disappearing_connection_is_reported_closed_once() {
        let mut tracker = ConnectionTracker::new();
        let a = tcp(10, "192.168.1.5:50000", "1.1.1.1:443");
        let b = tcp(10, "192.168.1.5:50001", "1.1.1.1:443");
        let _ = tracker.update(&[a.clone(), b.clone()]);

        let events = tracker.update(std::slice::from_ref(&a));
        assert_eq!(closed_pids(&events), [10]);
        assert!(opened_pids(&events).is_empty());
        // Gone for good: no repeated Closed.
        assert!(tracker.update(&[a]).is_empty());
        assert_eq!(tracker.len(), 1);
    }

    #[test]
    fn two_connections_of_one_pid_are_distinct() {
        let mut tracker = ConnectionTracker::new();
        let events = tracker.update(&[
            tcp(10, "10.0.0.2:1000", "8.8.8.8:443"),
            tcp(10, "10.0.0.2:1001", "8.8.8.8:443"),
        ]);
        assert_eq!(opened_pids(&events), [10, 10]);
    }

    #[test]
    fn duplicate_rows_are_collapsed() {
        let mut tracker = ConnectionTracker::new();
        let row = udp(7, "0.0.0.0:3478");
        let events = tracker.update(&[row.clone(), row.clone()]);
        assert_eq!(opened_pids(&events), [7]);
        let events = tracker.update(&[]);
        assert_eq!(closed_pids(&events), [7]);
    }

    #[test]
    fn same_local_endpoint_reused_by_another_pid_is_a_new_connection() {
        let mut tracker = ConnectionTracker::new();
        let _ = tracker.update(&[udp(7, "0.0.0.0:5000")]);
        let events = tracker.update(&[udp(8, "0.0.0.0:5000")]);
        assert_eq!(opened_pids(&events), [8]);
        assert_eq!(closed_pids(&events), [7]);
    }

    #[test]
    fn reconnect_on_a_new_port_closes_old_and_opens_new() {
        let mut tracker = ConnectionTracker::new();
        let _ = tracker.update(&[tcp(10, "10.0.0.2:1000", "8.8.8.8:443")]);
        let events = tracker.update(&[tcp(10, "10.0.0.2:1002", "8.8.8.8:443")]);
        assert_eq!(opened_pids(&events), [10]);
        assert_eq!(closed_pids(&events), [10]);
    }

    #[test]
    fn non_active_and_loopback_rows_are_ignored() {
        let mut tracker = ConnectionTracker::new();
        let mut listening = tcp(10, "0.0.0.0:80", "0.0.0.0:0");
        listening.state = ConnectionState::Listening;
        let mut closing = tcp(10, "10.0.0.2:1000", "8.8.8.8:443");
        closing.state = ConnectionState::Other;
        let events = tracker.update(&[
            listening,
            closing,
            tcp(0, "10.0.0.2:2000", "8.8.8.8:443"),
            tcp(11, "127.0.0.1:3000", "127.0.0.1:4000"),
            tcp(11, "[::1]:3000", "[::1]:4000"),
            udp(12, "127.0.0.1:5000"),
        ]);
        assert!(events.is_empty());
        assert!(tracker.is_empty());
    }

    #[test]
    fn established_to_closing_counts_as_closed() {
        let mut tracker = ConnectionTracker::new();
        let est = tcp(10, "10.0.0.2:1000", "8.8.8.8:443");
        let _ = tracker.update(std::slice::from_ref(&est));
        let mut fin = est;
        fin.state = ConnectionState::Other;
        let events = tracker.update(&[fin]);
        assert_eq!(closed_pids(&events), [10]);
    }

    #[test]
    fn ipv6_connections_are_tracked() {
        let mut tracker = ConnectionTracker::new();
        let events = tracker.update(&[tcp(10, "[2001:db8::2]:5000", "[2001:db8::1]:443")]);
        assert_eq!(opened_pids(&events), [10]);
    }

    #[test]
    fn processes_are_grouped_by_pid() {
        let conns = [
            tcp(10, "10.0.0.2:1000", "8.8.8.8:443"),
            tcp(10, "10.0.0.2:1001", "8.8.8.8:443"),
            udp(5, "0.0.0.0:53"),
            udp(0, "0.0.0.0:54"),
            udp(99, "0.0.0.0:55"),
        ];
        let procs = processes_from_connections(&conns, |pid| {
            (pid != 99).then(|| (format!("p{pid}.exe"), None))
        });
        let pids: Vec<u32> = procs.iter().map(|p| p.pid).collect();
        assert_eq!(pids, [5, 10]);
        assert_eq!(procs[1].name, "p10.exe");
    }
}
