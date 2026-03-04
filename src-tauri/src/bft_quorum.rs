use sha2::{Digest, Sha256};
use std::collections::HashMap;

/// Phase 5 (Vanguard): Byzantine Fault Tolerance (BFT) Quorum Slicing
/// Implements deterministic consensus for highly sensitive or untrusted payload chunks.
pub struct BftQuorum;

const MERKLE_CHUNK_SIZE: usize = 262_144; // 256KB logical blocks

impl BftQuorum {
    /// Validates payloads using Merkle-Tree inspired sub-block BFT consensus.
    /// Instead of nullifying an entire 50MB payload because one exit node injected
    /// a byte, we divide the payload into 256KB logical pages. We vote on every
    /// page across all payloads to synthesize a perfect 'Frankenstein' payload.
    pub fn achieve_consensus(payloads: Vec<Vec<u8>>) -> Result<Vec<u8>, String> {
        if payloads.is_empty() {
            return Err("Quorum is empty, cannot achieve consensus.".to_string());
        }

        let total_nodes = payloads.len();
        let threshold = (total_nodes / 2) + 1;
        
        let max_len = payloads.iter().map(|p| p.len()).max().unwrap_or(0);
        if max_len == 0 {
            return Ok(Vec::new());
        }

        let num_chunks = max_len.div_ceil(MERKLE_CHUNK_SIZE);
        let mut synthesized_payload = Vec::with_capacity(max_len);

        for chunk_idx in 0..num_chunks {
            let start = chunk_idx * MERKLE_CHUNK_SIZE;
            let mut hash_votes: HashMap<String, usize> = HashMap::new();
            let mut hash_to_chunk: HashMap<String, Vec<u8>> = HashMap::new();

            for payload in &payloads {
                if payload.len() <= start { continue; } // Byzantine truncation
                let end = (start + MERKLE_CHUNK_SIZE).min(payload.len());
                let chunk_data = &payload[start..end];

                let mut hasher = Sha256::new();
                hasher.update(chunk_data);
                let hex_hash = format!("{:x}", hasher.finalize());

                *hash_votes.entry(hex_hash.clone()).or_insert(0) += 1;
                hash_to_chunk.entry(hex_hash).or_insert_with(|| chunk_data.to_vec());
            }

            let mut best_hash = String::new();
            let mut max_votes = 0;

            for (hash, votes) in &hash_votes {
                if *votes > max_votes {
                    max_votes = *votes;
                    best_hash = hash.clone();
                }
            }

            if max_votes >= threshold {
                if let Some(mut valid_chunk) = hash_to_chunk.remove(&best_hash) {
                    synthesized_payload.append(&mut valid_chunk);
                }
            } else {
                return Err(format!(
                    "Merkle BFT Quorum Failure at 256KB chunk {}: Consensus not met. Top match had {}/{} votes. Compromised exit nodes detected.",
                    chunk_idx, max_votes, total_nodes
                ));
            }
        }

        Ok(synthesized_payload)
    }
}
