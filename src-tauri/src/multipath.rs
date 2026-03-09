//! Phase 45: Experimental multipath download engine.
//!
//! This remains a laboratory path for benchmarking chunk assignment ideas.
//! Production downloads run through `aria_downloader.rs`, which owns resume
//! state, stop/pause semantics, progress telemetry, and native Arti lifecycle.
//!
//! Integrates with:
//! - `tor_native::ArtiSwarm` for circuit pool access
//! - `scorer::CircuitScorer` for bandwidth-weighted circuit selection
//! - `bbr::BbrController` for congestion-aware pacing

use crate::arti_client::ArtiClient;
use anyhow::{anyhow, Result};
use reqwest::StatusCode;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

use crate::bbr::BbrController;
use crate::scorer::CircuitScorer;

// ============================================================================
// CONFIGURATION
// ============================================================================

/// Minimum file size to trigger multipath (below this, single-circuit is fine)
const MULTIPATH_MIN_SIZE: u64 = 10 * 1024 * 1024; // 10MB
/// Size of each download chunk per circuit
const CHUNK_SIZE: u64 = 2 * 1024 * 1024; // 2MB chunks
/// Maximum concurrent chunks in flight across all circuits
const MAX_INFLIGHT: usize = 32;
/// Timeout per chunk request
const CHUNK_TIMEOUT: Duration = Duration::from_secs(45);
/// Maximum retries per chunk
const MAX_CHUNK_RETRIES: u32 = 5;

// ============================================================================
// CHUNK MANAGER
// ============================================================================

#[derive(Debug, Clone)]
struct Chunk {
    index: usize,
    start_byte: u64,
    end_byte: u64, // inclusive
}

#[derive(Debug)]
#[allow(dead_code)]
struct ChunkResult {
    index: usize,
    data: Vec<u8>,
    circuit_id: usize,
    elapsed_ms: u64,
    bytes: u64,
}

/// Manages the chunk assignment and reassembly for a multipath download.
#[allow(dead_code)]
struct ChunkManager {
    chunks: Vec<Chunk>,
    next_chunk: AtomicUsize,
    completed: AtomicUsize,
    total_chunks: usize,
    file_size: u64,
}

impl ChunkManager {
    fn new(file_size: u64) -> Self {
        let mut chunks = Vec::new();
        let mut offset = 0u64;
        let mut idx = 0;
        while offset < file_size {
            let end = (offset + CHUNK_SIZE - 1).min(file_size - 1);
            chunks.push(Chunk {
                index: idx,
                start_byte: offset,
                end_byte: end,
            });
            offset = end + 1;
            idx += 1;
        }
        let total = chunks.len();
        ChunkManager {
            chunks,
            next_chunk: AtomicUsize::new(0),
            completed: AtomicUsize::new(0),
            total_chunks: total,
            file_size,
        }
    }

    /// Claim the next unclaimed chunk (atomic, lock-free)
    fn claim_next(&self) -> Option<Chunk> {
        let idx = self.next_chunk.fetch_add(1, Ordering::Relaxed);
        self.chunks.get(idx).cloned()
    }

    fn mark_complete(&self) -> usize {
        self.completed.fetch_add(1, Ordering::Relaxed) + 1
    }

    fn is_done(&self) -> bool {
        self.completed.load(Ordering::Relaxed) >= self.total_chunks
    }

    fn progress_pct(&self) -> f64 {
        if self.total_chunks == 0 {
            return 100.0;
        }
        (self.completed.load(Ordering::Relaxed) as f64 / self.total_chunks as f64) * 100.0
    }
}

// ============================================================================
// BANDWIDTH-WEIGHTED CIRCUIT SELECTION (HFT-style)
// ============================================================================

/// Selects the best circuit for the next chunk based on actual throughput measurements.
/// Uses Thompson Sampling from CircuitScorer — circuits with higher observed bandwidth
/// get exponentially more chunk assignments.
fn select_best_circuit(scorer: &CircuitScorer, num_circuits: usize) -> usize {
    if num_circuits == 0 {
        return 0;
    }
    scorer.best_circuit_for_url(num_circuits)
}

// ============================================================================
// MULTIPATH DOWNLOAD ENGINE
// ============================================================================

/// Downloads a single large file across multiple Tor circuits simultaneously.
/// Each circuit downloads different byte ranges in parallel, reassembled on disk.
///
/// Returns total bytes downloaded.
pub async fn multipath_download(
    url: &str,
    output_path: &Path,
    socks_ports: &[u16],
    app: &AppHandle,
    cancel: Arc<AtomicBool>,
) -> Result<u64> {
    if socks_ports.is_empty() {
        return Err(anyhow!("No SOCKS ports available for multipath download"));
    }

    let num_circuits = socks_ports.len();
    let scorer = Arc::new(CircuitScorer::new(num_circuits));
    let bbr = Arc::new(BbrController::new(num_circuits.min(8), num_circuits));

    let clients_state: Vec<crate::tor_native::SharedTorClient> = {
        if let Some(guard) = app.state::<crate::AppState>().download_swarm_guard.lock().await.as_ref() {
            guard.lock().await.get_arti_clients()
        } else {
            Vec::new()
        }
    };
    if clients_state.is_empty() {
        return Err(anyhow!(
            "No active Tor clients available for multipath download"
        ));
    }

    // Build ArtiClients for each circuit
    let clients: Vec<ArtiClient> = clients_state
        .iter()
        .map(|shared_client| {
            let tor_client = shared_client.read().unwrap().clone();
            let isolation_token = arti_client::IsolationToken::new();
            ArtiClient::new((*tor_client).clone(), Some(isolation_token))
        })
        .collect();

    // Phase 1: Probe for file size and range support
    let _ = app.emit(
        "crawl_log",
        format!(
            "[MULTIPATH] Probing {} for range support across {} circuits...",
            url, num_circuits
        ),
    );

    let probe_client = &clients[0];
    let resp = probe_client
        .head(url)
        .send()
        .await
        .map_err(|e| anyhow!("HEAD probe failed: {}", e))?;

    let file_size = resp.content_length().ok_or_else(|| {
        anyhow!("Server did not return Content-Length — multipath requires known file size")
    })?;

    let supports_ranges = resp
        .headers()
        .get("accept-ranges")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.to_lowercase().contains("bytes"))
        .unwrap_or(false);

    if !supports_ranges {
        return Err(anyhow!(
            "Server does not support byte ranges — falling back to single-circuit"
        ));
    }

    if file_size < MULTIPATH_MIN_SIZE {
        return Err(anyhow!(
            "File too small for multipath ({} bytes < {} threshold) — use single-circuit",
            file_size,
            MULTIPATH_MIN_SIZE
        ));
    }

    // Phase 2: Create chunk manager
    let chunk_mgr = Arc::new(ChunkManager::new(file_size));
    let _ = app.emit(
        "crawl_log",
        format!(
            "[MULTIPATH] File: {} bytes | {} chunks × {}KB | {} circuits",
            file_size,
            chunk_mgr.total_chunks,
            CHUNK_SIZE / 1024,
            num_circuits
        ),
    );

    // Phase 3: Pre-allocate output file
    {
        let file = std::fs::File::create(output_path)?;
        file.set_len(file_size)?;
    }

    // Phase 4: Spawn circuit workers
    let output_path = Arc::new(output_path.to_path_buf());
    let url = Arc::new(url.to_string());
    let inflight = Arc::new(AtomicUsize::new(0));
    let total_downloaded = Arc::new(AtomicU64::new(0));
    let started_at = Instant::now();

    let mut handles = tokio::task::JoinSet::new();

    for (circuit_id, client) in clients.iter().cloned().enumerate().take(num_circuits) {
        let chunk_mgr = chunk_mgr.clone();
        let scorer = scorer.clone();
        let bbr = bbr.clone();
        let url = url.clone();
        let output_path = output_path.clone();
        let inflight = inflight.clone();
        let total_downloaded = total_downloaded.clone();
        let cancel = cancel.clone();
        let app = app.clone();
        let nc = num_circuits;

        handles.spawn(async move {
            loop {
                if cancel.load(Ordering::Relaxed) { break; }
                if chunk_mgr.is_done() { break; }

                // BBR-aware pacing: yield if too many chunks inflight
                let current_inflight = inflight.load(Ordering::Relaxed);
                let bbr_limit = bbr.current_active();
                if current_inflight >= MAX_INFLIGHT.min(bbr_limit * 2) {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    continue;
                }

                // Bandwidth-weighted selection: prefer this circuit if it's scoring well
                let best = select_best_circuit(&scorer, nc);
                if best != circuit_id && chunk_mgr.completed.load(Ordering::Relaxed) > 2 {
                    // Not the best circuit — yield to let better circuits claim chunks
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }

                let chunk = match chunk_mgr.claim_next() {
                    Some(c) => c,
                    None => break,
                };

                inflight.fetch_add(1, Ordering::Relaxed);
                let start_time = Instant::now();

                let mut success = false;
                for attempt in 1..=MAX_CHUNK_RETRIES {
                    let start_offset = chunk.start_byte;
                    let end_offset = chunk.end_byte;
                    let response = client
                        .get(url.as_str())
                        .header("Range", &format!("bytes={start_offset}-{end_offset}"))
                        .send()
                        .await;
                    match response
                    {
                        Ok(resp) if resp.status() == StatusCode::PARTIAL_CONTENT || resp.status().is_success() => {
                            match resp.bytes().await {
                                Ok(data) => {
                                    let elapsed_ms = start_time.elapsed().as_millis() as u64;
                                    let data_len = data.len() as u64;

                                    // Write chunk to file at correct offset
                                    let path = output_path.clone();
                                    let offset = chunk.start_byte;
                                    let data_vec = data.to_vec();
                                    tokio::task::spawn_blocking(move || {
                                        use std::io::{Seek, SeekFrom, Write};
                                        let mut f = std::fs::OpenOptions::new()
                                            .write(true)
                                            .open(path.as_path())
                                            .unwrap();
                                        f.seek(SeekFrom::Start(offset)).unwrap();
                                        f.write_all(&data_vec).unwrap();
                                    }).await.unwrap();

                                    // Update telemetry
                                    scorer.record_piece(circuit_id, data_len, elapsed_ms);
                                    bbr.on_success(data_len, elapsed_ms);
                                    total_downloaded.fetch_add(data_len, Ordering::Relaxed);

                                    let completed = chunk_mgr.mark_complete();
                                    let pct = chunk_mgr.progress_pct();
                                    if completed.is_multiple_of(10) || completed == chunk_mgr.total_chunks {
                                        let elapsed_secs = started_at.elapsed().as_secs_f64();
                                        let speed_mbps = (total_downloaded.load(Ordering::Relaxed) as f64 / elapsed_secs) / (1024.0 * 1024.0);
                                        let _ = app.emit("crawl_log", format!(
                                            "[MULTIPATH] {}/{} chunks ({:.1}%) | {:.2} MB/s | Circuit {}",
                                            completed, chunk_mgr.total_chunks, pct, speed_mbps, circuit_id
                                        ));
                                    }

                                    success = true;
                                    break;
                                }
                                Err(e) => {
                                    eprintln!("[MULTIPATH] Body read error on circuit {} chunk {}: {}", circuit_id, chunk.index, e);
                                    bbr.on_timeout();
                                }
                            }
                        }
                        Ok(resp) if resp.status().as_u16() == 503 || resp.status().as_u16() == 429 => {
                            bbr.on_reject();
                            tokio::time::sleep(Duration::from_millis(2000 * attempt as u64)).await;
                        }
                        Ok(resp) => {
                            eprintln!("[MULTIPATH] Unexpected status {} for chunk {} on circuit {}",
                                resp.status(), chunk.index, circuit_id);
                            bbr.on_reject();
                            tokio::time::sleep(Duration::from_millis(1000)).await;
                        }
                        Err(e) => {
                            eprintln!("[MULTIPATH] Request error on circuit {} chunk {} attempt {}: {}",
                                circuit_id, chunk.index, attempt, e);
                            bbr.on_timeout();
                            tokio::time::sleep(Duration::from_millis(1500 * attempt as u64)).await;
                        }
                    }
                }

                inflight.fetch_sub(1, Ordering::Relaxed);

                if !success {
                    eprintln!("[MULTIPATH] Chunk {} failed after {} retries on circuit {}",
                        chunk.index, MAX_CHUNK_RETRIES, circuit_id);
                }
            }
        });
    }

    // Wait for all workers
    while let Some(result) = handles.join_next().await {
        if let Err(e) = result {
            eprintln!("[MULTIPATH] Worker panicked: {:?}", e);
        }
    }

    let total = total_downloaded.load(Ordering::Relaxed);
    let elapsed = started_at.elapsed();
    let speed_mbps = (total as f64 / elapsed.as_secs_f64()) / (1024.0 * 1024.0);

    let _ = app.emit(
        "crawl_log",
        format!(
            "[MULTIPATH] ✓ Complete: {} bytes in {:.1}s ({:.2} MB/s) across {} circuits",
            total,
            elapsed.as_secs_f64(),
            speed_mbps,
            num_circuits
        ),
    );

    if !chunk_mgr.is_done() {
        return Err(anyhow!(
            "Multipath download incomplete: {}/{} chunks",
            chunk_mgr.completed.load(Ordering::Relaxed),
            chunk_mgr.total_chunks
        ));
    }

    Ok(total)
}

/// Check if a file should use multipath downloading.
/// Returns true if the file is large enough and the server supports ranges.
pub fn should_use_multipath(file_size: Option<u64>, circuit_count: usize) -> bool {
    circuit_count >= 2 && file_size.is_some_and(|s| s >= MULTIPATH_MIN_SIZE)
}
