//! Maps local ports to owning PIDs, built from a connection snapshot.
//!
//! The WinDivert NETWORK layer cannot filter by process, so packets are attributed by
//! looking up their local port (chosen by packet direction) in the connection table. A port that belongs to more than
//! one PID is ambiguous and is never attributed (the packet is passed through untouched).

use crate::flow::{PacketFlow, Transport};
use bwp_core::{ConnectionInfo, Protocol};
use std::collections::HashMap;
use std::net::SocketAddr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Owner {
    Single(u32),
    Ambiguous,
}

/// `(transport, local port)` to PID lookup table.
#[derive(Debug, Default, Clone)]
pub struct PortMap {
    owners: HashMap<(Transport, u16), Owner>,
}

impl PortMap {
    /// Builds the map from a connection snapshot. Rows without an owner (PID 0) and
    /// unparsable addresses are ignored.
    #[must_use]
    pub fn from_connections(connections: &[ConnectionInfo]) -> Self {
        let mut owners: HashMap<(Transport, u16), Owner> = HashMap::new();
        for conn in connections.iter().filter(|c| c.pid != 0) {
            let Ok(addr) = conn.local_addr.parse::<SocketAddr>() else {
                continue;
            };
            let transport = match conn.protocol {
                Protocol::Tcp => Transport::Tcp,
                Protocol::Udp => Transport::Udp,
            };
            owners
                .entry((transport, addr.port()))
                .and_modify(|owner| {
                    if *owner != Owner::Single(conn.pid) {
                        *owner = Owner::Ambiguous;
                    }
                })
                .or_insert(Owner::Single(conn.pid));
        }
        Self { owners }
    }

    /// Number of distinct local ports known.
    #[must_use]
    pub fn len(&self) -> usize {
        self.owners.len()
    }

    /// Whether the map is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.owners.is_empty()
    }

    fn pid_of(&self, transport: Transport, port: u16) -> Option<u32> {
        match self.owners.get(&(transport, port))? {
            Owner::Single(pid) => Some(*pid),
            Owner::Ambiguous => None,
        }
    }

    /// Finds the process a packet belongs to.
    ///
    /// The local endpoint is the source of an outbound packet and the destination of an
    /// inbound one; only that port is looked up. A remote port that happens to equal a
    /// local listener's port (e.g. outbound HTTPS while a local service listens on 443)
    /// therefore does not hide the owner. Unknown or ambiguous ports yield `None` and the
    /// packet must not be throttled.
    #[must_use]
    pub fn pid_for(&self, flow: &PacketFlow, outbound: bool) -> Option<u32> {
        let local_port = if outbound {
            flow.src_port
        } else {
            flow.dst_port
        };
        self.pid_of(flow.transport, local_port)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bwp_core::ConnectionState;

    fn conn(pid: u32, protocol: Protocol, local: &str) -> ConnectionInfo {
        ConnectionInfo {
            pid,
            protocol,
            local_addr: local.to_string(),
            remote_addr: None,
            state: ConnectionState::Established,
        }
    }

    fn flow(transport: Transport, src_port: u16, dst_port: u16) -> PacketFlow {
        PacketFlow {
            transport,
            src_port,
            dst_port,
        }
    }

    #[test]
    fn attributes_outbound_and_inbound_packets() {
        let map = PortMap::from_connections(&[conn(10, Protocol::Tcp, "10.0.0.2:50000")]);
        assert_eq!(
            map.pid_for(&flow(Transport::Tcp, 50000, 443), true),
            Some(10)
        );
        assert_eq!(
            map.pid_for(&flow(Transport::Tcp, 443, 50000), false),
            Some(10)
        );
    }

    #[test]
    fn direction_decides_which_port_is_local() {
        let map = PortMap::from_connections(&[conn(10, Protocol::Tcp, "10.0.0.2:50000")]);
        // The local port seen on the wrong side is somebody else's traffic.
        assert_eq!(map.pid_for(&flow(Transport::Tcp, 50000, 443), false), None);
        assert_eq!(map.pid_for(&flow(Transport::Tcp, 443, 50000), true), None);
    }

    #[test]
    fn remote_port_equal_to_a_local_listener_does_not_hide_the_owner() {
        // PID 10 talks to a remote HTTPS server while PID 11 listens locally on 443.
        let map = PortMap::from_connections(&[
            conn(10, Protocol::Tcp, "10.0.0.2:50000"),
            conn(11, Protocol::Tcp, "0.0.0.0:443"),
        ]);
        assert_eq!(
            map.pid_for(&flow(Transport::Tcp, 50000, 443), true),
            Some(10)
        );
        assert_eq!(
            map.pid_for(&flow(Transport::Tcp, 443, 50000), false),
            Some(10)
        );
    }

    #[test]
    fn protocol_must_match() {
        let map = PortMap::from_connections(&[conn(10, Protocol::Tcp, "10.0.0.2:50000")]);
        assert_eq!(map.pid_for(&flow(Transport::Udp, 50000, 443), true), None);
    }

    #[test]
    fn unknown_flow_is_not_attributed() {
        let map = PortMap::from_connections(&[conn(10, Protocol::Tcp, "10.0.0.2:50000")]);
        assert_eq!(map.pid_for(&flow(Transport::Tcp, 1111, 2222), true), None);
        assert!(
            PortMap::default()
                .pid_for(&flow(Transport::Tcp, 1, 2), true)
                .is_none()
        );
    }

    #[test]
    fn shared_port_is_ambiguous() {
        let map = PortMap::from_connections(&[
            conn(10, Protocol::Udp, "0.0.0.0:5353"),
            conn(11, Protocol::Udp, "[::]:5353"),
        ]);
        assert_eq!(map.pid_for(&flow(Transport::Udp, 5353, 5353), true), None);
        // The same PID twice (IPv4 + IPv6 socket) is not ambiguous.
        let map = PortMap::from_connections(&[
            conn(10, Protocol::Udp, "0.0.0.0:5353"),
            conn(10, Protocol::Udp, "[::]:5353"),
        ]);
        assert_eq!(
            map.pid_for(&flow(Transport::Udp, 40000, 5353), false),
            Some(10)
        );
    }

    #[test]
    fn ownerless_and_bad_rows_are_ignored() {
        let map = PortMap::from_connections(&[
            conn(0, Protocol::Tcp, "10.0.0.2:1"),
            conn(5, Protocol::Tcp, "garbage"),
        ]);
        assert!(map.is_empty());
    }
}
