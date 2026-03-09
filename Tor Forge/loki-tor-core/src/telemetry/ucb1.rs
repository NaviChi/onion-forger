use std::collections::HashMap;
use std::time::Duration;
use serde::{Serialize, Deserialize};

/// UCB1 (Upper Confidence Bound) algorithm for Circuit Selection
/// Formula: UCB1 = Average Speed + C * sqrt(ln(Total Pieces) / Circuit Pieces)
pub struct Ucb1Scorer {
    pub total_operations: u64,
    pub exploration_factor: f64,
    pub stats: HashMap<usize, CircuitStats>,
    pub db: Option<sled::Db>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CircuitStats {
    pub operations: u64,
    pub cumulative_latency_ms: u64,
}

impl Ucb1Scorer {
    pub fn new(exploration_factor: f64, db_path: Option<&str>) -> Self {
        let mut total_operations = 0;
        let mut stats = HashMap::new();
        let mut db = None;

        // WAL Integration: Load from sled db if provided
        if let Some(path) = db_path {
            if let Ok(database) = sled::open(path) {
                for next in database.iter() {
                    if let Ok((k, v)) = next {
                        if let (Ok(circuit_id), Ok(stat)) = (
                            std::str::from_utf8(&k).map(|s| s.parse::<usize>().unwrap_or(0)),
                            serde_json::from_slice::<CircuitStats>(&v)
                        ) {
                            total_operations += stat.operations;
                            stats.insert(circuit_id, stat);
                        }
                    }
                }
                db = Some(database);
                tracing::info!("Restored {} circuit stats from WAL.", stats.len());
            }
        }

        Self {
            total_operations,
            exploration_factor,
            stats,
            db,
        }
    }

    /// Safely flush a specific circuit's stats to the WAL
    fn flush_to_wal(&self, circuit_id: usize, stat: &CircuitStats) {
        if let Some(database) = &self.db {
            if let Ok(serialized) = serde_json::to_vec(stat) {
                let _ = database.insert(circuit_id.to_string().as_bytes(), serialized);
                // Flush asynchronously or periodically in a real implementation
                let _ = database.flush(); 
            }
        }
    }

    /// Record a completed operation (like a downloaded chunk) on a specific circuit
    pub fn record_success(&mut self, circuit_id: usize, latency_ms: u64) {
        self.total_operations += 1;
        let stat_clone = {
            let stat = self.stats.entry(circuit_id).or_default();
            stat.operations += 1;
            stat.cumulative_latency_ms += latency_ms;
            stat.clone()
        };
        self.flush_to_wal(circuit_id, &stat_clone);
    }

    /// Add a penalty to a circuit for failing or timing out
    pub fn record_failure(&mut self, circuit_id: usize) {
        let stat_clone = {
            let stat = self.stats.entry(circuit_id).or_default();
            // heavy penalty: assume it took 10 seconds of latency
            stat.cumulative_latency_ms += 10_000;
            // Usually, UCB1 assumes operations are attempts.
            stat.operations += 1;
            stat.clone()
        };
        self.total_operations += 1;
        self.flush_to_wal(circuit_id, &stat_clone);
    }

    /// Calculates the UCB1 score. Higher is better.
    pub fn score(&self, circuit_id: usize) -> f64 {
        if let Some(stat) = self.stats.get(&circuit_id) {
            if stat.operations == 0 {
                return f64::MAX; // Always explore un-tried circuits first
            }
            
            let avg_latency = stat.cumulative_latency_ms as f64 / stat.operations as f64;
            // We want lower latency to be a higher score, so we invert or subtract.
            // Let's use speed = 1.0 / (avg_latency + 1.0)
            let speed = 1000.0 / (avg_latency + 1.0);
            
            let exploration = self.exploration_factor * 
                ((self.total_operations as f64).ln() / stat.operations as f64).sqrt();
            
            speed + exploration
        } else {
            f64::MAX // If unknown, absolute priority to explore
        }
    }

    /// Returns the best circuit ID to use next, based on highest UCB1 score
    pub fn select_best_circuit(&self, available_circuits: &[usize]) -> Option<usize> {
        let mut best = None;
        let mut best_score = f64::MIN;

        for &cid in available_circuits {
            let s = self.score(cid);
            if s > best_score {
                best_score = s;
                best = Some(cid);
            }
        }
        best
    }

    /// Calculates artificial delay for slower circuits to throttle them
    pub fn yield_delay(&self, circuit_id: usize, best_latency_ms: u64) -> Duration {
        if let Some(stat) = self.stats.get(&circuit_id) {
            if stat.operations > 0 {
                let avg_latency = stat.cumulative_latency_ms / stat.operations;
                if avg_latency > best_latency_ms * 2 {
                    // It's a slow circuit, yield it to delay
                    let delay = (avg_latency - best_latency_ms * 2).min(2000);
                    return Duration::from_millis(delay);
                }
            }
        }
        Duration::from_millis(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ucb1_scoring() {
        let mut scorer = Ucb1Scorer::new(2.0, None);
        
        // Never used circuits get MAX priority
        assert_eq!(scorer.score(1), f64::MAX);
        
        // Circuit 1 succeeds fast (10ms)
        scorer.record_success(1, 10);
        // Circuit 2 succeeds slow (5000ms)
        scorer.record_success(2, 5000);
        
        let score_fast = scorer.score(1);
        let score_slow = scorer.score(2);
        
        assert!(score_fast > score_slow, "Fast circuit should have higher score");
        
        // Select best
        let available = vec![1, 2, 3]; // 3 is untouched
        let best = scorer.select_best_circuit(&available);
        assert_eq!(best, Some(3), "Should pick completely exploreable option 3 first");
    }
}
