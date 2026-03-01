#[derive(Debug, Clone)]
pub struct KalmanFilter {
    /// Estimated state (e.g. latency in ms or bandwidth in bps)
    pub x: f64,
    /// Error covariance (uncertainty)
    pub p: f64,
    /// Process noise covariance (expected natural variance)
    pub q: f64,
    /// Measurement noise covariance (expected measurement error)
    pub r: f64,
}

impl KalmanFilter {
    /// Initializes a new Kalman Filter.
    /// `q`: Process noise (e.g. 1e-5 for slow changes, 1e-1 for fast)
    /// `r`: Measurement noise (e.g. 0.1 for high confidence, 10.0 for noisy Tor nodes)
    pub fn new(q: f64, r: f64, initial_value: f64) -> Self {
        Self {
            x: initial_value,
            p: 1.0,  // Initial uncertainty
            q,
            r,
        }
    }

    /// Feeds a new noisy measurement into the filter and returns the newly predicted true state.
    pub fn update(&mut self, measurement: f64) -> f64 {
        // 1. Prediction step
        self.p += self.q;

        // 2. Kalman Gain computation
        let k = self.p / (self.p + self.r);

        // 3. Update estimate with measurement
        self.x += k * (measurement - self.x);

        // 4. Update error covariance
        self.p *= 1.0 - k;

        self.x
    }

    /// Predicts the next state without adopting a new measurement.
    pub fn predict(&self) -> f64 {
        self.x
    }
}
