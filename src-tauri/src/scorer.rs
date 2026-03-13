use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Thompson Sampling Multi-Armed Bandit circuit scorer with EKF.
/// Tracks per-circuit performance and computes optimal URL queue assignment using probability distributions.
pub struct CircuitScorer {
    pieces_completed: Vec<AtomicU64>,
    total_bytes: Vec<AtomicU64>,
    total_elapsed_ms: Vec<AtomicU64>,
    global_pieces: AtomicU64,
    capacity: usize,
    // Phase 4.3 (Vanguard): Predictive Kalman Filtering (f64 bits in AtomicU64)
    kalman_x: Vec<AtomicU64>,      // Estimated true latency state
    kalman_p: Vec<AtomicU64>,      // Estimate covariance
    kalman_r: Vec<AtomicU64>,      // Measurement noise variance
    kalman_future: Vec<AtomicU64>, // Predictive state 2 steps ahead
    latency_samples: Vec<AtomicU64>,
}

fn store_f64(atomic: &AtomicU64, val: f64) {
    atomic.store(val.to_bits(), Ordering::Relaxed);
}

fn load_f64(atomic: &AtomicU64) -> f64 {
    let bits = atomic.load(Ordering::Relaxed);
    if bits == 0 {
        Default::default()
    } else {
        f64::from_bits(bits)
    }
}

impl CircuitScorer {
    pub fn new(num_circuits: usize) -> Self {
        CircuitScorer {
            pieces_completed: (0..num_circuits).map(|_| AtomicU64::new(0)).collect(),
            total_bytes: (0..num_circuits).map(|_| AtomicU64::new(0)).collect(),
            total_elapsed_ms: (0..num_circuits).map(|_| AtomicU64::new(0)).collect(),
            global_pieces: AtomicU64::new(0),
            capacity: num_circuits,
            kalman_x: (0..num_circuits).map(|_| AtomicU64::new(0)).collect(),
            kalman_p: (0..num_circuits).map(|_| AtomicU64::new(0)).collect(),
            kalman_r: (0..num_circuits).map(|_| AtomicU64::new(0)).collect(),
            kalman_future: (0..num_circuits).map(|_| AtomicU64::new(0)).collect(),
            latency_samples: (0..num_circuits).map(|_| AtomicU64::new(0)).collect(),
        }
    }

    /// Record a completed HTTP request for a circuit
    pub fn record_piece(&self, cid: usize, bytes: u64, elapsed_ms: u64) {
        if cid < self.capacity {
            self.pieces_completed[cid].fetch_add(1, Ordering::Relaxed);
            self.total_bytes[cid].fetch_add(bytes, Ordering::Relaxed);
            self.total_elapsed_ms[cid].fetch_add(elapsed_ms.max(1), Ordering::Relaxed);
            self.global_pieces.fetch_add(1, Ordering::Relaxed);
            // Update latency EMA
            self.record_latency(cid, elapsed_ms);
        }
    }

    /// Record latency and update Aerospace grade Kalman Filter
    fn record_latency(&self, cid: usize, elapsed_ms: u64) {
        if cid >= self.capacity {
            return;
        }
        self.latency_samples[cid].fetch_add(1, Ordering::Relaxed);

        let latency = elapsed_ms as f64;
        let q = 0.05; // Process drift variance

        let mut x = load_f64(&self.kalman_x[cid]);
        let mut p = load_f64(&self.kalman_p[cid]);
        let mut r = load_f64(&self.kalman_r[cid]);

        if x == 0.0 {
            // Initialization
            x = latency;
            p = 1.0;
            r = 100.0;
        }

        // 1. Predict
        let p_pred = p + q;

        // 2. Dynamic Update Measurement Noise (R)
        let residual = latency - x;
        r = (0.7 * r) + (0.3 * residual * residual).max(1.0);

        // 3. Update Step
        let k = p_pred / (p_pred + r); // Kalman Gain
        x += k * residual;
        p = (1.0 - k) * p_pred;

        // Predict trajectory 2 steps ahead to detect node death before timeout
        let momentum = residual * k;
        let predicted_future = x + (momentum * 2.0);

        store_f64(&self.kalman_x[cid], x);
        store_f64(&self.kalman_p[cid], p);
        store_f64(&self.kalman_r[cid], r);
        store_f64(&self.kalman_future[cid], predicted_future);
    }

    /// Check if a circuit is about to stall using Kalman Predictive Horizon
    pub fn is_degrading(&self, cid: usize) -> bool {
        if cid >= self.capacity {
            return false;
        }
        let samples = self.latency_samples[cid].load(Ordering::Relaxed);
        if samples < 5 {
            return false;
        } // Need enough data

        let x = load_f64(&self.kalman_x[cid]);
        let future = load_f64(&self.kalman_future[cid]);

        if x == 0.0 {
            return false;
        }

        // If the filter predicts an explosion in latency (spiking 2.5x the current stabilized mean)
        future > (x * 2.5)
    }

    /// Compute Thompson Sampling score for a circuit (higher = should get more pieces)
    pub fn thompson_score(&self, cid: usize) -> f64 {
        if cid >= self.capacity {
            return 0.0;
        }
        let n = self.pieces_completed[cid].load(Ordering::Relaxed);
        if n == 0 {
            return f64::MAX; // Untested = infinite score (explore first)
        }

        let total_b = self.total_bytes[cid].load(Ordering::Relaxed) as f64;
        let total_ms = self.total_elapsed_ms[cid].load(Ordering::Relaxed).max(1) as f64;
        let avg_speed = total_b / total_ms; // bytes per ms (mean)

        // The Kalman filter tracks latency. We use its covariance (uncertainty) to drive exploration.
        let mut variance = load_f64(&self.kalman_p[cid]);
        if variance < 0.001 {
            variance = 0.001;
        }

        // Box-Muller transform for normal distribution N(mean, variance) Lock-Free
        let std_dev = variance.sqrt();
        let time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let u1 = (((time ^ (time >> 12)) % 10000) as f64 / 10000.0).max(0.0001);
        let u2 = (((time ^ (time >> 20)) % 10000) as f64 / 10000.0).max(0.0001);

        let z0 = (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos();

        let thompson_scaling_factor = 0.01; // map latency variance to bw speed scale

        avg_speed + (z0 * std_dev * thompson_scaling_factor)
    }

    /// Compute average speed in MB/s for a circuit
    pub fn avg_speed_mbps(&self, cid: usize) -> f64 {
        if cid >= self.capacity {
            return 0.0;
        }
        let total_b = self.total_bytes[cid].load(Ordering::Relaxed) as f64;
        let total_ms = self.total_elapsed_ms[cid].load(Ordering::Relaxed).max(1) as f64;
        (total_b / total_ms) * 1000.0 / 1_048_576.0 // Convert bytes/ms to MB/s
    }

    /// Phase 45: Select the best circuit for the next download chunk.
    /// Uses Thompson Sampling scores with Kalman degradation avoidance.
    pub fn best_circuit_for_url(&self, num_circuits: usize) -> usize {
        let limit = num_circuits.min(self.capacity);
        if limit == 0 {
            return 0;
        }

        let mut best_cid = 0;
        let mut best_score = f64::NEG_INFINITY;
        for cid in 0..limit {
            // Skip degrading circuits
            if self.is_degrading(cid) {
                continue;
            }
            let score = self.thompson_score(cid);
            if score > best_score {
                best_score = score;
                best_cid = cid;
            }
        }
        best_cid
    }

    /// Phase 140: Returns circuit IDs whose measured average speed is above
    /// `threshold_mbps` (e.g. 0.3 MB/s), sorted fastest-first.
    /// Only considers circuits with ≥3 recorded pieces (enough data for
    /// statistically meaningful speed measurement).
    pub fn fast_circuits_above_threshold(
        &self,
        num_circuits: usize,
        threshold_mbps: f64,
    ) -> Vec<(usize, f64)> {
        let limit = num_circuits.min(self.capacity);
        let mut qualified: Vec<(usize, f64)> = (0..limit)
            .filter_map(|cid| {
                let pieces = self.pieces_completed[cid].load(Ordering::Relaxed);
                if pieces < 3 {
                    return None; // Not enough data
                }
                let speed = self.avg_speed_mbps(cid);
                if speed >= threshold_mbps && !self.is_degrading(cid) {
                    Some((cid, speed))
                } else {
                    None
                }
            })
            .collect();
        // Sort by speed descending — fastest circuits first
        qualified.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        qualified
    }

    /// Phase 140: Build a download worker pool from only circuits that meet the
    /// minimum speed threshold. If no circuits qualify, falls back to ALL
    /// non-degrading circuits (prevents starvation on slow networks).
    ///
    /// Returns: Vec of circuit IDs to use for parallel download.
    pub fn select_fast_download_pool(
        &self,
        num_circuits: usize,
        min_speed_mbps: f64,
        max_pool_size: usize,
    ) -> Vec<usize> {
        let fast = self.fast_circuits_above_threshold(num_circuits, min_speed_mbps);
        if !fast.is_empty() {
            return fast.into_iter().map(|(cid, _)| cid).take(max_pool_size).collect();
        }
        // Fallback: no circuits meet threshold — use all non-degrading circuits
        let limit = num_circuits.min(self.capacity);
        (0..limit)
            .filter(|&cid| !self.is_degrading(cid))
            .take(max_pool_size)
            .collect()
    }

    /// Phase 142 (R3): Compute median Kalman-estimated latency across all circuits
    /// with at least 3 samples. Returns milliseconds. Used by adaptive stall threshold.
    /// If no circuits have data, returns 0.0 (caller should use fallback constant).
    pub fn median_latency_ms(&self) -> f64 {
        let mut latencies: Vec<f64> = (0..self.capacity)
            .filter(|&cid| self.latency_samples[cid].load(Ordering::Relaxed) >= 3)
            .map(|cid| load_f64(&self.kalman_x[cid]))
            .filter(|&x| x > 0.0)
            .collect();

        if latencies.is_empty() {
            return 0.0;
        }

        latencies.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let mid = latencies.len() / 2;
        if latencies.len() % 2 == 0 {
            (latencies[mid - 1] + latencies[mid]) / 2.0
        } else {
            latencies[mid]
        }
    }

    /// Phase 142: Compute global aggregate download speed across all circuits (MB/s).
    pub fn global_avg_speed_mbps(&self) -> f64 {
        let total_b: u64 = (0..self.capacity)
            .map(|cid| self.total_bytes[cid].load(Ordering::Relaxed))
            .sum();
        let total_ms: u64 = (0..self.capacity)
            .map(|cid| self.total_elapsed_ms[cid].load(Ordering::Relaxed))
            .sum();
        if total_ms == 0 {
            return 0.0;
        }
        (total_b as f64 / total_ms.max(1) as f64) * 1000.0 / 1_048_576.0
    }

    /// How long a circuit should wait before claiming the next URL target.
    /// Fast circuits: 0ms. Slow circuits: up to 1000ms.
    /// This naturally gives more work to faster circuits.
    pub fn yield_delay(&self, cid: usize) -> Duration {
        if cid >= self.capacity {
            return Duration::ZERO;
        }
        let my_score = self.thompson_score(cid);
        if my_score == f64::MAX {
            return Duration::ZERO;
        } // Untested, no delay

        // Collect scores of all active circuits
        let mut scores: Vec<f64> = (0..self.capacity)
            .filter(|&i| self.pieces_completed[i].load(Ordering::Relaxed) > 0)
            .map(|i| self.thompson_score(i))
            .collect();
        if scores.is_empty() {
            return Duration::ZERO;
        }

        scores.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
        let best = scores.first().copied().unwrap_or(1.0);
        if best <= 0.0 {
            return Duration::ZERO;
        }

        // Ratio: 0.0 (worst) to 1.0 (best)
        let ratio = (my_score / best).clamp(0.0, 1.0);

        // Map: top 50% → 0ms, bottom 50% → 0-1000ms proportional
        if ratio > 0.5 {
            Duration::ZERO
        } else {
            let delay_ms = ((0.5 - ratio) * 2000.0) as u64; // 0-1000ms
            Duration::from_millis(delay_ms.min(1000))
        }
    }
}
