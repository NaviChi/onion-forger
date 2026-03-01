use sha2::{Digest, Sha256};
use std::collections::HashMap;

/// Phase 5 (Vanguard): Byzantine Fault Tolerance (BFT) Quorum Slicing
/// Implements deterministic consensus for highly sensitive or untrusted payload chunks.
pub struct BftQuorum;

impl BftQuorum {
    /// Validates a payload chunk by comparing it to multiple identical fetches
    /// from disjoint Tor circuits. Achieves BFT consensus by calculating SHA-256
    /// hashes and voting. Removes and detects malicious payloads injected by compromised exit relays.
    ///
    /// The quorum must have theoretically at least `3f + 1` nodes to tolerate `f` byzantine faults.
    pub fn achieve_consensus(payloads: Vec<Vec<u8>>) -> Result<Vec<u8>, String> {
        if payloads.is_empty() {
            return Err("Quorum is empty, cannot achieve consensus.".to_string());
        }

        let mut hash_votes: HashMap<String, usize> = HashMap::new();
        let mut hash_to_payload: HashMap<String, Vec<u8>> = HashMap::new();

        for payload in payloads {
            let mut hasher = Sha256::new();
            hasher.update(&payload);
            let hex_hash = format!("{:x}", hasher.finalize());

            *hash_votes.entry(hex_hash.clone()).or_insert(0) += 1;
            hash_to_payload.entry(hex_hash).or_insert(payload);
        }

        // Find the majority hash
        let mut best_hash = String::new();
        let mut max_votes = 0;

        for (hash, votes) in &hash_votes {
            if *votes > max_votes {
                max_votes = *votes;
                best_hash = hash.clone();
            }
        }

        let total_nodes = hash_votes.values().sum::<usize>();
        let threshold = (total_nodes / 2) + 1;

        if max_votes >= threshold {
            if let Some(valid_payload) = hash_to_payload.remove(&best_hash) {
                return Ok(valid_payload);
            }
        }

        Err(format!(
            "BFT Quorum Failure: Consensus not met. Top match had {}/{} votes. Compromised exit nodes detected.",
            max_votes, total_nodes
        ))
    }
}
