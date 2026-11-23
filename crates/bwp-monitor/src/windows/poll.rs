//! Polling connection monitor built on the IP Helper API.
//!
//! Every poll interval (500 ms by default, `[general] poll_interval_ms`) the TCP (IPv4 and IPv6) and UDP tables are read with
//! `GetExtendedTcpTable` / `GetExtendedUdpTable`, and the difference to the previous
//! snapshot is turned into [`MonitorEvent`]s by [`ConnectionTracker`].
//!
//! This is the only place with `unsafe` code in the monitor: the FFI calls and the
//! reinterpretation of the returned buffer as `MIB_*` rows.
#![allow(unsafe_code)]

use std::ffi::c_void;
use std::mem::{offset_of, size_of};
use std::net::IpAddr;

use bwp_core::{BwpError, BwpResult, ConnectionInfo, MonitorEvent, ProcessInfo};
use tokio::sync::mpsc;
use tokio::time::{Duration, MissedTickBehavior, interval};
use tracing::{debug, error, info};
use windows::Win32::NetworkManagement::IpHelper::{
    GetExtendedTcpTable, GetExtendedUdpTable, MIB_TCP6ROW_OWNER_PID, MIB_TCP6TABLE_OWNER_PID,
    MIB_TCPROW_OWNER_PID, MIB_TCPTABLE_OWNER_PID, MIB_UDP6ROW_OWNER_PID, MIB_UDP6TABLE_OWNER_PID,
    MIB_UDPROW_OWNER_PID, MIB_UDPTABLE_OWNER_PID, TCP_TABLE_OWNER_PID_ALL, UDP_TABLE_OWNER_PID,
};
use windows::Win32::Networking::WinSock::{AF_INET, AF_INET6};

use crate::diff::{ConnectionTracker, processes_from_connections};
use crate::table::{
    RawTcpRow, fetch_table, ipv4_from_network_order, ipv6_from_bytes, rows_fit, tcp_connection,
    udp_connection,
};

/// Default delay between two snapshots.
pub const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Spawns the asynchronous polling loop.
pub fn spawn_polling_loop(tx: mpsc::Sender<MonitorEvent>, poll_interval: Duration) {
    drop(tokio::spawn(async move {
        info!("Connection polling started (GetExtendedTcpTable / GetExtendedUdpTable).");
        let mut tracker = ConnectionTracker::new();
        let mut ticker = interval(poll_interval);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);

        loop {
            let _ = ticker.tick().await;

            let snapshot = match tokio::task::spawn_blocking(snapshot_connections).await {
                Ok(Ok(snapshot)) => snapshot,
                Ok(Err(e)) => {
                    // Keep the previous state: a failed poll must not look like every
                    // connection having closed.
                    error!("Connection snapshot failed: {e}");
                    continue;
                }
                Err(e) => {
                    error!("Connection snapshot task failed: {e}");
                    continue;
                }
            };

            let events = tracker.update(&snapshot);
            if !events.is_empty() {
                debug!(
                    events = events.len(),
                    tracked = tracker.len(),
                    "connection changes"
                );
            }
            for event in events {
                if tx.send(event).await.is_err() {
                    info!("Event receiver dropped, stopping connection polling.");
                    return;
                }
            }
        }
    }));
}

/// Returns the processes that own at least one connection, with name and executable path.
///
/// # Errors
/// Fails if reading the connection tables fails.
pub fn snapshot_network_processes() -> BwpResult<Vec<ProcessInfo>> {
    use sysinfo::{ProcessesToUpdate, System};

    let connections = snapshot_connections()?;
    let mut system = System::new();
    let _ = system.refresh_processes(ProcessesToUpdate::All, true);
    Ok(processes_from_connections(&connections, |pid| {
        let process = system.process(sysinfo::Pid::from_u32(pid))?;
        let name = process.name().to_string_lossy().into_owned();
        let exe = process.exe().map(|p| p.to_string_lossy().into_owned());
        (!name.is_empty()).then_some((name, exe))
    }))
}

/// Returns every TCP (IPv4/IPv6, all states) and UDP (IPv4/IPv6) endpoint with its owning PID.
///
/// # Errors
/// Fails if one of the four tables cannot be read.
pub fn snapshot_connections() -> BwpResult<Vec<ConnectionInfo>> {
    let mut all = Vec::new();
    all.extend(tcp4()?);
    all.extend(tcp6()?);
    all.extend(udp4()?);
    all.extend(udp6()?);
    Ok(all)
}

/// Reads one table into an aligned buffer, returning the buffer and the bytes written.
fn read_table(
    what: &str,
    mut api: impl FnMut(Option<*mut c_void>, *mut u32) -> u32,
) -> BwpResult<(Vec<u64>, usize)> {
    fetch_table(|buffer, size| {
        let ptr = buffer.map(|b| b.as_mut_ptr().cast::<c_void>());
        api(ptr, std::ptr::from_mut(size))
    })
    .map_err(|e| BwpError::Monitor(format!("{what}: {e}")))
}

/// Interprets `buffer` as a `MIB_*TABLE_OWNER_PID` (a `u32` count followed by rows).
///
/// Returns `None` if the buffer is too small for the advertised number of rows.
///
/// # Safety
/// `buffer` must hold a table of `Row` structs filled in by the IP Helper API, with
/// `rows_offset` the offset of its first row.
unsafe fn table_rows<Row>(buffer: &[u64], written: usize, rows_offset: usize) -> Option<&[Row]> {
    if written < size_of::<u32>() || written > std::mem::size_of_val(buffer) {
        return None;
    }
    let base = buffer.as_ptr().cast::<u8>();
    // SAFETY: `written >= 4`, and the buffer is 8-byte aligned.
    let entries = unsafe { base.cast::<u32>().read() };
    if !rows_fit(entries, size_of::<Row>(), rows_offset, written) {
        return None;
    }
    // SAFETY: the check above guarantees `entries` rows lie inside the buffer, and the
    // API wrote them; `Row` consists of `u32`/`u8` fields so any alignment >= 4 is fine.
    Some(unsafe {
        std::slice::from_raw_parts(base.add(rows_offset).cast::<Row>(), entries as usize)
    })
}

fn tcp4() -> BwpResult<Vec<ConnectionInfo>> {
    let (buffer, written) = read_table("IPv4 TCP table", |ptr, size| {
        // SAFETY: `ptr` is null or points to a buffer of `*size` bytes, `size` is valid.
        unsafe {
            GetExtendedTcpTable(
                ptr,
                size,
                false,
                u32::from(AF_INET.0),
                TCP_TABLE_OWNER_PID_ALL,
                0,
            )
        }
    })?;
    // SAFETY: the buffer was filled by GetExtendedTcpTable(AF_INET, OWNER_PID_ALL).
    let rows = unsafe {
        table_rows::<MIB_TCPROW_OWNER_PID>(
            &buffer,
            written,
            offset_of!(MIB_TCPTABLE_OWNER_PID, table),
        )
    }
    .ok_or_else(|| BwpError::Monitor("malformed IPv4 TCP table".to_string()))?;
    Ok(rows
        .iter()
        .map(|r| {
            tcp_connection(&RawTcpRow {
                local_ip: IpAddr::V4(ipv4_from_network_order(r.dwLocalAddr)),
                local_port: r.dwLocalPort,
                remote_ip: IpAddr::V4(ipv4_from_network_order(r.dwRemoteAddr)),
                remote_port: r.dwRemotePort,
                state: r.dwState,
                pid: r.dwOwningPid,
            })
        })
        .collect())
}

fn tcp6() -> BwpResult<Vec<ConnectionInfo>> {
    let (buffer, written) = read_table("IPv6 TCP table", |ptr, size| {
        // SAFETY: as in `tcp4`.
        unsafe {
            GetExtendedTcpTable(
                ptr,
                size,
                false,
                u32::from(AF_INET6.0),
                TCP_TABLE_OWNER_PID_ALL,
                0,
            )
        }
    })?;
    // SAFETY: the buffer was filled by GetExtendedTcpTable(AF_INET6, OWNER_PID_ALL).
    let rows = unsafe {
        table_rows::<MIB_TCP6ROW_OWNER_PID>(
            &buffer,
            written,
            offset_of!(MIB_TCP6TABLE_OWNER_PID, table),
        )
    }
    .ok_or_else(|| BwpError::Monitor("malformed IPv6 TCP table".to_string()))?;
    Ok(rows
        .iter()
        .map(|r| {
            tcp_connection(&RawTcpRow {
                local_ip: IpAddr::V6(ipv6_from_bytes(r.ucLocalAddr)),
                local_port: r.dwLocalPort,
                remote_ip: IpAddr::V6(ipv6_from_bytes(r.ucRemoteAddr)),
                remote_port: r.dwRemotePort,
                state: r.dwState,
                pid: r.dwOwningPid,
            })
        })
        .collect())
}

fn udp4() -> BwpResult<Vec<ConnectionInfo>> {
    let (buffer, written) = read_table("IPv4 UDP table", |ptr, size| {
        // SAFETY: as in `tcp4`.
        unsafe {
            GetExtendedUdpTable(
                ptr,
                size,
                false,
                u32::from(AF_INET.0),
                UDP_TABLE_OWNER_PID,
                0,
            )
        }
    })?;
    // SAFETY: the buffer was filled by GetExtendedUdpTable(AF_INET, OWNER_PID).
    let rows = unsafe {
        table_rows::<MIB_UDPROW_OWNER_PID>(
            &buffer,
            written,
            offset_of!(MIB_UDPTABLE_OWNER_PID, table),
        )
    }
    .ok_or_else(|| BwpError::Monitor("malformed IPv4 UDP table".to_string()))?;
    Ok(rows
        .iter()
        .map(|r| {
            udp_connection(
                IpAddr::V4(ipv4_from_network_order(r.dwLocalAddr)),
                r.dwLocalPort,
                r.dwOwningPid,
            )
        })
        .collect())
}

fn udp6() -> BwpResult<Vec<ConnectionInfo>> {
    let (buffer, written) = read_table("IPv6 UDP table", |ptr, size| {
        // SAFETY: as in `tcp4`.
        unsafe {
            GetExtendedUdpTable(
                ptr,
                size,
                false,
                u32::from(AF_INET6.0),
                UDP_TABLE_OWNER_PID,
                0,
            )
        }
    })?;
    // SAFETY: the buffer was filled by GetExtendedUdpTable(AF_INET6, OWNER_PID).
    let rows = unsafe {
        table_rows::<MIB_UDP6ROW_OWNER_PID>(
            &buffer,
            written,
            offset_of!(MIB_UDP6TABLE_OWNER_PID, table),
        )
    }
    .ok_or_else(|| BwpError::Monitor("malformed IPv6 UDP table".to_string()))?;
    Ok(rows
        .iter()
        .map(|r| {
            udp_connection(
                IpAddr::V6(ipv6_from_bytes(r.ucLocalAddr)),
                r.dwLocalPort,
                r.dwOwningPid,
            )
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Smoke test against the real API: the machine running the tests has a network stack,
    /// and the snapshot must be internally consistent.
    #[test]
    fn live_snapshot_is_readable() {
        let connections = snapshot_connections().unwrap();
        for conn in &connections {
            assert!(conn.local_addr.parse::<std::net::SocketAddr>().is_ok());
        }
    }
}
