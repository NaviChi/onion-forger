use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Instant;
use std::sync::RwLock;

/// Application-Layer BBR (Bottleneck Bandwidth and RTT) congestion controller.
/// Paces active circuit count instantly to the bottleneck bandwidth rather than linear AIMD scaling.
pub struct BbrController {
    active: AtomicUsize,
    max: usize,
    min: usize,
    
    // Track bytes delivered in a timing window
    window_start: RwLock<Instant>,
    delivered_bytes: AtomicU64,
    
    // Sliding max bandwidth (bytes per ms)
    max_bw_bps: AtomicU64,
    // Sliding min RTT (ms)
    min_rtt_ms: AtomicU64,
}

impl BbrController {
    pub fn new(initial: usize, max: usize) -> Self {
        BbrController {
            active: AtomicUsize::new(initial),
            max,
            min: 1,
            window_start: RwLock::new(Instant::now()),
            delivered_bytes: AtomicU64::new(0),
            max_bw_bps: AtomicU64::new(10), // Base default (10 bytes/ms = 10KB/s)
            min_rtt_ms: AtomicU64::new(1000), // Default high RTT for Tor (1000ms)
        }
    }

    /// Update with actual bytes delivered and RTT. 
    /// If payload is unknown or metadata-only, use est_bytes.
    pub fn on_success(&self, bytes: u64, rtt_ms: u64) {
        let rtt = rtt_ms.max(1);
        
        let current_min_rtt = self.min_rtt_ms.load(Ordering::Relaxed);
        if rtt < current_min_rtt {
            // Update min RTT if we found a better route
            // To prevent stale min_RTT from locking us in forever, we drift average slightly
            self.min_rtt_ms.store(rtt, Ordering::Relaxed);
        } else {
            // Decay min_rtt to allow discovery of new higher floors if target degrades
            self.min_rtt_ms.fetch_add(1.max(rtt / 1000), Ordering::Relaxed);
        }

        // Add to window
        self.delivered_bytes.fetch_add(bytes, Ordering::Relaxed);
        
        let now = Instant::now();
        let elapsed_ms = {
            let start = self.window_start.read().unwrap();
            now.duration_since(*start).as_millis()
        };

        // Window processing every 1000ms
        if elapsed_ms >= 1000 {
            // Upgrade lock to reset window
            if let Ok(mut start) = self.window_start.try_write() {
                let final_elapsed = now.duration_since(*start).as_millis() as u64;
                if final_elapsed >= 1000 {
                    let final_delivered = self.delivered_bytes.swap(0, Ordering::Relaxed);
                    let current_bw = final_delivered / final_elapsed.max(1);
                    
                    let mut max_bw = self.max_bw_bps.load(Ordering::Relaxed);
                    if current_bw > max_bw {
                        self.max_bw_bps.store(current_bw, Ordering::Relaxed);
                        max_bw = current_bw;
                    } else {
                        // Decay max_bw to handle changing network conditions (circuit dying)
                        max_bw = (max_bw * 9 / 10).max(10);
                        self.max_bw_bps.store(max_bw, Ordering::Relaxed);
                    }

                    *start = now;
                    self.recalculate_concurrency(max_bw, self.min_rtt_ms.load(Ordering::Relaxed));
                }
            }
        }
    }

    fn recalculate_concurrency(&self, max_bw_bps: u64, min_rtt_ms: u64) {
        // BBR fundamentally limits inflight to BDP = BtlBw * RTprop
        let bdp_bytes = max_bw_bps * min_rtt_ms;
        
        // Assume an average web request response is 32KB for Tor operations
        let avg_req_size = 32768; 
        
        let target_concurrency = (bdp_bytes / avg_req_size) as usize;
        let clamped = target_concurrency.clamp(self.min, self.max);
        
        let current = self.active.load(Ordering::Relaxed);
        let next = if clamped > current {
            clamped // Snap up instantly (HFT style BBR probe block)
        } else {
            (current * 3 / 4).max(clamped) // Multiplicatively drift down smoothly
        };
        
        self.active.store(next, Ordering::Relaxed);
    }

    /// Default success ping used by endpoints lacking immediate telemetry bytes
    pub fn on_success_blind(&self) {
        self.on_success(65536, 1000); // Emulate a reasonable baseline ping
    }

    pub fn on_timeout(&self) {
        let current = self.active.load(Ordering::Relaxed);
        let new_val = (current * 3 / 4).max(self.min);
        self.active.store(new_val, Ordering::Relaxed);
    }

    pub fn on_reject(&self) {
        let current = self.active.load(Ordering::Relaxed);
        let new_val = (current / 2).max(self.min);
        self.active.store(new_val, Ordering::Relaxed);
    }

    pub fn current_active(&self) -> usize {
        self.active.load(Ordering::Relaxed)
    }
}
