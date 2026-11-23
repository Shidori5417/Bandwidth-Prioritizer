//! Platform-independent helpers for reading the Windows IP Helper connection tables.
//!
//! `GetExtendedTcpTable` / `GetExtendedUdpTable` fill a caller-provided buffer with raw
//! `MIB_*` rows. The byte-order conversions, state mapping and buffer-size retry loop are
//! kept here, free of any Windows types, so they are unit tested on every platform.

use bwp_core::{BwpError, BwpResult, ConnectionInfo, ConnectionState, Protocol};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

/// `NO_ERROR`.
pub const ERROR_SUCCESS: u32 = 0;
/// `ERROR_INSUFFICIENT_BUFFER`: the buffer is too small, the required size was written back.
pub const ERROR_INSUFFICIENT_BUFFER: u32 = 122;

/// `MIB_TCP_STATE_LISTEN`.
const TCP_STATE_LISTEN: u32 = 2;
/// `MIB_TCP_STATE_ESTAB`.
const TCP_STATE_ESTABLISHED: u32 = 5;

/// Largest table accepted (64 MiB); anything bigger is treated as a bogus size.
const MAX_TABLE_BYTES: usize = 64 * 1024 * 1024;

/// Maximum number of buffer-size attempts (the table can grow between two calls).
const MAX_ATTEMPTS: usize = 10;

/// Converts a `dwLocalPort` / `dwRemotePort` field to a host-order port.
///
/// The IP Helper API stores the port in network byte order in the low 16 bits of a `DWORD`
/// (the upper 16 bits are unspecified and must be ignored).
#[must_use]
pub const fn port_from_network_order(raw: u32) -> u16 {
    // The low two bytes of the `DWORD` in memory are the big-endian port.
    u16::from_be(raw as u16)
}

/// Converts an IPv4 `DWORD` address (stored in network byte order) to an [`Ipv4Addr`].
#[must_use]
pub const fn ipv4_from_network_order(raw: u32) -> Ipv4Addr {
    // The bytes in memory are already `a.b.c.d`, whatever the host endianness.
    let b = raw.to_ne_bytes();
    Ipv4Addr::new(b[0], b[1], b[2], b[3])
}

/// Maps a `MIB_TCP_STATE` value to the simplified [`ConnectionState`].
#[must_use]
pub const fn tcp_state(raw: u32) -> ConnectionState {
    match raw {
        TCP_STATE_ESTABLISHED => ConnectionState::Established,
        TCP_STATE_LISTEN => ConnectionState::Listening,
        _ => ConnectionState::Other,
    }
}

/// Raw TCP row contents, independent of the IPv4/IPv6 `MIB_*` struct.
#[derive(Debug, Clone, Copy)]
pub struct RawTcpRow {
    /// Local address.
    pub local_ip: IpAddr,
    /// Local port, network byte order in the low 16 bits.
    pub local_port: u32,
    /// Remote address.
    pub remote_ip: IpAddr,
    /// Remote port, network byte order in the low 16 bits.
    pub remote_port: u32,
    /// `MIB_TCP_STATE`.
    pub state: u32,
    /// Owning process id.
    pub pid: u32,
}

/// Converts a TCP row into a [`ConnectionInfo`]. Listening sockets have no remote address.
#[must_use]
pub fn tcp_connection(row: &RawTcpRow) -> ConnectionInfo {
    let state = tcp_state(row.state);
    let remote_addr = (state != ConnectionState::Listening).then(|| {
        SocketAddr::new(row.remote_ip, port_from_network_order(row.remote_port)).to_string()
    });
    ConnectionInfo {
        pid: row.pid,
        protocol: Protocol::Tcp,
        local_addr: SocketAddr::new(row.local_ip, port_from_network_order(row.local_port))
            .to_string(),
        remote_addr,
        state,
    }
}

/// Converts a UDP row (local endpoint only) into a [`ConnectionInfo`].
#[must_use]
pub fn udp_connection(local_ip: IpAddr, local_port: u32, pid: u32) -> ConnectionInfo {
    ConnectionInfo {
        pid,
        protocol: Protocol::Udp,
        local_addr: SocketAddr::new(local_ip, port_from_network_order(local_port)).to_string(),
        remote_addr: None,
        state: ConnectionState::Other,
    }
}

/// Builds an IPv6 address from the 16 raw bytes of a `ucLocalAddr` / `ucRemoteAddr` field.
#[must_use]
pub fn ipv6_from_bytes(bytes: [u8; 16]) -> Ipv6Addr {
    Ipv6Addr::from(bytes)
}

/// Whether a table of `entries` rows of `row_size` bytes, starting `rows_offset` bytes into
/// the buffer, fits in the `written` bytes the API reported.
#[must_use]
pub fn rows_fit(entries: u32, row_size: usize, rows_offset: usize, written: usize) -> bool {
    (entries as usize)
        .checked_mul(row_size)
        .and_then(|n| n.checked_add(rows_offset))
        .is_some_and(|total| total <= written)
}

/// Fills a buffer using the usual two-step Win32 pattern, retrying while it is too small.
///
/// `call(buffer, size)` must invoke the API with `buffer` (or a null pointer for `None`) and
/// `size` as the in/out size in bytes, and return the Win32 error code. The buffer is
/// 8-byte aligned (`u64` elements), which satisfies every `MIB_*` table. On success the
/// buffer and the number of valid bytes are returned.
///
/// The table may grow between the size query and the real call, which yields
/// `ERROR_INSUFFICIENT_BUFFER` again: the loop retries with the newly reported size (plus
/// some slack) up to a fixed number of attempts.
///
/// # Errors
/// Returns [`BwpError::Platform`] for any other error code or when the table keeps growing.
pub fn fetch_table(
    mut call: impl FnMut(Option<&mut [u64]>, &mut u32) -> u32,
) -> BwpResult<(Vec<u64>, usize)> {
    let mut size: u32 = 0;
    let mut buffer: Vec<u64> = Vec::new();

    for _ in 0..MAX_ATTEMPTS {
        let code = if buffer.is_empty() {
            call(None, &mut size)
        } else {
            // The API takes the buffer capacity in bytes.
            size = u32::try_from(buffer.len() * 8).unwrap_or(u32::MAX);
            call(Some(&mut buffer), &mut size)
        };

        match code {
            ERROR_SUCCESS if !buffer.is_empty() => return Ok((buffer, size as usize)),
            // A successful size query with no buffer cannot happen; treat as "need buffer".
            ERROR_SUCCESS | ERROR_INSUFFICIENT_BUFFER => {
                let wanted = size as usize;
                if wanted > MAX_TABLE_BYTES {
                    return Err(BwpError::Platform(format!(
                        "IP Helper reported an implausible table size of {wanted} bytes"
                    )));
                }
                // 1/8 plus 4 KiB of slack covers connections opened in between.
                let padded = wanted.saturating_add(wanted / 8).saturating_add(4096);
                buffer = vec![0u64; padded.div_ceil(8)];
            }
            other => {
                return Err(BwpError::Platform(format!(
                    "IP Helper table query failed with error {other}"
                )));
            }
        }
    }
    Err(BwpError::Platform(
        "IP Helper table kept growing while being read".to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_is_converted_from_network_byte_order() {
        // Port 443 = 0x01BB; on the wire (and in the low bytes of the DWORD) it is 01 BB.
        let raw = u32::from_ne_bytes([0x01, 0xBB, 0, 0]);
        assert_eq!(port_from_network_order(raw), 443);
        // Garbage in the upper half must be ignored.
        let raw = u32::from_ne_bytes([0x00, 0x50, 0xAB, 0xCD]);
        assert_eq!(port_from_network_order(raw), 80);
        assert_eq!(port_from_network_order(0), 0);
    }

    #[test]
    fn ipv4_is_converted_from_network_byte_order() {
        let raw = u32::from_ne_bytes([192, 168, 1, 20]);
        assert_eq!(ipv4_from_network_order(raw), Ipv4Addr::new(192, 168, 1, 20));
    }

    #[test]
    fn tcp_states_are_mapped() {
        assert_eq!(tcp_state(5), ConnectionState::Established);
        assert_eq!(tcp_state(2), ConnectionState::Listening);
        assert_eq!(tcp_state(11), ConnectionState::Other);
        assert_eq!(tcp_state(1), ConnectionState::Other);
    }

    #[test]
    fn tcp_row_becomes_connection_info() {
        let row = RawTcpRow {
            local_ip: IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2)),
            local_port: u32::from_ne_bytes([0xC3, 0x50, 0, 0]), // 50000
            remote_ip: IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)),
            remote_port: u32::from_ne_bytes([0x01, 0xBB, 0, 0]),
            state: 5,
            pid: 4242,
        };
        let conn = tcp_connection(&row);
        assert_eq!(conn.pid, 4242);
        assert_eq!(conn.protocol, Protocol::Tcp);
        assert_eq!(conn.local_addr, "10.0.0.2:50000");
        assert_eq!(conn.remote_addr.as_deref(), Some("1.1.1.1:443"));
        assert_eq!(conn.state, ConnectionState::Established);
    }

    #[test]
    fn listening_row_has_no_remote_and_ipv6_is_bracketed() {
        let row = RawTcpRow {
            local_ip: IpAddr::V6(ipv6_from_bytes([0; 16])),
            local_port: u32::from_ne_bytes([0x00, 0x50, 0, 0]),
            remote_ip: IpAddr::V6(ipv6_from_bytes([0; 16])),
            remote_port: 0,
            state: 2,
            pid: 4,
        };
        let conn = tcp_connection(&row);
        assert_eq!(conn.local_addr, "[::]:80");
        assert_eq!(conn.remote_addr, None);
        assert_eq!(conn.state, ConnectionState::Listening);
    }

    #[test]
    fn udp_row_becomes_connection_info() {
        let conn = udp_connection(
            IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            u32::from_ne_bytes([0x14, 0xE9, 0, 0]), // 5353
            77,
        );
        assert_eq!(conn.local_addr, "0.0.0.0:5353");
        assert_eq!(conn.protocol, Protocol::Udp);
        assert_eq!(conn.remote_addr, None);
    }

    #[test]
    fn rows_fit_checks_bounds_and_overflow() {
        assert!(rows_fit(0, 24, 4, 4));
        assert!(rows_fit(2, 24, 4, 52));
        assert!(!rows_fit(2, 24, 4, 51));
        assert!(!rows_fit(u32::MAX, usize::MAX, 4, 100));
    }

    /// Fake API whose required size is `needs` bytes; it returns data only when the offered
    /// size is large enough, like the real functions.
    fn fake_api(needs: &mut Vec<u32>) -> impl FnMut(Option<&mut [u64]>, &mut u32) -> u32 + '_ {
        move |buffer, size| {
            let required = if needs.len() > 1 {
                needs.remove(0)
            } else {
                needs[0]
            };
            match buffer {
                Some(buf) if (buf.len() * 8) as u32 >= required => {
                    buf[0] = 0xDEAD;
                    *size = required;
                    ERROR_SUCCESS
                }
                _ => {
                    *size = required;
                    ERROR_INSUFFICIENT_BUFFER
                }
            }
        }
    }

    #[test]
    fn fetch_table_resizes_after_size_query() {
        let mut needs = vec![1000];
        let (buf, written) = fetch_table(fake_api(&mut needs)).unwrap();
        assert_eq!(written, 1000);
        assert_eq!(buf[0], 0xDEAD);
        assert!(buf.len() * 8 >= 1000);
    }

    #[test]
    fn fetch_table_retries_when_table_grows_in_between() {
        // The size query says 1000, but by the real call 20 000 bytes are needed.
        let mut needs = vec![1000, 20_000, 20_000];
        let (_, written) = fetch_table(fake_api(&mut needs)).unwrap();
        assert_eq!(written, 20_000);
    }

    #[test]
    fn fetch_table_gives_up_when_table_keeps_growing() {
        let mut size = 1000u32;
        let result = fetch_table(|_, out| {
            size += 1000;
            *out = size;
            ERROR_INSUFFICIENT_BUFFER
        });
        assert!(result.is_err());
    }

    #[test]
    fn fetch_table_rejects_absurd_sizes() {
        assert!(
            fetch_table(|_, out| {
                *out = u32::MAX;
                ERROR_INSUFFICIENT_BUFFER
            })
            .is_err()
        );
    }

    #[test]
    fn fetch_table_reports_other_errors() {
        let err = fetch_table(|_, _| 87).unwrap_err();
        assert!(err.to_string().contains("87"), "{err}");
    }
}
