use std::time::Instant;
use tokio::time::Duration;

#[derive(Clone)]
pub struct DdosGuard {
    last_403: Instant,
    consecutive_403: u32,
    ekf_score: f64,
}

impl DdosGuard {
    pub fn new() -> Self {
        Self {
            last_403: Instant::now(),
            consecutive_403: 0,
            ekf_score: 1.0,
        }
    }

    pub fn record_response(&mut self, status: u16) -> Option<Duration> {
        let weight = match status {
            403 | 400 | 404 => 3.0, // DDoS pattern
            429 | 503 => 2.0,       // Rate Limit pattern
            _ => 0.5,               // Success/Normal
        };

        // EKF-style covariance continuous drift updates
        self.ekf_score = self.ekf_score * 0.7 + weight * 0.3;

        // Predictive backoff scaling based on severity
        if status == 403 || status == 400 || status == 404 || status == 429 {
            self.consecutive_403 += 1;
            self.last_403 = Instant::now();
            let delay_ms = (80.0 * self.ekf_score).clamp(50.0, 500.0) as u64;
            Some(Duration::from_millis(
                delay_ms * self.consecutive_403.min(5) as u64,
            ))
        } else {
            self.consecutive_403 = 0;
            if self.ekf_score > 0.6 {
                let bbr_delay_ms = (12.0 * self.ekf_score).clamp(0.0, 80.0) as u64;
                Some(Duration::from_millis(bbr_delay_ms))
            } else {
                None // NATURAL BLASTING!
            }
        }
    }
}
