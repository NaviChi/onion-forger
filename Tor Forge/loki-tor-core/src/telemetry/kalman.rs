/// A 1D Kalman Filter to track circuit latency and throughput, mathematically predicting stalls.
#[derive(Debug, Clone)]
pub struct CircuitKalmanFilter {
    // State estimate (e.g., latency ms or speed)
    pub x: f64,
    // Estimate uncertainty
    pub p: f64,
    // Process noise (how fast the true state changes)
    pub q: f64,
    // Measurement noise (how noisy the measurements are)
    pub r: f64,
}

impl Default for CircuitKalmanFilter {
    fn default() -> Self {
        Self {
            x: 0.0,
            p: 1.0,
            q: 0.1,    // Expect some natural darknet variance
            r: 50.0,   // High measurement noise (Tor connections fluctuate wildly)
        }
    }
}

impl CircuitKalmanFilter {
    pub fn new(initial_estimate: f64, process_noise: f64, measurement_noise: f64) -> Self {
        Self {
            x: initial_estimate,
            p: 1.0,
            q: process_noise,
            r: measurement_noise,
        }
    }

    /// Update the filter with a new measurement and return the new prediction
    pub fn update(&mut self, measurement: f64) -> f64 {
        // Prediction update
        self.p += self.q;

        // Measurement update
        let k = self.p / (self.p + self.r); // Kalman Gain
        self.x += k * (measurement - self.x);
        self.p *= 1.0 - k;

        self.x // Mathematical state estimate (smoothed)
    }
    
    /// Predict state ahead without a measurement
    pub fn predict(&self) -> f64 {
        self.x
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kalman_filter_smoothing() {
        let mut kf = CircuitKalmanFilter::new(500.0, 0.1, 50.0);
        
        // Inject a spike (noisy Tor latency measurement)
        let smoothed_1 = kf.update(1000.0);
        assert!(smoothed_1 > 500.0 && smoothed_1 < 1000.0); // It shouldn't jump fully to 1000
        
        // Inject good values
        kf.update(500.0);
        kf.update(500.0);
        kf.update(450.0);
        
        let final_smooth = kf.update(480.0);
        // It should stabilize close to 450-500
        assert!(final_smooth > 400.0 && final_smooth < 600.0);
    }
}
