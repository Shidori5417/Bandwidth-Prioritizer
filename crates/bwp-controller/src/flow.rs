//! Minimal IP/TCP/UDP header parsing used to attribute captured packets to flows.
//!
//! Only what the packet shaper needs is extracted: transport protocol and the two ports.
//! Anything unusual (any IPv4 fragment, IPv6 extension headers, other protocols, truncated
//! packets) yields `None`, which the shaper treats as "not ours, pass through untouched".

/// Transport protocol of a parsed packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Transport {
    /// TCP.
    Tcp,
    /// UDP.
    Udp,
}

/// The facts about a packet needed to find its owning process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PacketFlow {
    /// Transport protocol.
    pub transport: Transport,
    /// Source port (host order).
    pub src_port: u16,
    /// Destination port (host order).
    pub dst_port: u16,
}

const PROTO_TCP: u8 = 6;
const PROTO_UDP: u8 = 17;

/// Parses an IPv4 or IPv6 packet as delivered by the WinDivert NETWORK layer.
///
/// Returns `None` for IPv4 fragments (first and later ones), IPv6 packets with extension headers, protocols
/// other than TCP/UDP and packets too short to contain the ports.
#[must_use]
pub fn parse_flow(packet: &[u8]) -> Option<PacketFlow> {
    let version = packet.first()? >> 4;
    let (protocol, l4_offset) = match version {
        4 => {
            let ihl = usize::from(packet[0] & 0x0F) * 4;
            if ihl < 20 || packet.len() < ihl {
                return None;
            }
            // Flags/fragment offset: every fragment (offset != 0, or More Fragments set,
            // which also covers the first one) bypasses the shaper so that a datagram is
            // never split between shaped and unshaped fragments.
            let flags_and_offset = u16::from_be_bytes([*packet.get(6)?, *packet.get(7)?]);
            if flags_and_offset & (0x2000 | 0x1FFF) != 0 {
                return None;
            }
            (*packet.get(9)?, ihl)
        }
        6 => (*packet.get(6)?, 40),
        _ => return None,
    };

    let transport = match protocol {
        PROTO_TCP => Transport::Tcp,
        PROTO_UDP => Transport::Udp,
        _ => return None,
    };
    let ports = packet.get(l4_offset..l4_offset + 4)?;
    Some(PacketFlow {
        transport,
        src_port: u16::from_be_bytes([ports[0], ports[1]]),
        dst_port: u16::from_be_bytes([ports[2], ports[3]]),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ipv4(protocol: u8, ihl_words: u8, frag: u16, sport: u16, dport: u16) -> Vec<u8> {
        let ihl = usize::from(ihl_words) * 4;
        let mut p = vec![0u8; ihl + 8];
        p[0] = 0x40 | ihl_words;
        p[6..8].copy_from_slice(&frag.to_be_bytes());
        p[9] = protocol;
        p[ihl..ihl + 2].copy_from_slice(&sport.to_be_bytes());
        p[ihl + 2..ihl + 4].copy_from_slice(&dport.to_be_bytes());
        p
    }

    fn ipv6(next_header: u8, sport: u16, dport: u16) -> Vec<u8> {
        let mut p = vec![0u8; 48];
        p[0] = 0x60;
        p[6] = next_header;
        p[40..42].copy_from_slice(&sport.to_be_bytes());
        p[42..44].copy_from_slice(&dport.to_be_bytes());
        p
    }

    #[test]
    fn parses_ipv4_tcp_and_udp() {
        let flow = parse_flow(&ipv4(6, 5, 0, 50000, 443)).unwrap();
        assert_eq!(
            flow,
            PacketFlow {
                transport: Transport::Tcp,
                src_port: 50000,
                dst_port: 443
            }
        );
        let flow = parse_flow(&ipv4(17, 5, 0x4000, 53, 40000)).unwrap(); // DF set
        assert_eq!(flow.transport, Transport::Udp);
        assert_eq!((flow.src_port, flow.dst_port), (53, 40000));
    }

    #[test]
    fn honours_ipv4_options() {
        let flow = parse_flow(&ipv4(6, 7, 0, 1234, 80)).unwrap();
        assert_eq!((flow.src_port, flow.dst_port), (1234, 80));
    }

    #[test]
    fn parses_ipv6() {
        let flow = parse_flow(&ipv6(17, 5000, 3478)).unwrap();
        assert_eq!(flow.transport, Transport::Udp);
        assert_eq!((flow.src_port, flow.dst_port), (5000, 3478));
    }

    #[test]
    fn rejects_everything_unusual() {
        // Non-first fragment, and first fragment (More Fragments set, offset 0).
        assert!(parse_flow(&ipv4(6, 5, 0x00B9, 1, 2)).is_none());
        assert!(parse_flow(&ipv4(6, 5, 0x2000, 1, 2)).is_none());
        // ICMP, IPv6 extension header (hop-by-hop = 0), ICMPv6.
        assert!(parse_flow(&ipv4(1, 5, 0, 1, 2)).is_none());
        assert!(parse_flow(&ipv6(0, 1, 2)).is_none());
        assert!(parse_flow(&ipv6(58, 1, 2)).is_none());
        // Truncated and garbage.
        assert!(parse_flow(&[]).is_none());
        assert!(parse_flow(&[0x45, 0, 0]).is_none());
        assert!(parse_flow(&ipv4(6, 5, 0, 1, 2)[..22]).is_none());
        assert!(parse_flow(&[0x45; 8]).is_none());
        assert!(parse_flow(&[0x10; 64]).is_none());
        // IHL smaller than the minimum header.
        let mut bad = ipv4(6, 5, 0, 1, 2);
        bad[0] = 0x44;
        assert!(parse_flow(&bad).is_none());
    }
}
