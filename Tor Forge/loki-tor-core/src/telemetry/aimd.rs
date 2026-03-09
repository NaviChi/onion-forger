//! Additive Increase, Multiplicative Decrease (TCP-like Congestion Control)
//! Probes Tor exit limits safely without triggering 429/DOS protections.

#[derive(Debug, Clone)]
pub struct AimdController {
    current_concurrency: usize,
    max_concurrency: usize,
    ss_thresh: usize,
    in_slow_start: bool,
}

impl AimdController {
    pub fn new(max: usize) -> Self {
        Self {
            current_concurrency: 1,
            max_concurrency: max,
            ss_thresh: max / 2, // Start halving at 50% max initially
            in_slow_start: true,
        }
    }

    /// Call when a request succeeds
    pub fn on_success(&mut self) -> usize {
        if self.in_slow_start {
            self.current_concurrency += 1;
            if self.current_concurrency >= self.ss_thresh {
                self.in_slow_start = false;
            }
        } else {
            // Additive increase
            self.current_concurrency += 1; 
        }
        
        if self.current_concurrency > self.max_concurrency {
            self.current_concurrency = self.max_concurrency;
        }
        self.current_concurrency
    }

    /// Call when a connection stalls, drops (e.g. 429), or takes too long
    pub fn on_reject(&mut self) -> usize {
        self.in_slow_start = false;
        self.ss_thresh = (self.current_concurrency / 2).max(1);
        self.current_concurrency = self.ss_thresh;
        self.current_concurrency
    }
    
    pub fn get(&self) -> usize {
        self.current_concurrency
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_aimd_slow_start_and_reject() {
        let mut controller = AimdController::new(10);
        assert_eq!(controller.get(), 1);
        
        // Slow start phase (additive here for simplicity of test, but could be exponential)
        controller.on_success();
        assert_eq!(controller.get(), 2);
        controller.on_success();
        assert_eq!(controller.get(), 3);
        
        // Reject cuts it in half
        controller.on_reject();
        assert_eq!(controller.get(), 1); // 3/2 = 1
        
        // Max limit test
        for _ in 0..20 {
            controller.on_success();
        }
        assert_eq!(controller.get(), 10);
    }
}
