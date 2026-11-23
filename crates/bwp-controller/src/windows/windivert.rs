//! **Experimental** WinDivert packet shaper. Opt-in, off by default, never run on real
//! hardware by the author: read this before enabling it.
//!
//! Enabling requires *both* the `experimental-windivert` Cargo feature at build time and
//! `[shaping] experimental_windivert = true` in `config.toml` (read at startup). The
//! process needs Administrator rights and `WinDivert.dll` plus `WinDivert64.sys` next to
//! the executable.
//!
//! # Design
//!
//! * A NETWORK-layer handle with the filter `!loopback and (tcp or udp)`. The filter does
//!   **not** use the `pid` field (it does not exist at that layer).
//! * The handle is opened only while at least one PID is throttled and is shut down and
//!   closed as soon as the last rule goes away (or on `ClearAll`), so with no active
//!   throttle no packet ever passes through user space.
//! * Packets are attributed to PIDs through the connection table
//!   ([`bwp_monitor::snapshot_connections`]): a [`PortMap`] of local port to PID, refreshed
//!   every second and immediately (rate limited) when a packet of an unknown flow is seen.
//!   Unknown or ambiguous flows are **never** throttled.
//! * **Every packet is re-injected immediately unless its PID is throttled.** Throttled
//!   PIDs go through a per-PID token bucket: packets that do not fit are *delayed* in a
//!   queue (up to 500 ms, 8 MiB overall) and released by a dedicated timer thread. Only
//!   when the delay would exceed that, or the queue is full, is a packet dropped.
//! * No busy loops: the capture thread blocks in `WinDivertRecv`, the release thread waits
//!   on a condition variable until the next packet is due, and the port refresher parks.
//!
//! # Safety net
//!
//! On any error (capture failure, repeated send failures), on `ClearAll`, on shutdown and
//! on drop, the handle is `WinDivertShutdown(RECV)`: from that moment the driver stops
//! diverting packets, so traffic flows normally again; queued packets are then flushed
//! (sent) and the handle is closed. A process crash closes the handle through the OS.
//! Packets already inside WinDivert's kernel queue at that moment are drained by the
//! capture thread and passed through as well, but anything still unread when the handle is
//! closed is lost (a few packets at most; TCP retransmits).
//!
//! # Known limitations
//!
//! * Attribution is by local port: UDP sockets sharing a port and flows that start between
//!   two table refreshes are not throttled. IPv6 extension headers and fragments are
//!   passed through unthrottled.
//! * Inbound traffic is delayed, not rate limited at the sender: it slows TCP via
//!   back-pressure but cannot slow UDP down at the source.
//! * Delayed packets keep their original checksum/offload flags; virtual adapters and
//!   unusual offload setups are untested.
//! * `WinDivert64.sys` must be loadable (Secure Boot / HVCI drivers blocklists may refuse it).
#![allow(unsafe_code)]

use std::collections::BTreeMap;
use std::ffi::CString;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use bwp_core::{BandwidthLimit, BwpError, BwpResult};
use tokio::sync::{RwLock, mpsc};
use tracing::{debug, error, info, warn};
use windivert_sys::address::WINDIVERT_ADDRESS;
use windivert_sys::{
    WINDIVERT_MTU_MAX, WinDivertClose, WinDivertFlags, WinDivertLayer, WinDivertOpen,
    WinDivertRecv, WinDivertSend, WinDivertShutdown, WinDivertShutdownMode,
};
use windows48::Win32::Foundation::HANDLE;

use crate::flow::parse_flow;
use crate::portmap::PortMap;
use crate::shaper::{Shaper, ShaperConfig, Submit};
use crate::traits::ThrottleCommand;

/// WinDivert filter: all non-loopback TCP/UDP. Process filtering is impossible at this layer.
const FILTER: &str = "!loopback and (tcp or udp)";

/// Interval of the periodic connection-table refresh.
const REFRESH_INTERVAL: Duration = Duration::from_secs(1);
/// Minimum spacing of refreshes requested by unknown flows.
const REFRESH_REQUEST_SPACING: Duration = Duration::from_millis(250);
/// Consecutive `WinDivertSend` failures after which the shaper gives up and releases traffic.
const MAX_CONSECUTIVE_SEND_FAILURES: u32 = 50;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Owned WinDivert handle. Closed on drop.
struct Handle(HANDLE);

impl Handle {
    fn open(filter: &str) -> io::Result<Self> {
        let filter =
            CString::new(filter).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
        // SAFETY: `filter` is a valid NUL-terminated string that outlives the call.
        let handle = unsafe {
            WinDivertOpen(
                filter.as_ptr(),
                WinDivertLayer::Network,
                0,
                WinDivertFlags::new(),
            )
        };
        if handle.is_invalid() {
            Err(io::Error::last_os_error())
        } else {
            Ok(Self(handle))
        }
    }

    /// Blocks until a packet arrives; returns its length and address.
    fn recv(&self, buffer: &mut [u8]) -> io::Result<(usize, WINDIVERT_ADDRESS)> {
        let mut address = WINDIVERT_ADDRESS::default();
        let mut received = 0u32;
        // SAFETY: the buffer pointer/length describe a live mutable slice, `received` and
        // `address` are valid for writes, the handle is open.
        let ok = unsafe {
            WinDivertRecv(
                self.0,
                buffer.as_mut_ptr().cast(),
                u32::try_from(buffer.len()).unwrap_or(u32::MAX),
                &mut received,
                &mut address,
            )
        };
        if ok.as_bool() {
            Ok((received as usize, address))
        } else {
            Err(io::Error::last_os_error())
        }
    }

    fn send(&self, packet: &[u8], address: &WINDIVERT_ADDRESS) -> io::Result<()> {
        let mut sent = 0u32;
        // SAFETY: `packet` is a live slice, `address` and `sent` are valid, the handle is open.
        let ok = unsafe {
            WinDivertSend(
                self.0,
                packet.as_ptr().cast(),
                u32::try_from(packet.len()).unwrap_or(u32::MAX),
                &mut sent,
                address,
            )
        };
        if ok.as_bool() {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }

    /// Stops the driver from diverting new packets (they flow normally again) and makes
    /// a blocked `recv` return once the queue is drained. Sending stays possible.
    fn shutdown_recv(&self) {
        // SAFETY: the handle is open. Failure only means it was already shut down.
        let ok = unsafe { WinDivertShutdown(self.0, WinDivertShutdownMode::Recv) };
        if !ok.as_bool() {
            debug!("WinDivertShutdown failed: {}", io::Error::last_os_error());
        }
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: the handle is open and no other reference exists any more.
        let _ = unsafe { WinDivertClose(self.0) };
    }
}

/// A packet being held by the shaper.
struct Held {
    data: Vec<u8>,
    address: WINDIVERT_ADDRESS,
}

struct State {
    shaper: Shaper<Held>,
    ports: PortMap,
}

struct Shared {
    state: Mutex<State>,
    /// Wakes the release thread when the queue changes or on stop.
    wake: Condvar,
    /// Set when the session is over (stop requested or capture ended).
    stop: AtomicBool,
}

/// Wakes the release thread for shutdown.
///
/// The mutex is taken first: the release thread holds it from checking the stop flag until
/// it is atomically parked on the condition variable, so the notification cannot be lost
/// in between.
fn wake_for_stop(shared: &Shared) {
    let _guard = lock(&shared.state);
    shared.wake.notify_all();
}

/// Sends every queued packet right away and empties the queue.
fn flush_all(handle: &Handle, shared: &Shared) {
    let held = lock(&shared.state).shaper.clear();
    for packet in held {
        if let Err(e) = handle.send(&packet.data, &packet.address) {
            debug!("could not re-inject queued packet while flushing: {e}");
        }
    }
}

/// Captures packets and either re-injects them at once or hands them to the shaper.
fn capture_loop(handle: &Handle, shared: &Shared, refresher: &thread::Thread) {
    let mut buffer = vec![0u8; WINDIVERT_MTU_MAX as usize];
    let mut last_refresh_request = Instant::now();
    let mut send_failures = 0u32;

    loop {
        let (len, address) = match handle.recv(&mut buffer) {
            Ok(received) => received,
            Err(e) => {
                if shared.stop.load(Ordering::Acquire) {
                    debug!("WinDivert capture ended: {e}");
                } else {
                    error!("WinDivert capture failed, releasing all traffic: {e}");
                }
                break;
            }
        };
        let packet = &buffer[..len];
        let flow = parse_flow(packet);

        // One short critical section: attribute the packet and see whether it is throttled.
        let (pid, throttled) = {
            let state = lock(&shared.state);
            let pid = flow.and_then(|f| state.ports.pid_for(&f, address.outbound()));
            (pid, state.shaper.is_throttled(pid))
        };
        if pid.is_none()
            && flow.is_some()
            && last_refresh_request.elapsed() >= REFRESH_REQUEST_SPACING
        {
            last_refresh_request = Instant::now();
            refresher.unpark();
        }

        let send_now = if throttled {
            let held = Held {
                data: packet.to_vec(),
                address,
            };
            let outcome = lock(&shared.state)
                .shaper
                .submit(pid, len, held, Instant::now());
            match outcome {
                Submit::Pass(_) => true,
                Submit::Queued => {
                    shared.wake.notify_one();
                    false
                }
                Submit::Dropped => false,
            }
        } else {
            true
        };

        if send_now {
            match handle.send(packet, &address) {
                Ok(()) => send_failures = 0,
                Err(e) => {
                    send_failures += 1;
                    debug!("re-injecting a packet failed: {e}");
                    if send_failures >= MAX_CONSECUTIVE_SEND_FAILURES {
                        error!("WinDivert cannot re-inject packets ({e}); releasing all traffic.");
                        break;
                    }
                }
            }
        }
    }

    // Whatever the reason: stop diverting, then let everything that is held go.
    shared.stop.store(true, Ordering::Release);
    handle.shutdown_recv();
    wake_for_stop(shared);
    flush_all(handle, shared);
}

/// Sends delayed packets when they become due; sleeps on a condition variable otherwise.
fn release_loop(handle: &Handle, shared: &Shared) {
    let mut state = lock(&shared.state);
    loop {
        if shared.stop.load(Ordering::Acquire) {
            return;
        }
        let now = Instant::now();
        let mut due = Vec::new();
        while let Some(packet) = state.shaper.pop_due(now) {
            due.push(packet);
        }
        if !due.is_empty() {
            drop(state);
            for packet in due {
                if let Err(e) = handle.send(&packet.data, &packet.address) {
                    debug!("sending a delayed packet failed: {e}");
                }
            }
            state = lock(&shared.state);
            continue;
        }
        state = match state.shaper.next_release() {
            Some(at) => {
                let wait = at.saturating_duration_since(now);
                shared
                    .wake
                    .wait_timeout(state, wait)
                    .unwrap_or_else(PoisonError::into_inner)
                    .0
            }
            None => shared
                .wake
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner),
        };
    }
}

/// Keeps the local-port to PID map fresh.
fn refresh_loop(shared: &Shared) {
    while !shared.stop.load(Ordering::Acquire) {
        match bwp_monitor::snapshot_connections() {
            Ok(connections) => {
                let ports = PortMap::from_connections(&connections);
                lock(&shared.state).ports = ports;
            }
            Err(e) => warn!("could not refresh the connection table: {e}"),
        }
        thread::park_timeout(REFRESH_INTERVAL);
    }
}

/// An open WinDivert handle plus its worker threads.
struct Session {
    handle: Arc<Handle>,
    shared: Arc<Shared>,
    threads: Vec<JoinHandle<()>>,
    refresher: thread::Thread,
    capture_finished: Arc<AtomicBool>,
}

impl Session {
    fn start() -> BwpResult<Self> {
        let initial_ports = match bwp_monitor::snapshot_connections() {
            Ok(connections) => PortMap::from_connections(&connections),
            Err(e) => {
                return Err(BwpError::Controller(format!(
                    "cannot read the connection table, refusing to start shaping: {e}"
                )));
            }
        };

        let handle = Arc::new(Handle::open(FILTER).map_err(|e| {
            BwpError::Controller(format!(
                "WinDivertOpen failed: {e}. Run as Administrator and keep WinDivert.dll and \
                 WinDivert64.sys next to bwp-engine.exe."
            ))
        })?);
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                shaper: Shaper::new(ShaperConfig::default()),
                ports: initial_ports,
            }),
            wake: Condvar::new(),
            stop: AtomicBool::new(false),
        });
        let capture_finished = Arc::new(AtomicBool::new(false));

        let spawn = |name: &str, job: Box<dyn FnOnce() + Send>| {
            thread::Builder::new()
                .name(name.to_string())
                .spawn(job)
                .map_err(|e| BwpError::Controller(format!("cannot spawn thread {name}: {e}")))
        };

        let refresh_thread = {
            let shared = shared.clone();
            spawn("bwp-port-refresh", Box::new(move || refresh_loop(&shared)))
        };
        let refresh_thread = match refresh_thread {
            Ok(t) => t,
            Err(e) => {
                // Nothing was diverted yet: dropping the handle closes it.
                return Err(e);
            }
        };
        let refresher = refresh_thread.thread().clone();

        let release_thread = {
            let (handle, shared) = (handle.clone(), shared.clone());
            spawn(
                "bwp-shaper-release",
                Box::new(move || release_loop(&handle, &shared)),
            )
        };
        let capture_thread = {
            let (handle, shared, finished) =
                (handle.clone(), shared.clone(), capture_finished.clone());
            let refresher = refresher.clone();
            spawn(
                "bwp-shaper-capture",
                Box::new(move || {
                    capture_loop(&handle, &shared, &refresher);
                    finished.store(true, Ordering::Release);
                }),
            )
        };

        let mut session = Self {
            handle,
            shared,
            threads: vec![refresh_thread],
            refresher,
            capture_finished,
        };
        match (release_thread, capture_thread) {
            (Ok(release), Ok(capture)) => {
                session.threads.push(release);
                session.threads.push(capture);
                info!("WinDivert packet shaping started (experimental).");
                Ok(session)
            }
            (release, capture) => {
                session.threads.extend(release.into_iter().chain(capture));
                session.stop();
                Err(BwpError::Controller(
                    "could not start the shaper threads".to_string(),
                ))
            }
        }
    }

    /// Whether the capture thread has ended on its own (an error), meaning shaping is over.
    fn has_failed(&self) -> bool {
        self.capture_finished.load(Ordering::Acquire)
    }

    fn set_limit(&self, pid: u32, limit: BandwidthLimit) {
        lock(&self.shared.state)
            .shaper
            .set_limit(pid, limit, Instant::now());
    }

    fn remove(&self, pid: u32) {
        let released = lock(&self.shared.state).shaper.remove(pid);
        for packet in released {
            if let Err(e) = self.handle.send(&packet.data, &packet.address) {
                debug!("could not release a queued packet: {e}");
            }
        }
    }

    /// Stops diverting, flushes queued packets, joins the threads and closes the handle.
    fn stop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
        // After this the driver no longer diverts new packets: traffic flows normally.
        self.handle.shutdown_recv();
        wake_for_stop(&self.shared);
        self.refresher.unpark();
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
        flush_all(&self.handle, &self.shared);
        info!("WinDivert packet shaping stopped.");
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if !self.threads.is_empty() {
            self.stop();
        }
    }
}

fn publish(
    active_rules: &Arc<RwLock<Vec<(u32, BandwidthLimit)>>>,
    rules: &BTreeMap<u32, BandwidthLimit>,
) {
    *active_rules.blocking_write() = rules.iter().map(|(&pid, &limit)| (pid, limit)).collect();
}

/// Runs the experimental shaping engine on the current thread until the command channel
/// closes.
///
/// # Errors
/// Currently never fails; failures are logged and leave traffic unthrottled.
pub fn run_shaping_engine(
    mut command_rx: mpsc::Receiver<ThrottleCommand>,
    active_rules: &Arc<RwLock<Vec<(u32, BandwidthLimit)>>>,
) -> BwpResult<()> {
    warn!(
        "EXPERIMENTAL WinDivert packet shaping is enabled ([shaping] experimental_windivert). \
         It has not been tested on real hardware; set it to false to turn it off."
    );

    let mut rules: BTreeMap<u32, BandwidthLimit> = BTreeMap::new();
    let mut session: Option<Session> = None;

    // `blocking_recv` parks the thread until a command arrives: no busy loop.
    while let Some(command) = command_rx.blocking_recv() {
        if session.as_ref().is_some_and(Session::has_failed) {
            error!("The WinDivert shaper stopped after an error; traffic is no longer limited.");
            session = None;
            rules.clear();
        }

        match command {
            ThrottleCommand::ApplyLimit { pid, limit } => {
                if session.is_none() {
                    match Session::start() {
                        Ok(s) => session = Some(s),
                        Err(e) => {
                            error!("Packet shaping unavailable, traffic stays unlimited: {e}")
                        }
                    }
                }
                if let Some(session) = &session {
                    session.set_limit(pid, limit);
                    let _ = rules.insert(pid, limit);
                }
            }
            ThrottleCommand::RemoveLimit { pid } => {
                if rules.remove(&pid).is_some() {
                    if let Some(session) = &session {
                        session.remove(pid);
                    }
                }
                if rules.is_empty() {
                    session = None;
                }
            }
            ThrottleCommand::ClearAll => {
                rules.clear();
                session = None;
            }
        }
        publish(active_rules, &rules);
    }

    drop(session);
    Ok(())
}
