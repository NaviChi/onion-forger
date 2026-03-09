#[allow(unused_imports)]
use ring::digest::{Context, Digest, SHA256};
use std::collections::HashMap;

/// Byzantine Fault Tolerance (BFT) verification for high-value downloads.
/// Routes the same payload through 3 isolated circuits and compares hashes.
#[derive(Debug)]
pub struct BftVerifier {
    hashes: Vec<Vec<u8>>,
    min_validators: usize,
}

impl BftVerifier {
    pub fn new(min_validators: usize) -> Self {
        Self { hashes: Vec::new(), min_validators }
    }

    /// Add a completed download's hash
    pub fn add_hash(&mut self, hash: &[u8]) {
        self.hashes.push(hash.to_vec());
    }

    /// Check if we have a quorum (majority agreement). Returns the agreed hash if true.
    pub fn check_quorum(&self) -> Option<Vec<u8>> {
        // Enforce the minimum number of validators before consensus
        if self.hashes.len() < self.min_validators {
            return None;
        }

        let mut counts: HashMap<Vec<u8>, usize> = HashMap::new();
        for h in &self.hashes {
            *counts.entry(h.clone()).or_insert(0) += 1;
        }

        // We need a majority (e.g., 2 out of 3)
        let threshold = (self.hashes.len() / 2) + 1;
        for (hash, count) in counts {
            if count >= threshold {
                return Some(hash);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bft_quorum_consensus() {
        let mut bft = BftVerifier::new(3); // Wait for 3 nodes at least
        
        // 1st node returns payload A
        bft.add_hash(&[1, 2, 3, 4]);
        assert_eq!(bft.check_quorum(), None); // No majority yet
        
        // 2nd node returns modified payload (Malicious Injection!)
        bft.add_hash(&[9, 9, 9, 9]);
        assert_eq!(bft.check_quorum(), None); // Conflicting, no majority
        
        // 3rd node returns payload A. Quorum (2 out of 3) reached!
        bft.add_hash(&[1, 2, 3, 4]);
        assert_eq!(bft.check_quorum(), Some(vec![1, 2, 3, 4]));
    }
}
