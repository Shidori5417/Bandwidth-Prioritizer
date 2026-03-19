use bwp_core::BandwidthLimit;
use std::time::Instant;

/// A high-performance, lock-free Token Bucket algorithm meant to run
/// synchronously inside the packet interception loop.
pub struct TokenBucket {
    /// Limit being enforced
    limit: BandwidthLimit,
    /// Capacity of the bucket (max burst), typically equivalent to 1 second of limit.
    capacity: f64,
    /// Current available tokens (in bytes)
    tokens: f64,
    /// Last refill timestamp
    last_refill: Instant,
}

impl TokenBucket {
    /// Instantiate a newly filled bucket for the specified limit.
    pub fn new(limit: BandwidthLimit) -> Self {
        let bytes_per_sec = limit.as_bytes_per_sec() as f64;
        Self {
            limit,
            // A common strategy is parsing MTU burst size
            // but setting capacity identical to per-sec limit accommodates most cases safely.
            capacity: bytes_per_sec,
            tokens: bytes_per_sec,
            last_refill: Instant::now(),
        }
    }

    /// Try to consume `amount` tokens (bytes) for a packet.
    /// If there aren't enough tokens, returns false, which means the packet should be DROP/Delay.
    pub fn consume(&mut self, amount: usize) -> bool {
        self.refill();

        let req = amount as f64;
        if self.tokens >= req {
            self.tokens -= req;
            true
        } else {
            false
        }
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

    /// Internal synchronous time-delta calculating logic.
    #[inline(always)]
    fn refill(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();

        let rate = self.limit.as_bytes_per_sec() as f64;
        let generated = elapsed * rate;

        if generated > 0.0 {
            self.tokens = (self.tokens + generated).min(self.capacity);
            self.last_refill = now;
        }
    }
}
