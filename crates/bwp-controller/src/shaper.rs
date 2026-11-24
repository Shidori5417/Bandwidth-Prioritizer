//! Platform-independent packet shaping core: per-PID token buckets plus a delay queue.
//!
//! The Windows packet engine feeds every captured packet through [`Shaper::submit`] and
//! gets one of three outcomes:
//!
//! * [`Submit::Pass`]: send it right away (the PID is not throttled, or tokens were
//!   available). **This is the default for everything that is not explicitly throttled.**
//! * [`Submit::Queued`]: the packet is held and becomes due after the delay its PID's token
//!   bucket requires; [`Shaper::pop_due`] hands it back when it is time.
//! * [`Submit::Dropped`]: the delay would exceed [`ShaperConfig::max_delay`] or the queue is
//!   full, so the packet is discarded. TCP reacts to this like to congestion; for UDP it is
//!   real loss.
//!
//! The type is generic over the packet payload and takes the clock as an argument so it can
//! be unit tested without any networking.

use crate::token_bucket::TokenBucket;
use bwp_core::BandwidthLimit;
use std::cmp::{Ordering, Reverse};
use std::collections::{BinaryHeap, HashMap};
use std::time::{Duration, Instant};

/// Tunables of the shaper.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShaperConfig {
    /// Longest a packet may be held; beyond that it is dropped instead of queued.
    pub max_delay: Duration,
    /// Upper bound on the bytes held in the delay queue (all PIDs together).
    pub max_queued_bytes: usize,
    /// Smallest rate ever enforced, in bytes per second. A configured limit of 0 would
    /// otherwise cut a process off completely.
    pub min_rate_bytes_per_sec: u64,
    /// Smallest burst, in bytes (at least one full-size packet must fit).
    pub min_burst_bytes: usize,
}

impl Default for ShaperConfig {
    fn default() -> Self {
        Self {
            max_delay: Duration::from_millis(500),
            max_queued_bytes: 8 * 1024 * 1024,
            min_rate_bytes_per_sec: 1_000,
            min_burst_bytes: 3_000,
        }
    }
}

/// What to do with a submitted packet.
#[derive(Debug, PartialEq, Eq)]
pub enum Submit<P> {
    /// Send immediately; the packet is handed back.
    Pass(P),
    /// Held back; it will come out of [`Shaper::pop_due`].
    Queued,
    /// Discard.
    Dropped,
}

struct Entry<P> {
    release_at: Instant,
    seq: u64,
    pid: u32,
    len: usize,
    packet: P,
}

impl<P> PartialEq for Entry<P> {
    fn eq(&self, other: &Self) -> bool {
        self.release_at == other.release_at && self.seq == other.seq
    }
}
impl<P> Eq for Entry<P> {}
impl<P> PartialOrd for Entry<P> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl<P> Ord for Entry<P> {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.release_at, self.seq).cmp(&(other.release_at, other.seq))
    }
}

/// Per-PID rate limiting with a delay queue.
pub struct Shaper<P> {
    config: ShaperConfig,
    buckets: HashMap<u32, TokenBucket>,
    queue: BinaryHeap<Reverse<Entry<P>>>,
    queued_bytes: usize,
    next_seq: u64,
    dropped: u64,
}

impl<P> Shaper<P> {
    /// Creates a shaper with no rules: everything passes.
    #[must_use]
    pub fn new(config: ShaperConfig) -> Self {
        Self {
            config,
            buckets: HashMap::new(),
            queue: BinaryHeap::new(),
            queued_bytes: 0,
            next_seq: 0,
            dropped: 0,
        }
    }

    fn effective(&self, limit: BandwidthLimit) -> (BandwidthLimit, f64) {
        let rate = limit
            .as_bytes_per_sec()
            .max(self.config.min_rate_bytes_per_sec);
        // Burst: 100 ms of traffic, but never less than a couple of full-size packets.
        let burst = (rate / 10).max(self.config.min_burst_bytes as u64);
        (BandwidthLimit::from_bytes(rate), burst as f64)
    }

    /// Starts (or updates) throttling of `pid`.
    pub fn set_limit(&mut self, pid: u32, limit: BandwidthLimit, now: Instant) {
        let (limit, burst) = self.effective(limit);
        match self.buckets.get_mut(&pid) {
            Some(bucket) => bucket.update_limit_with_burst(limit, burst),
            None => {
                let _ = self
                    .buckets
                    .insert(pid, TokenBucket::with_burst_at(limit, burst, now));
            }
        }
    }

    /// Stops throttling `pid` and returns its queued packets (oldest first) so the caller
    /// can send them immediately.
    pub fn remove(&mut self, pid: u32) -> Vec<P> {
        let _ = self.buckets.remove(&pid);
        let (mine, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut self.queue)
            .into_vec()
            .into_iter()
            .partition(|Reverse(e)| e.pid == pid);
        self.queue = rest.into();
        let mut mine: Vec<Entry<P>> = mine.into_iter().map(|Reverse(e)| e).collect();
        mine.sort();
        self.queued_bytes = self
            .queued_bytes
            .saturating_sub(mine.iter().map(|e| e.len).sum());
        mine.into_iter().map(|e| e.packet).collect()
    }

    /// Removes every rule and returns all queued packets (oldest first).
    pub fn clear(&mut self) -> Vec<P> {
        self.buckets.clear();
        let mut all: Vec<Entry<P>> = std::mem::take(&mut self.queue)
            .into_vec()
            .into_iter()
            .map(|Reverse(e)| e)
            .collect();
        all.sort();
        self.queued_bytes = 0;
        all.into_iter().map(|e| e.packet).collect()
    }

    /// Whether any PID is currently throttled.
    #[must_use]
    pub fn has_rules(&self) -> bool {
        !self.buckets.is_empty()
    }

    /// Whether a packet of this (possibly unknown) PID would be subject to a rule.
    #[must_use]
    pub fn is_throttled(&self, pid: Option<u32>) -> bool {
        pid.is_some_and(|pid| self.buckets.contains_key(&pid))
    }

    /// The PIDs currently throttled.
    #[must_use]
    pub fn throttled_pids(&self) -> Vec<u32> {
        let mut pids: Vec<u32> = self.buckets.keys().copied().collect();
        pids.sort_unstable();
        pids
    }

    /// Packets currently held.
    #[must_use]
    pub fn queued_packets(&self) -> usize {
        self.queue.len()
    }

    /// Packets dropped so far because they could not be delayed.
    #[must_use]
    pub fn dropped_packets(&self) -> u64 {
        self.dropped
    }

    /// Decides what to do with a packet of `len` bytes owned by `pid` (`None` = unknown).
    pub fn submit(&mut self, pid: Option<u32>, len: usize, packet: P, now: Instant) -> Submit<P> {
        let Some(pid) = pid else {
            return Submit::Pass(packet);
        };
        let Some(bucket) = self.buckets.get_mut(&pid) else {
            return Submit::Pass(packet);
        };
        let queue_has_room = self.queued_bytes.saturating_add(len) <= self.config.max_queued_bytes;
        let queue_has_pid_packets = self.queue.iter().any(|Reverse(e)| e.pid == pid);

        match bucket.reserve_at(len, now, self.config.max_delay) {
            // Tokens available and nothing of this PID waiting ahead of it: no reordering.
            Some(wait) if wait.is_zero() && !queue_has_pid_packets => Submit::Pass(packet),
            Some(wait) if queue_has_room => {
                self.queued_bytes += len;
                self.queue.push(Reverse(Entry {
                    release_at: now + wait,
                    seq: self.next_seq,
                    pid,
                    len,
                    packet,
                }));
                self.next_seq += 1;
                Submit::Queued
            }
            // Either the wait is too long or the queue is full.
            _ => {
                self.dropped += 1;
                Submit::Dropped
            }
        }
    }

    /// When the earliest queued packet becomes due, if any.
    #[must_use]
    pub fn next_release(&self) -> Option<Instant> {
        self.queue.peek().map(|Reverse(e)| e.release_at)
    }

    /// Pops the next packet that is due at `now`.
    pub fn pop_due(&mut self, now: Instant) -> Option<P> {
        if self
            .queue
            .peek()
            .is_some_and(|Reverse(e)| e.release_at <= now)
        {
            let Reverse(entry) = self.queue.pop()?;
            self.queued_bytes = self.queued_bytes.saturating_sub(entry.len);
            Some(entry.packet)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    fn shaper() -> Shaper<&'static str> {
        Shaper::new(ShaperConfig::default())
    }

    // 80 kbps = 10_000 B/s => burst 3000 B (min), so 1500 B packets: 2 free, then paced
    // at 150 ms each.
    fn limited(now: Instant) -> Shaper<&'static str> {
        let mut s = shaper();
        s.set_limit(7, BandwidthLimit::from_kbps(80), now);
        s
    }

    #[test]
    fn no_rules_means_everything_passes() {
        let t0 = Instant::now();
        let mut s = shaper();
        assert_eq!(s.submit(Some(7), 1500, "a", t0), Submit::Pass("a"));
        assert_eq!(s.submit(None, 1500, "b", t0), Submit::Pass("b"));
        assert!(!s.has_rules());
    }

    #[test]
    fn unknown_and_unthrottled_pids_always_pass() {
        let t0 = Instant::now();
        let mut s = limited(t0);
        for _ in 0..100 {
            assert_eq!(s.submit(None, 1500, "x", t0), Submit::Pass("x"));
            assert_eq!(s.submit(Some(8), 1500, "y", t0), Submit::Pass("y"));
        }
        assert_eq!(s.queued_packets(), 0);
    }

    #[test]
    fn throttled_pid_gets_burst_then_is_delayed() {
        let t0 = Instant::now();
        let mut s = limited(t0);
        assert_eq!(s.submit(Some(7), 1500, "p1", t0), Submit::Pass("p1"));
        assert_eq!(s.submit(Some(7), 1500, "p2", t0), Submit::Pass("p2"));
        assert_eq!(s.submit(Some(7), 1500, "p3", t0), Submit::Queued);
        assert_eq!(s.submit(Some(7), 1500, "p4", t0), Submit::Queued);
        assert_eq!(s.next_release(), Some(t0 + 150 * MS));
        assert_eq!(s.pop_due(t0 + 149 * MS), None);
        assert_eq!(s.pop_due(t0 + 150 * MS), Some("p3"));
        assert_eq!(s.pop_due(t0 + 150 * MS), None);
        assert_eq!(s.pop_due(t0 + 300 * MS), Some("p4"));
        assert_eq!(s.queued_packets(), 0);
    }

    #[test]
    fn order_is_preserved_even_when_tokens_return() {
        let t0 = Instant::now();
        let mut s = limited(t0);
        let _ = s.submit(Some(7), 3000, "burst", t0);
        assert_eq!(s.submit(Some(7), 1500, "q", t0), Submit::Queued);
        // Even if enough tokens have accrued by now, a packet must not overtake the queue.
        assert_eq!(
            s.submit(Some(7), 100, "small", t0 + 100 * MS),
            Submit::Queued
        );
        assert_eq!(s.pop_due(t0 + Duration::from_secs(5)), Some("q"));
        assert_eq!(s.pop_due(t0 + Duration::from_secs(5)), Some("small"));
    }

    #[test]
    fn long_waits_are_dropped_not_queued() {
        let t0 = Instant::now();
        let mut s = limited(t0);
        let mut dropped = 0;
        for _ in 0..50 {
            if s.submit(Some(7), 1500, "p", t0) == Submit::Dropped {
                dropped += 1;
            }
        }
        // 500 ms max delay at 10 kB/s = 5000 B of backlog beyond the 3000 B burst.
        assert!(dropped > 0);
        assert_eq!(s.dropped_packets(), dropped);
        assert!(s.queued_packets() <= 4);
        assert!(s.next_release().unwrap() <= t0 + 500 * MS);
    }

    #[test]
    fn full_queue_drops_instead_of_growing() {
        let t0 = Instant::now();
        let mut s: Shaper<()> = Shaper::new(ShaperConfig {
            max_queued_bytes: 2000,
            max_delay: Duration::from_secs(60),
            ..ShaperConfig::default()
        });
        s.set_limit(7, BandwidthLimit::from_kbps(80), t0);
        assert_eq!(s.submit(Some(7), 3000, (), t0), Submit::Pass(()));
        assert_eq!(s.submit(Some(7), 1500, (), t0), Submit::Queued);
        assert_eq!(s.submit(Some(7), 1500, (), t0), Submit::Dropped);
    }

    #[test]
    fn zero_limit_is_clamped_so_traffic_still_flows() {
        let t0 = Instant::now();
        let mut s = shaper();
        s.set_limit(7, BandwidthLimit::from_kbps(0), t0);
        assert_eq!(s.submit(Some(7), 1000, "a", t0), Submit::Pass("a"));
        // Eventually (min rate 1000 B/s) more traffic is admitted.
        assert!(matches!(
            s.submit(Some(7), 1000, "b", t0 + Duration::from_secs(2)),
            Submit::Pass(_)
        ));
    }

    #[test]
    fn remove_releases_only_that_pids_queue() {
        let t0 = Instant::now();
        let mut s = limited(t0);
        s.set_limit(9, BandwidthLimit::from_kbps(80), t0);
        for name in ["a1", "a2", "a3"] {
            let _ = s.submit(Some(7), 1500, name, t0);
        }
        for name in ["b1", "b2", "b3"] {
            let _ = s.submit(Some(9), 1500, name, t0);
        }
        let released = s.remove(7);
        assert_eq!(released, ["a3"]);
        assert_eq!(s.throttled_pids(), [9]);
        assert_eq!(s.queued_packets(), 1);
        // 7 is no longer throttled.
        assert_eq!(s.submit(Some(7), 1500, "z", t0), Submit::Pass("z"));
    }

    #[test]
    fn clear_returns_everything_in_order_and_resets() {
        let t0 = Instant::now();
        let mut s = limited(t0);
        for name in ["a", "b", "c", "d"] {
            let _ = s.submit(Some(7), 1500, name, t0);
        }
        assert_eq!(s.clear(), ["c", "d"]);
        assert!(!s.has_rules());
        assert_eq!(s.queued_packets(), 0);
        assert_eq!(s.next_release(), None);
        assert_eq!(s.submit(Some(7), 1500, "e", t0), Submit::Pass("e"));
    }

    #[test]
    fn changing_the_limit_takes_effect() {
        let t0 = Instant::now();
        let mut s = limited(t0);
        let _ = s.submit(Some(7), 3000, "burst", t0);
        s.set_limit(7, BandwidthLimit::from_kbps(800), t0);
        // Ten times faster: 1500 B now waits ~15 ms instead of 150 ms.
        assert_eq!(s.submit(Some(7), 1500, "q", t0), Submit::Queued);
        assert!(s.next_release().unwrap() < t0 + 20 * MS);
    }
}
