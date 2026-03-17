//! Polling Fallback Implementation using GetExtendedTcpTable.
//!
//! This module kicks in if the ETW pipeline fails. It periodically invokes Windows APIs
//! to snapshot active networking states, computes diffs, and synthesizes `MonitorEvent`s.
//! O(N) cost per poll interval.

use bwp_core::{BwpError, MonitorEvent, BwpResult, ConnectionInfo, ProcessInfo, Protocol, ConnectionState};
use tokio::sync::mpsc;
use tokio::time::{sleep, Duration};
use tracing::{debug, error, info};
use std::mem;
use std::net::{Ipv4Addr, Ipv6Addr};

use windows::Win32::NetworkManagement::IpHelper::{
    GetExtendedTcpTable, GetExtendedUdpTable, MIB_TCPROW_OWNER_PID, MIB_TCPTABLE_OWNER_PID,
    MIB_TCP_STATE_ESTAB, MIB_TCP_STATE_LISTEN,
    TCP_TABLE_OWNER_PID_ALL, UDP_TABLE_OWNER_PID, MIB_UDPROW_OWNER_PID, MIB_UDPTABLE_OWNER_PID,
    MIB_TCP6ROW_OWNER_PID, MIB_TCP6TABLE_OWNER_PID, MIB_UDP6ROW_OWNER_PID, MIB_UDP6TABLE_OWNER_PID,
};
use windows::Win32::Networking::WinSock::{AF_INET, AF_INET6};
use windows::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, NO_ERROR};

/// Spawns the asynchronous polling loop.
pub fn spawn_polling_loop(tx: mpsc::Sender<MonitorEvent>) {
    tokio::spawn(async move {
        info!("Graceful fallback to GetExtendedTcpTable polling loop started.");

        let mut previous_state: Vec<ConnectionInfo> = vec![];

        loop {
            match snapshot_connections() {
                Ok(current_state) => {
                    // Compute Diffs: ConnectionOpened & ConnectionClosed
                    // (Simplified logic for demonstration: actual impl maintains a HashSet)
                    // Synthesize ETW-like MonitorEvents here.
                    
                    for conn in &current_state {
                        if !previous_state.iter().any(|p| p.pid == conn.pid && p.local_addr == conn.local_addr) {
                            let _ = tx.send(MonitorEvent::ConnectionOpened(conn.clone())).await;
                        }
                    }

                    for prev in &previous_state {
                        if !current_state.iter().any(|c| c.pid == prev.pid && c.local_addr == prev.local_addr) {
                            let _ = tx.send(MonitorEvent::ConnectionClosed(prev.clone())).await;
                        }
                    }

                    previous_state = current_state;
                }
                Err(e) => {
                    error!("Polling failed: {e}");
                }
            }

            sleep(Duration::from_millis(500)).await;
        }
    });
}

/// Returns a frozen snapshot of processes using the net APIs (`GetExtendedTcpTable`, `GetExtendedUdpTable`).
pub fn snapshot_network_processes() -> BwpResult<Vec<ProcessInfo>> {
    // Phase 1 Dummy implementation.
    // In actual implementation, we map `MIB_TCPROW_OWNER_PID` rows.
    Ok(vec![])
}

/// Returns a frozen snapshot of active outbound connections per PID.
pub fn snapshot_connections() -> BwpResult<Vec<ConnectionInfo>> {
    let mut all_connections = Vec::new();

    match get_extended_tcp_table(AF_INET.0 as u32) {
        Ok(tcp_conns) => all_connections.extend(tcp_conns),
        Err(e) => error!("Failed to snapshot TCP IPv4 connections: {e}"),
    }
    match get_extended_tcp_table(AF_INET6.0 as u32) {
        Ok(tcp_conns) => all_connections.extend(tcp_conns),
        Err(e) => error!("Failed to snapshot TCP IPv6 connections: {e}"),
    }

    match get_extended_udp_table(AF_INET.0 as u32) {
        Ok(udp_conns) => all_connections.extend(udp_conns),
        Err(e) => error!("Failed to snapshot UDP IPv4 connections: {e}"),
    }
    match get_extended_udp_table(AF_INET6.0 as u32) {
        Ok(udp_conns) => all_connections.extend(udp_conns),
        Err(e) => error!("Failed to snapshot UDP IPv6 connections: {e}"),
    }

    Ok(all_connections)
}

fn map_tcp_state(state_val: u32) -> ConnectionState {
    let state = windows::Win32::NetworkManagement::IpHelper::MIB_TCP_STATE(state_val as i32);
    match state {
        MIB_TCP_STATE_ESTAB => ConnectionState::Established,
        MIB_TCP_STATE_LISTEN => ConnectionState::Listening,
        _ => ConnectionState::Other,
    }
}

#[allow(unsafe_code)]
fn get_extended_tcp_table(family: u32) -> BwpResult<Vec<ConnectionInfo>> {
    let mut buffer: Vec<u32> = vec![];
    let mut size: u32 = 0;

    // First call to get the required size
    let mut res = unsafe {
        GetExtendedTcpTable(
            None,
            &mut size,
            true, // sort
            family,
            TCP_TABLE_OWNER_PID_ALL,
            0,
        )
    };

    while res == ERROR_INSUFFICIENT_BUFFER.0 {
        buffer.resize((size as usize + 3) / 4, 0);
        res = unsafe {
            GetExtendedTcpTable(
                Some(buffer.as_mut_ptr() as *mut _),
                &mut size,
                true,
                family,
                TCP_TABLE_OWNER_PID_ALL,
                0,
            )
        };
    }

    if res != NO_ERROR.0 {
        return Err(BwpError::Platform(format!("GetExtendedTcpTable failed with code {}", res)));
    }

    if buffer.is_empty() {
        return Ok(vec![]);
    }

    let mut connections = Vec::new();

    if family == AF_INET.0 as u32 {
        let table = unsafe { &*(buffer.as_ptr() as *const MIB_TCPTABLE_OWNER_PID) };
        let num_entries = table.dwNumEntries as usize;
        let table_ptr = table.table.as_ptr();

        for i in 0..num_entries {
            let row = unsafe { &*table_ptr.add(i) };
            let local_ip = Ipv4Addr::from(u32::from_be(row.dwLocalAddr));
            let local_port = u16::from_be(row.dwLocalPort as u16);

            let remote_ip = Ipv4Addr::from(u32::from_be(row.dwRemoteAddr));
            let remote_port = u16::from_be(row.dwRemotePort as u16);

            let state_enum = windows::Win32::NetworkManagement::IpHelper::MIB_TCP_STATE(row.dwState as i32);
            let remote_addr = if state_enum == MIB_TCP_STATE_LISTEN {
                None
            } else {
                Some(format!("{remote_ip}:{remote_port}"))
            };

            connections.push(ConnectionInfo {
                pid: row.dwOwningPid,
                protocol: Protocol::Tcp,
                local_addr: format!("{local_ip}:{local_port}"),
                remote_addr,
                state: map_tcp_state(row.dwState),
            });
        }
    } else if family == AF_INET6.0 as u32 {
        let table = unsafe { &*(buffer.as_ptr() as *const MIB_TCP6TABLE_OWNER_PID) };
        let num_entries = table.dwNumEntries as usize;
        let table_ptr = table.table.as_ptr();

        for i in 0..num_entries {
            let row = unsafe { &*table_ptr.add(i) };
            let local_ip = Ipv6Addr::from(row.ucLocalAddr);
            let local_port = u16::from_be(row.dwLocalPort as u16);

            let remote_ip = Ipv6Addr::from(row.ucRemoteAddr);
            let remote_port = u16::from_be(row.dwRemotePort as u16);

            let state_enum = windows::Win32::NetworkManagement::IpHelper::MIB_TCP_STATE(row.dwState as i32);
            let remote_addr = if state_enum == MIB_TCP_STATE_LISTEN {
                None
            } else {
                Some(format!("[{remote_ip}]:{remote_port}"))
            };

            connections.push(ConnectionInfo {
                pid: row.dwOwningPid,
                protocol: Protocol::Tcp,
                local_addr: format!("[{local_ip}]:{local_port}"),
                remote_addr,
                state: map_tcp_state(row.dwState),
            });
        }
    }

    Ok(connections)
}

#[allow(unsafe_code)]
fn get_extended_udp_table(family: u32) -> BwpResult<Vec<ConnectionInfo>> {
    let mut buffer: Vec<u32> = vec![];
    let mut size: u32 = 0;

    let mut res = unsafe {
        GetExtendedUdpTable(
            None,
            &mut size,
            true,
            family,
            UDP_TABLE_OWNER_PID,
            0,
        )
    };

    while res == ERROR_INSUFFICIENT_BUFFER.0 {
        buffer.resize((size as usize + 3) / 4, 0);
        res = unsafe {
            GetExtendedUdpTable(
                Some(buffer.as_mut_ptr() as *mut _),
                &mut size,
                true,
                family,
                UDP_TABLE_OWNER_PID,
                0,
            )
        };
    }

    if res != NO_ERROR.0 {
        return Err(BwpError::Platform(format!("GetExtendedUdpTable failed with code {}", res)));
    }

    if buffer.is_empty() {
        return Ok(vec![]);
    }

    let mut connections = Vec::new();

    if family == AF_INET.0 as u32 {
        let table = unsafe { &*(buffer.as_ptr() as *const MIB_UDPTABLE_OWNER_PID) };
        let num_entries = table.dwNumEntries as usize;
        let table_ptr = table.table.as_ptr();

        for i in 0..num_entries {
            let row = unsafe { &*table_ptr.add(i) };
            let local_ip = Ipv4Addr::from(u32::from_be(row.dwLocalAddr));
            let local_port = u16::from_be(row.dwLocalPort as u16);

            connections.push(ConnectionInfo {
                pid: row.dwOwningPid,
                protocol: Protocol::Udp,
                local_addr: format!("{local_ip}:{local_port}"),
                remote_addr: None,
                state: ConnectionState::Other,
            });
        }
    } else if family == AF_INET6.0 as u32 {
        let table = unsafe { &*(buffer.as_ptr() as *const MIB_UDP6TABLE_OWNER_PID) };
        let num_entries = table.dwNumEntries as usize;
        let table_ptr = table.table.as_ptr();

        for i in 0..num_entries {
            let row = unsafe { &*table_ptr.add(i) };
            let local_ip = Ipv6Addr::from(row.ucLocalAddr);
            let local_port = u16::from_be(row.dwLocalPort as u16);

            connections.push(ConnectionInfo {
                pid: row.dwOwningPid,
                protocol: Protocol::Udp,
                local_addr: format!("[{local_ip}]:{local_port}"),
                remote_addr: None,
                state: ConnectionState::Other,
            });
        }
    }

    Ok(connections)
}
