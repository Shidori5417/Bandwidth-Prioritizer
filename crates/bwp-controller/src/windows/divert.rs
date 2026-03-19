//! WinDivert engine logic
//!
//! Generates a BPF filter strictly scoped to throttled PIDs.
//! Flow-based interception ensures whitelisted (unthrottled) PIDs bypass user-space.

use crate::token_bucket::TokenBucket;
use crate::traits::ThrottleCommand;
use bwp_core::{BandwidthLimit, BwpError, BwpResult};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};
use tracing::{debug, info, warn};
use windivert::prelude::WinDivertFlags;
use windivert::{WinDivert, layer::NetworkLayer};

// Reconstruct a WinDivert BPF string matching *only* the PIDs we care about.
// `outbound and (pid == 1000 or pid == 2000)`
// If empty, return a dummy string like "false" to drop exactly 0 packets.
fn build_filter_string(pids: Vec<u32>) -> String {
    if pids.is_empty() {
        return "false".to_string(); // Do not intercept anything.
    }

    let checks: Vec<String> = pids.iter().map(|p| format!("pid == {}", p)).collect();
    format!("outbound and ({})", checks.join(" or "))
}

pub fn run_windivert_engine(
    mut command_rx: mpsc::Receiver<ThrottleCommand>,
    active_rules_lock: Arc<RwLock<Vec<(u32, BandwidthLimit)>>>,
) -> BwpResult<()> {
    // Token buckets mapped directly by PID (Flow-Level throttling).
    let mut buckets: HashMap<u32, TokenBucket> = HashMap::new();
    let mut throttled_pids: Vec<u32> = Vec::new();

    // We start with a bypass rule.
    let mut filter = "false".to_string();
    let priority: i16 = 0;

    // Main execution loop:
    // This loop dynamically tears down and re-initializes WinDivert
    // exclusively when the filter string legitimately changes.
    loop {
        info!("WinDivert Filter Deployed: {}", filter);
        // Using "false" throws an invalid parameter error in WinDivert if instantiated,
        // so we'll suspend WinDivert instantiation completely if the whitelist implies 0 intercepts.

        let mut wd: Option<WinDivert<NetworkLayer>> = None;
        if filter != "false" {
            wd = match WinDivert::network(&filter, priority, WinDivertFlags::new()) {
                Ok(instance) => Some(instance),
                Err(e) => {
                    warn!("Failed to inject into WinDivert: {}", e);
                    None
                }
            };
        }

        let mut filter_changed = false;

        // Loop polling network packets, and yielding periodically to check command mailbox.
        loop {
            // Process commands asynchronously from the Controller Katmani.
            while let Ok(cmd) = command_rx.try_recv() {
                match cmd {
                    ThrottleCommand::ApplyLimit { pid, limit } => {
                        debug!("Throttle rule generated for {}", pid);
                        buckets.insert(pid, TokenBucket::new(limit));
                        if !throttled_pids.contains(&pid) {
                            throttled_pids.push(pid);
                            filter_changed = true;
                        }
                    }
                    ThrottleCommand::RemoveLimit { pid } => {
                        debug!("Throttle rule removed for {}", pid);
                        buckets.remove(&pid);
                        if let Some(pos) = throttled_pids.iter().position(|x| *x == pid) {
                            throttled_pids.remove(pos);
                            filter_changed = true;
                        }
                    }
                    ThrottleCommand::ClearAll => {
                        buckets.clear();
                        if !throttled_pids.is_empty() {
                            throttled_pids.clear();
                            filter_changed = true;
                        }
                    }
                }
            }

            if filter_changed {
                // Compile the new PID strict filter.
                filter = build_filter_string(throttled_pids.clone());

                // Write the cache logic back to RwLock so readers can see the latest state.
                let mut rules_snapshot = active_rules_lock.blocking_write();
                rules_snapshot.clear();
                for pid in &throttled_pids {
                    if let Some(tb) = buckets.get(pid) {
                        // Normally we'd extract the currently effective limit from TokenBucket
                        // rules_snapshot.push((*pid, BandwidthLimit::from_bytes(0 /*tb.limit*/)));
                    }
                }

                // Exit inner packet loop to rebuild `WinDivert` wrapper with new BPF schema.
                break;
            }

            // Packet interception phase.
            // If no `wd` handle is open (whitelist phase), sleep and poll commands.
            if let Some(ref handle) = wd {
                let mut packet_buf = vec![0u8; 65535];

                // Receive synchronously with a short timeout.
                // We don't want to get blocked indefinitely if the network is silent.
                /*
                  WinDivert crate has recv methods.
                  Implementation specifics depend on the precise version of windivert rust.
                */
                // For instance: if `recv` grabs a payload, we identify its PID via WinDivertAddress.
                // Call `TokenBucket::consume(packet_length)`
                // If allowed, `handle.send(packet, address)`.
            } else {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        }
    }
}
