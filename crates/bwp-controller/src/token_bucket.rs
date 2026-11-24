use bwp_core::BandwidthLimit;
use std::time::{Duration, Instant};

/// A token bucket rate limiter meant to run synchronously inside the packet
/// interception loop.
///
/// Besides the plain [`consume`](Self::consume) check it can *reserve* tokens on credit
/// ([`reserve_at`](Self::reserve_at)): the bucket may go into debt, and the caller is told
/// how long to delay the packet until the debt is paid. That turns the limiter into a
/// pacer, so packets can be queued instead of dropped.
pub struct TokenBucket {
    /// Limit being enforced
    limit: BandwidthLimit,
    /// Capacity of the bucket (max burst), in bytes.
    capacity: f64,
    /// Current available tokens (in bytes). Negative while the bucket is in debt.
    tokens: f64,
    /// Last refill timestamp
    last_refill: Instant,
}

impl TokenBucket {
    /// Instantiate a newly filled bucket for the specified limit, with a burst of one
    /// second worth of traffic.
    pub fn new(limit: BandwidthLimit) -> Self {
        let bytes_per_sec = limit.as_bytes_per_sec() as f64;
        Self::with_burst_at(limit, bytes_per_sec, Instant::now())
    }

    /// Instantiate a newly filled bucket with an explicit burst size (bytes), starting at `now`.
    pub fn with_burst_at(limit: BandwidthLimit, burst_bytes: f64, now: Instant) -> Self {
        Self {
            limit,
            capacity: burst_bytes,
            tokens: burst_bytes,
            last_refill: now,
        }
    }

    /// Try to consume `amount` tokens (bytes) for a packet.
    /// If there aren't enough tokens, returns false, which means the packet should be DROP/Delay.
    pub fn consume(&mut self, amount: usize) -> bool {
        self.consume_at(amount, Instant::now())
    }

    /// Like [`consume`](Self::consume) with an explicit clock.
    pub fn consume_at(&mut self, amount: usize, now: Instant) -> bool {
        self.refill(now);

        let req = amount as f64;
        if self.tokens >= req {
            self.tokens -= req;
            true
        } else {
            false
        }
    }

    /// Reserves `amount` bytes on credit and returns how long the packet has to wait
    /// before it may be sent (`Duration::ZERO` when tokens are available).
    ///
    /// Returns `None` without reserving anything if the wait would exceed `max_wait`
    /// (the caller should drop the packet) or if the limit is zero.
    pub fn reserve_at(
        &mut self,
        amount: usize,
        now: Instant,
        max_wait: Duration,
    ) -> Option<Duration> {
        self.refill(now);

        let rate = self.limit.as_bytes_per_sec() as f64;
        if rate <= 0.0 {
            return None;
        }
        let after = self.tokens - amount as f64;
        let wait = if after >= 0.0 {
            Duration::ZERO
        } else {
            Duration::from_secs_f64(-after / rate)
        };
        if wait > max_wait {
            return None;
        }
        self.tokens = after;
        Some(wait)
    }

    /// Update the bucket's bandwidth limit dynamically.
    pub fn update_limit(&mut self, new_limit: BandwidthLimit) {
        if self.limit != new_limit {
            self.limit = new_limit;
            let new_cap = new_limit.as_bytes_per_sec() as f64;
            self.capacity = new_cap;
            if self.tokens > self.capacity {
                self.tokens = self.capacity;
            }
        }
    }

    /// Like [`update_limit`](Self::update_limit), also resetting the burst size.
    pub fn update_limit_with_burst(&mut self, new_limit: BandwidthLimit, burst_bytes: f64) {
        self.limit = new_limit;
        self.capacity = burst_bytes;
        if self.tokens > self.capacity {
            self.tokens = self.capacity;
        }
    }

    /// Internal synchronous time-delta calculating logic.
    #[inline(always)]
    fn refill(&mut self, now: Instant) {
        let elapsed = now
            .saturating_duration_since(self.last_refill)
            .as_secs_f64();

        let rate = self.limit.as_bytes_per_sec() as f64;
        let generated = elapsed * rate;

        if generated > 0.0 {
            self.tokens = (self.tokens + generated).min(self.capacity);
            self.last_refill = now;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    #[test]
    fn consume_respects_capacity_and_refills() {
        let t0 = Instant::now();
        // 8000 bits/s = 1000 B/s, burst 1000 B.
        let mut bucket = TokenBucket::with_burst_at(BandwidthLimit::from_kbps(8), 1000.0, t0);
        assert!(bucket.consume_at(1000, t0));
        assert!(!bucket.consume_at(1, t0));
        assert!(bucket.consume_at(500, t0 + 500 * MS));
        assert!(!bucket.consume_at(1, t0 + 500 * MS));
    }

    #[test]
    fn refill_never_exceeds_capacity() {
        let t0 = Instant::now();
        let mut bucket = TokenBucket::with_burst_at(BandwidthLimit::from_kbps(8), 1000.0, t0);
        assert!(bucket.consume_at(1000, t0));
        // A long idle period only refills up to the burst size.
        let later = t0 + Duration::from_secs(60);
        assert!(bucket.consume_at(1000, later));
        assert!(!bucket.consume_at(1, later));
    }

    #[test]
    fn reserve_paces_packets_through_debt() {
        let t0 = Instant::now();
        // 1000 B/s with a 1000 B burst.
        let mut bucket = TokenBucket::with_burst_at(BandwidthLimit::from_kbps(8), 1000.0, t0);
        let max = Duration::from_secs(10);
        // Burst goes out immediately.
        assert_eq!(bucket.reserve_at(1000, t0, max), Some(Duration::ZERO));
        // The next 500 B must wait for 500 B of credit: 0.5 s.
        assert_eq!(bucket.reserve_at(500, t0, max), Some(500 * MS));
        // And the one after that waits for the debt of both: 1 s.
        assert_eq!(bucket.reserve_at(500, t0, max), Some(1000 * MS));
        // Half a second later the debt has shrunk accordingly.
        assert_eq!(bucket.reserve_at(0, t0 + 500 * MS, max), Some(500 * MS));
    }

    #[test]
    fn reserve_refuses_without_side_effects_when_wait_is_too_long() {
        let t0 = Instant::now();
        let mut bucket = TokenBucket::with_burst_at(BandwidthLimit::from_kbps(8), 1000.0, t0);
        assert_eq!(bucket.reserve_at(1000, t0, 100 * MS), Some(Duration::ZERO));
        assert_eq!(bucket.reserve_at(500, t0, 100 * MS), None);
        // Nothing was reserved by the refused call.
        assert_eq!(bucket.reserve_at(100, t0, 200 * MS), Some(100 * MS));
    }

    #[test]
    fn packets_larger_than_the_burst_are_still_paced_not_stuck() {
        let t0 = Instant::now();
        let mut bucket = TokenBucket::with_burst_at(BandwidthLimit::from_kbps(8), 100.0, t0);
        let wait = bucket
            .reserve_at(1500, t0, Duration::from_secs(10))
            .unwrap();
        assert_eq!(wait, Duration::from_secs_f64(1.4));
    }

    #[test]
    fn zero_limit_never_admits() {
        let t0 = Instant::now();
        let mut bucket = TokenBucket::with_burst_at(BandwidthLimit::from_bytes(0), 0.0, t0);
        assert_eq!(bucket.reserve_at(1, t0, Duration::from_secs(60)), None);
    }

    #[test]
    fn update_limit_clamps_tokens() {
        let t0 = Instant::now();
        let mut bucket = TokenBucket::with_burst_at(BandwidthLimit::from_kbps(80), 10_000.0, t0);
        bucket.update_limit(BandwidthLimit::from_kbps(8));
        assert!(bucket.consume_at(1000, t0));
        assert!(!bucket.consume_at(1, t0));
    }
}
