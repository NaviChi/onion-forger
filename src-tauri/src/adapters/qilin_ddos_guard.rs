//! Phase 76: Enhanced DDoS Guard with EKF covariance-driven jitter,
//! BBR-inspired min-RTT pacing, and quarantine queue for throttled URLs.
//!
//! Prevention Rules:
//!   PR-THROTTLE-JITTER-001: Never fire back-to-back requests from same circuit
//!                           within CRAWLI_QILIN_MIN_REQUEST_SPACING_MS (default 200ms).
//!   PR-THROTTLE-QUARANTINE-001: On 403/400/429/503, enqueue the URL into a quarantine
//!                           queue with a jittered unlock time instead of sleeping the worker.

use std::time::{Duration, Instant};

/// EKF-inspired covariance tracker for request pacing.
/// Tracks the "heat" of the target's DDoS gateway and produces
/// adaptive delays that scale with observed error severity.
#[derive(Clone, Debug)]
pub struct DdosGuard {
    /// EKF covariance estimate — higher = more aggressive gateway
    ekf_covariance: f64,
    /// BBR-inspired minimum observed RTT (in ms) for baseline pacing
    bbr_min_rtt_ms: f64,
    /// Exponential weighted moving average of recent response codes
    ewma_threat_score: f64,
    /// Consecutive throttle-class responses (403/400/429/503)
    consecutive_throttles: u32,
    /// Timestamp of last throttle for cooldown tracking
    last_throttle: Instant,
    /// Timestamp of last successful request for spacing enforcement
    last_request: Instant,
    /// Minimum spacing between requests (configurable via env)
    min_spacing_ms: u64,
}

/// Outcome of recording a response through the DDoS guard.
#[derive(Debug)]
pub enum DdosOutcome {
    /// Request succeeded, apply this optional inter-request spacing delay
    Proceed(Option<Duration>),
    /// Request was throttled — quarantine the URL for this duration
    /// The worker should NOT sleep; instead push to quarantine queue
    Quarantine(Duration),
}

impl DdosGuard {
    pub fn new() -> Self {
        let min_spacing_ms = std::env::var("CRAWLI_QILIN_MIN_REQUEST_SPACING_MS")
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .unwrap_or(200)
            .clamp(50, 2000);

        Self {
            ekf_covariance: 0.3,
            bbr_min_rtt_ms: 3000.0, // Conservative initial estimate for Tor
            ewma_threat_score: 0.0,
            consecutive_throttles: 0,
            last_throttle: Instant::now(),
            last_request: Instant::now(),
            min_spacing_ms,
        }
    }

    /// Update BBR min-RTT estimate from observed successful request latency
    pub fn update_rtt(&mut self, rtt_ms: u64) {
        let rtt = rtt_ms as f64;
        if rtt > 0.0 && rtt < self.bbr_min_rtt_ms {
            // BBR: track minimum RTT with slow decay
            self.bbr_min_rtt_ms = self.bbr_min_rtt_ms * 0.95 + rtt * 0.05;
        } else if rtt > 0.0 {
            // Slow upward drift to track network degradation
            self.bbr_min_rtt_ms = self.bbr_min_rtt_ms * 0.99 + rtt * 0.01;
        }
    }

    /// Record an HTTP response and return the appropriate outcome.
    /// On throttle codes (403/400/429/503): returns Quarantine with jittered duration.
    /// On success: returns Proceed with optional inter-request spacing.
    pub fn record_response(&mut self, status: u16) -> DdosOutcome {
        let threat_weight = match status {
            403 | 400 => 4.0,                          // DDoS gateway block — most severe
            429 => 3.5,                                // Explicit rate limit
            503 => 3.0,                                // Service unavailable (nginx overload)
            404 => 0.5,                                // Not found — neutral
            _ if (200..300).contains(&status) => -0.3, // Success reduces threat
            _ => 0.2,                                  // Unknown — slightly cautious
        };

        // EKF covariance update: Kalman-style innovation
        // Higher covariance = more uncertainty = wider jitter bands
        let innovation = threat_weight - self.ewma_threat_score;
        let kalman_gain = self.ekf_covariance / (self.ekf_covariance + 1.0);
        self.ewma_threat_score += kalman_gain * innovation;
        self.ekf_covariance = (1.0 - kalman_gain) * self.ekf_covariance + 0.05;
        // Clamp covariance to prevent runaway
        self.ekf_covariance = self.ekf_covariance.clamp(0.05, 5.0);

        let is_throttle = matches!(status, 403 | 400 | 429 | 503);

        if is_throttle {
            self.consecutive_throttles = self.consecutive_throttles.saturating_add(1);
            self.last_throttle = Instant::now();

            // Quarantine duration: EKF covariance × BBR min-RTT × escalation factor
            // This produces quarantine times that adapt to both network conditions
            // AND the aggressiveness of the target's DDoS gateway.
            let base_quarantine_ms = self.ekf_covariance * self.bbr_min_rtt_ms * 0.5;
            let escalation = (self.consecutive_throttles as f64).sqrt().clamp(1.0, 4.0);
            let quarantine_ms = (base_quarantine_ms * escalation).clamp(500.0, 15_000.0) as u64;

            // Add deterministic jitter based on consecutive count to desynchronize workers
            let jitter_ms = (self.consecutive_throttles as u64 * 137) % 500;
            let total_quarantine = Duration::from_millis(quarantine_ms + jitter_ms);

            DdosOutcome::Quarantine(total_quarantine)
        } else {
            // Success path — reset throttle streak
            if self.consecutive_throttles > 0 {
                // Graduated decay: don't instantly reset, decay by half
                self.consecutive_throttles /= 2;
            }

            // Inter-request spacing: enforce minimum + EKF-weighted jitter
            let elapsed_since_last = self.last_request.elapsed();
            self.last_request = Instant::now();

            let min_spacing = Duration::from_millis(self.min_spacing_ms);
            if elapsed_since_last < min_spacing {
                // PR-THROTTLE-JITTER-001: enforce minimum spacing
                let remaining = min_spacing - elapsed_since_last;
                // Add EKF jitter: higher threat score = more spacing
                let jitter_ms = (self.ewma_threat_score.max(0.0) * 30.0).clamp(0.0, 200.0) as u64;
                DdosOutcome::Proceed(Some(remaining + Duration::from_millis(jitter_ms)))
            } else if self.ewma_threat_score > 1.0 {
                // Elevated threat: add proportional spacing even when min is met
                let threat_spacing_ms = (self.ewma_threat_score * 40.0).clamp(0.0, 300.0) as u64;
                DdosOutcome::Proceed(Some(Duration::from_millis(threat_spacing_ms)))
            } else {
                DdosOutcome::Proceed(None) // Clean path — no delay needed
            }
        }
    }

    /// Returns the current EKF covariance (useful for telemetry)
    pub fn covariance(&self) -> f64 {
        self.ekf_covariance
    }

    /// Returns the current threat score (useful for telemetry)
    pub fn threat_score(&self) -> f64 {
        self.ewma_threat_score
    }

    /// Returns seconds since last throttle event
    pub fn seconds_since_last_throttle(&self) -> u64 {
        self.last_throttle.elapsed().as_secs()
    }

    /// Legacy compatibility: returns Option<Duration> for existing call sites.
    /// Converts DdosOutcome into the old-style delay.
    /// Prefer using record_response() directly for new code.
    pub fn record_response_legacy(&mut self, status: u16) -> Option<Duration> {
        match self.record_response(status) {
            DdosOutcome::Proceed(delay) => delay,
            DdosOutcome::Quarantine(duration) => Some(duration),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_reduces_threat_score() {
        let mut guard = DdosGuard::new();
        // Prime with a throttle
        let _ = guard.record_response(503);
        assert!(guard.ewma_threat_score > 0.0);
        // Several successes should reduce threat
        for _ in 0..10 {
            let _ = guard.record_response(200);
        }
        assert!(
            guard.ewma_threat_score < 0.5,
            "Threat score should decay: {}",
            guard.ewma_threat_score
        );
    }

    #[test]
    fn throttle_produces_quarantine() {
        let mut guard = DdosGuard::new();
        match guard.record_response(503) {
            DdosOutcome::Quarantine(d) => {
                assert!(
                    d.as_millis() >= 500,
                    "Quarantine should be at least 500ms: {}ms",
                    d.as_millis()
                );
            }
            other => panic!("Expected Quarantine, got {:?}", other),
        }
    }

    #[test]
    fn consecutive_throttles_escalate_quarantine() {
        let mut guard = DdosGuard::new();
        let first = match guard.record_response(429) {
            DdosOutcome::Quarantine(d) => d,
            _ => panic!("Expected Quarantine"),
        };
        let second = match guard.record_response(429) {
            DdosOutcome::Quarantine(d) => d,
            _ => panic!("Expected Quarantine"),
        };
        let third = match guard.record_response(429) {
            DdosOutcome::Quarantine(d) => d,
            _ => panic!("Expected Quarantine"),
        };
        // Quarantine duration should escalate with consecutive throttles
        assert!(
            third >= second && second >= first,
            "Escalation broken: {:?} vs {:?} vs {:?}",
            first,
            second,
            third
        );
    }

    #[test]
    fn rtt_update_tracks_minimum() {
        let mut guard = DdosGuard::new();
        guard.update_rtt(2000);
        guard.update_rtt(1500);
        guard.update_rtt(5000); // Slow RTT should barely move min
        assert!(
            guard.bbr_min_rtt_ms < 3000.0,
            "BBR min RTT not tracking: {}",
            guard.bbr_min_rtt_ms
        );
    }
}
