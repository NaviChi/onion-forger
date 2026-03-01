use std::sync::atomic::{AtomicUsize, Ordering};

/// AIMD (Additive Increase, Multiplicative Decrease) concurrency controller.
/// Dynamically adjusts active circuit count based on server response to avoid DDoS rate limits.
pub struct AimdController {
    active: AtomicUsize,
    max: usize,
    min: usize,
    consec_success: AtomicUsize,
}

impl AimdController {
    pub fn new(initial: usize, max: usize) -> Self {
        AimdController {
            active: AtomicUsize::new(initial),
            max,
            min: 1,
            consec_success: AtomicUsize::new(0),
        }
    }

    /// Call on successful HTTP crawl request
    pub fn on_success(&self) {
        let consec = self.consec_success.fetch_add(1, Ordering::Relaxed);
        // Additive increase: +1 circuit every 20 consecutive successes
        if consec > 0 && consec % 20 == 0 {
            let current = self.active.load(Ordering::Relaxed);
            if current < self.max {
                self.active.store(current + 1, Ordering::Relaxed);
            }
        }
    }

    /// Call on server rejection (429, 503, connection refused)
    pub fn on_reject(&self) {
        self.consec_success.store(0, Ordering::Relaxed);
        // Multiplicative decrease: halve active circuits immediately
        let current = self.active.load(Ordering::Relaxed);
        let new_val = (current / 2).max(self.min);
        self.active.store(new_val, Ordering::Relaxed);
    }

    /// Call on API timeout (milder decrease)
    pub fn on_timeout(&self) {
        self.consec_success.store(0, Ordering::Relaxed);
        let current = self.active.load(Ordering::Relaxed);
        let new_val = (current * 3 / 4).max(self.min);
        self.active.store(new_val, Ordering::Relaxed);
    }

    /// Check if this daemon should be allowed to fire a request
    pub fn should_be_active(&self, circuit_rank: usize) -> bool {
        circuit_rank < self.active.load(Ordering::Relaxed)
    }

    pub fn current_active(&self) -> usize {
        self.active.load(Ordering::Relaxed)
    }
}
