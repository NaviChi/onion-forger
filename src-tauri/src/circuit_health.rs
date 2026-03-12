// ═══════════════════════════════════════════════════════════════════════════════
// Phase 126: Shared CircuitHealth module
// Extracted from DragonForce adapter for cross-adapter reuse.
//
// Memory budget: 12 bytes per circuit (3 × AtomicU32).
//   - EWMA score:     4 bytes  (quality tracking, α=0.3)
//   - CUSUM statistic: 4 bytes  (change-point detection, threshold=2.0)
//   - Latency EWMA:   4 bytes  (adaptive TTFB, α=0.2)
//
// All operations are lock-free via compare_exchange_weak (CAS):
//   - ARM64: LDXR/STXR pair, no fence overhead with Relaxed ordering
//   - x86_64: LOCK CMPXCHG, single instruction
//   - Contention-free in practice (each worker targets a different slot)
// ═══════════════════════════════════════════════════════════════════════════════

use std::sync::atomic::{AtomicU32, Ordering};

/// Per-circuit health tracker with EWMA scoring, CUSUM change-point
/// detection, and latency-based adaptive TTFB.
///
/// # Zero-allocation design
/// All state is packed into 3 `AtomicU32` values (12 bytes total).
/// Float values are stored as raw bits via `f32::to_bits()` / `f32::from_bits()`.
/// The sentinel value `0u32` means "no data" for all three fields.
pub struct CircuitHealth {
    /// EWMA score stored as f32 bits in AtomicU32.
    /// 0.5 = neutral (no data), 1.0 = perfect, 0.0 = dead.
    ewma_bits: AtomicU32,
    /// CUSUM statistic for change-point detection.
    /// When this exceeds CUSUM_THRESHOLD, the circuit has degraded
    /// suddenly and should be repinned immediately.
    cusum_bits: AtomicU32,
    /// EWMA of response latency in milliseconds (f32 bits).
    /// Used for adaptive TTFB: max(3 × ewma_latency, 5000ms).
    latency_ewma_bits: AtomicU32,
}

impl CircuitHealth {
    /// Recency weight for EWMA score — 3× emphasis on recent observations.
    pub const ALPHA: f32 = 0.3;
    /// CUSUM threshold — ~3-4 consecutive failures trigger (4 × 0.55 = 2.2).
    pub const CUSUM_THRESHOLD: f32 = 2.0;
    /// Allowable drift before CUSUM accumulation begins.
    pub const CUSUM_DRIFT: f32 = 0.15;
    /// Smoother for latency EWMA — less volatile than score EWMA.
    pub const LATENCY_ALPHA: f32 = 0.2;

    /// Create a new circuit health tracker with no data (neutral state).
    pub fn new() -> Self {
        Self {
            ewma_bits: AtomicU32::new(0),
            cusum_bits: AtomicU32::new(0),
            latency_ewma_bits: AtomicU32::new(0),
        }
    }

    /// Current EWMA score. Returns 0.5 (neutral) if no data recorded.
    #[inline]
    pub fn score(&self) -> f32 {
        let bits = self.ewma_bits.load(Ordering::Relaxed);
        if bits == 0 { 0.5 } else { f32::from_bits(bits) }
    }

    /// Check if CUSUM has detected a change-point (sudden degradation).
    #[inline]
    pub fn cusum_triggered(&self) -> bool {
        let bits = self.cusum_bits.load(Ordering::Relaxed);
        if bits == 0 { return false; }
        f32::from_bits(bits) >= Self::CUSUM_THRESHOLD
    }

    /// Current raw CUSUM value (for logging/diagnostics).
    #[inline]
    pub fn cusum_value(&self) -> f32 {
        let bits = self.cusum_bits.load(Ordering::Relaxed);
        if bits == 0 { 0.0 } else { f32::from_bits(bits) }
    }

    /// Reset CUSUM after a repin (new circuit = fresh slate).
    #[inline]
    pub fn reset_cusum(&self) {
        self.cusum_bits.store(0, Ordering::Relaxed);
    }

    /// Adaptive TTFB timeout based on latency history.
    /// Returns `max(3 × ewma_latency_ms, 5000)` capped at 25000ms.
    /// Returns 25000ms (conservative) if no latency data recorded.
    #[inline]
    pub fn adaptive_ttfb_ms(&self) -> u64 {
        let bits = self.latency_ewma_bits.load(Ordering::Relaxed);
        if bits == 0 {
            25_000 // No data yet → conservative default
        } else {
            let ewma_ms = f32::from_bits(bits);
            // 3× observed latency, floored at 5s, capped at 25s
            ((ewma_ms * 3.0) as u64).clamp(5_000, 25_000)
        }
    }

    /// Record a successful request (outcome = 1.0).
    #[inline]
    pub fn record_success(&self) {
        self.update(1.0);
    }

    /// Record a failed request (outcome = 0.0).
    #[inline]
    pub fn record_failure(&self) {
        self.update(0.0);
    }

    /// Record response latency for adaptive TTFB computation.
    /// Uses EWMA with α=0.2 for smoother convergence.
    pub fn record_latency(&self, latency_ms: f32) {
        loop {
            let old = self.latency_ewma_bits.load(Ordering::Relaxed);
            let old_val = if old == 0 { latency_ms } else { f32::from_bits(old) };
            let new_val = Self::LATENCY_ALPHA * latency_ms
                + (1.0 - Self::LATENCY_ALPHA) * old_val;
            match self.latency_ewma_bits.compare_exchange_weak(
                old, new_val.to_bits(),
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(_) => continue,
            }
        }
    }

    /// Internal: update both EWMA score and CUSUM statistic.
    fn update(&self, outcome: f32) {
        // ── EWMA score update ──
        loop {
            let old = self.ewma_bits.load(Ordering::Relaxed);
            let old_score = if old == 0 { 0.5 } else { f32::from_bits(old) };
            let new_score = Self::ALPHA * outcome + (1.0 - Self::ALPHA) * old_score;
            match self.ewma_bits.compare_exchange_weak(
                old, new_score.to_bits(),
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(_) => continue, // CAS retry (microsecond convergence)
            }
        }

        // ── CUSUM change-point update ──
        // Accumulates negative deviations from the expected target (0.7).
        // Failures (outcome=0.0) add ~0.55 per event; successes drain it.
        // When cumulative sum ≥ CUSUM_THRESHOLD, circuit has degraded.
        let deviation = (0.7 - outcome) - Self::CUSUM_DRIFT; // positive = bad
        loop {
            let old = self.cusum_bits.load(Ordering::Relaxed);
            let old_cusum = if old == 0 { 0.0f32 } else { f32::from_bits(old) };
            // One-sided CUSUM — only tracks degradation (clamped ≥ 0)
            let new_cusum = (old_cusum + deviation).max(0.0);
            match self.cusum_bits.compare_exchange_weak(
                old, new_cusum.to_bits(),
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(_) => continue,
            }
        }
    }
}

impl Default for CircuitHealth {
    fn default() -> Self {
        Self::new()
    }
}

/// Check if ALL slots in a pool are dead (score ≤ threshold).
/// Used for the CUSUM graduated backoff: when every circuit is down,
/// there's no point cycling — better to back off and wait for recovery.
///
/// Returns `true` if all slots have EWMA score ≤ `dead_threshold`.
#[inline]
pub fn all_slots_dead(health: &[CircuitHealth], dead_threshold: f32) -> bool {
    health.iter().all(|h| h.score() <= dead_threshold)
}

/// Find the best (highest EWMA score) slot index.
/// Returns `(best_index, best_score)`.
#[inline]
pub fn best_slot(health: &[CircuitHealth]) -> (usize, f32) {
    let mut best_idx = 0;
    let mut best_sc = -1.0f32;
    for (i, h) in health.iter().enumerate() {
        let sc = h.score();
        if sc > best_sc {
            best_sc = sc;
            best_idx = i;
        }
    }
    (best_idx, best_sc)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initial_state() {
        let h = CircuitHealth::new();
        assert!((h.score() - 0.5).abs() < 0.001);
        assert!(!h.cusum_triggered());
        assert_eq!(h.adaptive_ttfb_ms(), 25_000);
    }

    #[test]
    fn test_consecutive_failures_trigger_cusum() {
        let h = CircuitHealth::new();
        // 4 consecutive failures should trigger CUSUM (4 × 0.55 = 2.2 > 2.0)
        for _ in 0..4 {
            h.record_failure();
        }
        assert!(h.cusum_triggered(), "CUSUM should trigger after 4 consecutive failures");
    }

    #[test]
    fn test_success_drains_cusum() {
        let h = CircuitHealth::new();
        // 3 failures (CUSUM ≈ 1.65, below threshold)
        for _ in 0..3 {
            h.record_failure();
        }
        assert!(!h.cusum_triggered());
        // 2 successes should drain CUSUM back toward 0
        h.record_success();
        h.record_success();
        assert!(!h.cusum_triggered());
    }

    #[test]
    fn test_adaptive_ttfb_convergence() {
        let h = CircuitHealth::new();
        // Record several 500ms latencies
        for _ in 0..10 {
            h.record_latency(500.0);
        }
        // Should converge to ~5000ms (3 × 500 = 1500, but floor is 5000)
        let ttfb = h.adaptive_ttfb_ms();
        assert_eq!(ttfb, 5_000, "Floor should be 5000ms");
    }

    #[test]
    fn test_adaptive_ttfb_high_latency() {
        let h = CircuitHealth::new();
        // Record several 5000ms latencies
        for _ in 0..10 {
            h.record_latency(5000.0);
        }
        // 3 × 5000 = 15000ms
        let ttfb = h.adaptive_ttfb_ms();
        assert!(ttfb >= 14_000 && ttfb <= 16_000, "Should be near 15000ms, got {}", ttfb);
    }

    #[test]
    fn test_reset_cusum() {
        let h = CircuitHealth::new();
        for _ in 0..5 {
            h.record_failure();
        }
        assert!(h.cusum_triggered());
        h.reset_cusum();
        assert!(!h.cusum_triggered());
    }

    #[test]
    fn test_all_slots_dead() {
        let health: Vec<CircuitHealth> = (0..4).map(|_| {
            let h = CircuitHealth::new();
            for _ in 0..10 {
                h.record_failure();
            }
            h
        }).collect();
        assert!(all_slots_dead(&health, 0.1));
    }

    #[test]
    fn test_best_slot_selection() {
        let health: Vec<CircuitHealth> = (0..4).map(|_| CircuitHealth::new()).collect();
        // Make slot 2 the best
        for _ in 0..5 {
            health[2].record_success();
        }
        for _ in 0..5 {
            health[0].record_failure();
            health[1].record_failure();
            health[3].record_failure();
        }
        let (idx, _score) = best_slot(&health);
        assert_eq!(idx, 2, "Slot 2 should be the best");
    }
}
