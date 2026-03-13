use crate::adapters::CrawlerAdapter;
use crate::arti_client::ArtiClient;
use crate::binary_telemetry::{self, DownloadStatusFrame, EventKind};
use anyhow::{anyhow, Result};
use http::header::CONTENT_RANGE;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::{self, File, OpenOptions};

// Phase 41: Windows NT Kernel SetFileValidData zero-filling blocks override for immense payloads
// Phase 128: Returns (Ok(()), mmap_safe) where mmap_safe=true only when SetFileValidData
// succeeded. Mmap over uncommitted pages causes STATUS_ACCESS_VIOLATION (0xc0000005)
// on non-admin Windows when NT hasn't zero-filled the pre-allocated region yet.
#[cfg(target_os = "windows")]
fn preallocate_windows_nt_blocks(file: &std::fs::File, size: u64) -> (std::io::Result<()>, bool) {
    use std::os::windows::io::AsRawHandle;
    let handle = file.as_raw_handle() as *mut std::ffi::c_void;
    extern "system" {
        fn SetFileValidData(hFile: *mut std::ffi::c_void, ValidDataLength: i64) -> i32;
        fn GetLastError() -> u32;
    }
    if let Err(e) = file.set_len(size) {
        return (Err(e), false);
    }
    let mmap_safe = unsafe {
        // Phase 129: Guard against missing SE_MANAGE_VOLUME_NAME privilege.
        // SetFileValidData requires admin rights. Without it, the call returns 0
        // and GetLastError() returns ERROR_PRIVILEGE_NOT_HELD (1314).
        // Fall through gracefully — NT will zero-fill on write (slower but correct).
        // Phase 128: Callers MUST NOT create mmap when this returns false — the valid
        // data length hasn't been extended, so mmap writes beyond the original valid
        // region will hit uncommitted pages and cause ACCESS_VIOLATION.
        let result = SetFileValidData(handle, size as i64);
        if result == 0 {
            let err = GetLastError();
            // Phase 129: Deduplicate this message — log once per session, not per file.
            static WARNED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
            if !WARNED.swap(true, std::sync::atomic::Ordering::Relaxed) {
                if err == 1314 {
                    eprintln!("[IO] SetFileValidData skipped (requires admin). NT will zero-fill. mmap disabled for pre-allocated files.");
                } else if err != 0 {
                    eprintln!("[IO] SetFileValidData warning: error code {}. mmap disabled for pre-allocated files.", err);
                }
            }
            false
        } else {
            true
        }
    };
    (Ok(()), mmap_safe)
}

#[cfg(not(target_os = "windows"))]
fn preallocate_windows_nt_blocks(file: &std::fs::File, size: u64) -> (std::io::Result<()>, bool) {
    (file.set_len(size), true)
}
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock as StdRwLock};
use std::time::{Duration, Instant, SystemTime};
use tauri::{AppHandle, Emitter, Manager};

use tokio::task::JoinSet;

#[cfg(target_os = "windows")]
use std::process::Command;

#[cfg(target_os = "windows")]
#[allow(dead_code)]
fn apply_windows_no_window(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

/// Log writer that writes timestamped entries to a file alongside the download
#[derive(Clone)]
struct DownloadLogger {
    file: Arc<Mutex<Option<File>>>,
}

impl DownloadLogger {
    fn new(output_dir: &str, filename_hint: &str) -> Self {
        // Phase 129: Write logs to .crawli_logs/ subdirectory to avoid
        // polluting the user's download folder with diagnostic files.
        let log_dir = Path::new(output_dir).join(".crawli_logs");
        let _ = fs::create_dir_all(&log_dir);

        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default();
        let ts = now.as_secs();
        // Sanitize filename for log
        let safe_name = filename_hint.replace(['/', '\\', ':', '?', '*', '"', '<', '>', '|'], "_");
        let log_path = log_dir.join(format!("ariaforge_{}_{}.log", safe_name, ts));

        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .ok();

        if file.is_some() {
            eprintln!("[DownloadLogger] Writing to {}", log_path.display());
        }

        DownloadLogger {
            file: Arc::new(Mutex::new(file)),
        }
    }

    fn log(&self, app: &AppHandle, msg: String) {
        // Emit to UI
        let _ = app.emit("log", msg.clone());
        // Write to file with timestamp
        if let Ok(mut guard) = self.file.lock() {
            if let Some(f) = guard.as_mut() {
                let now = SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default();
                let total_secs = now.as_secs();
                let hours = (total_secs % 86400) / 3600;
                let mins = (total_secs % 3600) / 60;
                let secs = total_secs % 60;
                let _ = writeln!(f, "[{:02}:{:02}:{:02}] {}", hours, mins, secs, msg);
            }
        }
    }
}

// Phase 44: Legacy get_tor_path removed — arti manages Tor in-process

struct ActiveCircuitGuard {
    counter: Arc<AtomicUsize>,
    load: usize,
}
impl ActiveCircuitGuard {
    fn new(counter: Arc<AtomicUsize>) -> Self {
        Self::with_load(counter, 1)
    }

    fn with_load(counter: Arc<AtomicUsize>, load: usize) -> Self {
        let load = load.max(1);
        counter.fetch_add(load, Ordering::Relaxed);
        Self { counter, load }
    }
}
impl Drop for ActiveCircuitGuard {
    fn drop(&mut self) {
        self.counter.fetch_sub(self.load, Ordering::Relaxed);
    }
}

const STREAM_TIMEOUT_SECS: u64 = 15;
const MAX_STALL_RETRIES: usize = 30;
const PROBE_SIZE: u64 = 102_400; // 100KB micro-probe (80% signal in 10% of time)
const PROBE_PROMOTION_SIZE: u64 = 32 * 1024; // 32KB transport seed for micro/small promotion
const HANDSHAKE_CULL_RATIO: f64 = 0.50; // Kill bottom 50% by handshake latency
const DEFAULT_DOWNLOAD_TOURNAMENT_CAP: usize = 48;
const DEFAULT_DOWNLOAD_INITIAL_ACTIVE_CAP_ONION: usize = 16;
const DEFAULT_DOWNLOAD_INITIAL_ACTIVE_CAP_CLEARNET: usize = 32;
const DEFAULT_DOWNLOAD_INITIAL_ACTIVE_MIN: usize = 4;
const DEFAULT_RESUME_COALESCE_PIECES: usize = 4;
const DEFAULT_BATCH_PROBE_PROMOTION_CACHE_MIB: usize = 64;
const DEFAULT_DOWNLOAD_MAX_HOST_CONNECTIONS_CLEARNET: usize = 32;
const DEFAULT_DOWNLOAD_MAX_HOST_CONNECTIONS_ONION: usize = 32;

// Phase 114: Forbids bisection algorithms from violating Windows memory-mapping page constraints.
#[allow(dead_code)]
const NTFS_PAGE_SIZE: u64 = 4096;
const ARIA_PIECE_SIZE: u64 = 1048576; // 1MB Bitfield Alignment

const MIN_PIECE_SIZE: u64 = ARIA_PIECE_SIZE; // 1MB minimum
const MAX_PIECE_SIZE: u64 = 52_428_800; // 50MB maximum

/// Phase 114: Compute optimal piece size based on file size and circuit count.
/// Enforces hardware cache-line boundaries (1MB) to prevent NTFS LockFreeMmap thrashing
fn compute_piece_size(content_length: u64, circuits: usize) -> u64 {
    if content_length == 0 || circuits == 0 {
        return MIN_PIECE_SIZE;
    }
    let target_pieces_per_circuit = 32u64; // IDM multiplexing subdivision
    let ideal = content_length / (circuits as u64 * target_pieces_per_circuit);
    
    // Align tightly to the nearest 1MB boundary to match APFS/NTFS extent hardware logic
    let aligned_ideal = (ideal / ARIA_PIECE_SIZE).max(1) * ARIA_PIECE_SIZE;
    aligned_ideal.clamp(MIN_PIECE_SIZE, MAX_PIECE_SIZE)
}

fn resume_coalesce_piece_limit() -> usize {
    env_usize("CRAWLI_RESUME_COALESCE_PIECES")
        .unwrap_or(DEFAULT_RESUME_COALESCE_PIECES)
        .clamp(1, 32)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PieceSpan {
    start_piece: usize,
    end_piece: usize,
    start_byte: u64,
    end_byte: u64,
}

fn build_piece_spans(
    piece_completed: &[bool],
    piece_size: u64,
    content_length: u64,
    coalesce_limit: usize,
) -> Vec<PieceSpan> {
    let mut spans = Vec::new();
    if piece_completed.is_empty() || content_length == 0 || piece_size == 0 {
        return spans;
    }

    let coalesce_limit = coalesce_limit.max(1);
    let mut idx = 0usize;
    while idx < piece_completed.len() {
        if piece_completed[idx] {
            idx += 1;
            continue;
        }

        let start_piece = idx;
        let mut end_piece = idx;
        while end_piece + 1 < piece_completed.len()
            && !piece_completed[end_piece + 1]
            && end_piece + 1 - start_piece < coalesce_limit
        {
            end_piece += 1;
        }

        let start_byte = start_piece as u64 * piece_size;
        let end_byte = (((end_piece as u64) + 1) * piece_size)
            .saturating_sub(1)
            .min(content_length.saturating_sub(1));
        spans.push(PieceSpan {
            start_piece,
            end_piece,
            start_byte,
            end_byte,
        });
        idx = end_piece + 1;
    }

    spans
}

fn piece_len_for_index(content_length: u64, piece_size: u64, piece_idx: usize) -> u64 {
    let start = piece_idx as u64 * piece_size;
    if start >= content_length {
        return 0;
    }
    (((piece_idx as u64) + 1) * piece_size).min(content_length) - start
}

fn normalize_download_state(
    state: &mut DownloadState,
    effective_circuits: usize,
    piece_size: u64,
    total_pieces: usize,
) {
    state.chunk_size = piece_size;
    if total_pieces == 0 {
        state.piece_mode = false;
        state.total_pieces = 0;
        if state.current_offsets.len() != effective_circuits {
            state.current_offsets = vec![0; effective_circuits];
        }
        if state.completed_chunks.len() != effective_circuits {
            state.completed_chunks = vec![false; effective_circuits];
        }
        return;
    }

    state.piece_mode = true;
    state.total_pieces = total_pieces;
    if state.completed_pieces.len() != total_pieces {
        state.completed_pieces.resize(total_pieces, false);
    }
    if state.current_offsets.len() != total_pieces {
        state.current_offsets.resize(total_pieces, 0);
    }
}

fn estimate_downloaded_bytes(state: &DownloadState, effective_circuits: usize) -> u64 {
    if state.piece_mode && state.total_pieces > 0 && state.chunk_size > 0 {
        let mut downloaded = 0u64;
        for piece_idx in 0..state.total_pieces {
            let piece_len = piece_len_for_index(state.content_length, state.chunk_size, piece_idx);
            if piece_len == 0 {
                continue;
            }
            if state
                .completed_pieces
                .get(piece_idx)
                .copied()
                .unwrap_or(false)
            {
                downloaded = downloaded.saturating_add(piece_len);
            } else {
                downloaded = downloaded.saturating_add(
                    state
                        .current_offsets
                        .get(piece_idx)
                        .copied()
                        .unwrap_or(0)
                        .min(piece_len),
                );
            }
        }
        return downloaded;
    }

    let mut downloaded = 0u64;
    for (i, &done) in state
        .completed_chunks
        .iter()
        .enumerate()
        .take(effective_circuits)
    {
        if done {
            let end_byte = if i == effective_circuits.saturating_sub(1) {
                state.content_length.saturating_sub(1)
            } else {
                ((i as u64 + 1) * state.chunk_size).saturating_sub(1)
            };
            let start_byte = i as u64 * state.chunk_size;
            downloaded = downloaded.saturating_add(end_byte.saturating_sub(start_byte) + 1);
        } else {
            downloaded = downloaded.saturating_add(
                state
                    .current_offsets
                    .get(i)
                    .copied()
                    .unwrap_or(0)
                    .min(state.chunk_size),
            );
        }
    }
    downloaded
}

// Health monitoring: kill circuits below this fraction of median speed
const MIN_SPEED_RATIO: f64 = 0.20; // 20% of median = too slow
const HEALTH_CHECK_INTERVAL_SECS: u64 = 15;

const UNCHOKE_INTERVAL_SECS: u64 = 30; // Test a fresh circuit every 30s

fn env_usize(name: &str) -> Option<usize> {
    std::env::var(name).ok()?.parse::<usize>().ok()
}

fn download_tournament_cap(is_onion: bool) -> usize {
    env_usize("CRAWLI_DOWNLOAD_TOURNAMENT_CAP")
        .unwrap_or(if is_onion {
            DEFAULT_DOWNLOAD_TOURNAMENT_CAP
        } else {
            DEFAULT_DOWNLOAD_TOURNAMENT_CAP * 2
        })
        .max(2)
}

fn download_tournament_candidate_count(target_workers: usize, is_onion: bool) -> usize {
    let target_workers = target_workers.max(1);
    let dynamic = crate::tor::tournament_candidate_count(target_workers);
    dynamic.clamp(target_workers, download_tournament_cap(is_onion))
}

fn download_initial_active_budget(
    scaled_circuits: usize,
    total_pieces: usize,
    is_onion: bool,
) -> usize {
    let scaled_circuits = scaled_circuits.max(1);
    let configured_cap = env_usize("CRAWLI_DOWNLOAD_ACTIVE_START_CAP")
        .unwrap_or(if is_onion {
            DEFAULT_DOWNLOAD_INITIAL_ACTIVE_CAP_ONION
        } else {
            DEFAULT_DOWNLOAD_INITIAL_ACTIVE_CAP_CLEARNET
        })
        .max(1);
    let dynamic_budget = ((total_pieces as f64).sqrt().ceil() as usize + 2)
        .clamp(DEFAULT_DOWNLOAD_INITIAL_ACTIVE_MIN, configured_cap.max(1));
    scaled_circuits
        .min(dynamic_budget)
        .max(DEFAULT_DOWNLOAD_INITIAL_ACTIVE_MIN.min(scaled_circuits))
}

fn handshake_keep_count(results_len: usize, target_workers: usize, is_onion: bool) -> usize {
    let results_len = results_len.max(1);
    if is_onion {
        ((results_len as f64 * (1.0 - HANDSHAKE_CULL_RATIO)) as usize).max(1)
    } else {
        target_workers.clamp(1, results_len)
    }
}

/// UCB1 Multi-Armed Bandit circuit scorer.
/// Tracks per-circuit performance and computes optimal piece assignment.
#[allow(dead_code)]
struct CircuitScorer {
    pieces_completed: Vec<AtomicU64>,
    total_bytes: Vec<AtomicU64>,
    total_elapsed_ms: Vec<AtomicU64>,
    global_pieces: AtomicU64,
    capacity: usize,
    // Phase 3: Mathematical Telemetry (Kalman Filtering)
    latency_kalman: Vec<std::sync::Mutex<crate::kalman::KalmanFilter>>,
    latency_baseline: Vec<AtomicU64>, // Baseline from first 3 pieces (ms)
    latency_samples: Vec<AtomicU64>,  // Number of samples recorded
    // Phase 130: CUSUM change-point detection per download circuit.
    // Detects sudden circuit degradation and triggers early recycling
    // before multiple pieces stall. 12 bytes per circuit via CircuitHealth.
    circuit_health: Vec<crate::circuit_health::CircuitHealth>,
}

#[allow(dead_code)]
impl CircuitScorer {
    fn new(num_circuits: usize) -> Self {
        let mut kalmans = Vec::with_capacity(num_circuits);
        for _ in 0..num_circuits {
            // q = 10.0 (process noise), r = 100.0 (measurement noise for volatile Tor relays)
            kalmans.push(std::sync::Mutex::new(crate::kalman::KalmanFilter::new(
                10.0, 100.0, 0.0,
            )));
        }

        CircuitScorer {
            pieces_completed: (0..num_circuits).map(|_| AtomicU64::new(0)).collect(),
            total_bytes: (0..num_circuits).map(|_| AtomicU64::new(0)).collect(),
            total_elapsed_ms: (0..num_circuits).map(|_| AtomicU64::new(0)).collect(),
            global_pieces: AtomicU64::new(0),
            capacity: num_circuits,
            latency_kalman: kalmans,
            latency_baseline: (0..num_circuits).map(|_| AtomicU64::new(0)).collect(),
            latency_samples: (0..num_circuits).map(|_| AtomicU64::new(0)).collect(),
            // Phase 130: CUSUM health tracker per download circuit
            circuit_health: (0..num_circuits).map(|_| crate::circuit_health::CircuitHealth::new()).collect(),
        }
    }

    /// Record a completed piece for a circuit
    fn record_piece(&self, cid: usize, bytes: u64, elapsed_ms: u64) {
        if cid < self.capacity {
            self.pieces_completed[cid].fetch_add(1, Ordering::Relaxed);
            self.total_bytes[cid].fetch_add(bytes, Ordering::Relaxed);
            self.total_elapsed_ms[cid].fetch_add(elapsed_ms.max(1), Ordering::Relaxed);
            self.global_pieces.fetch_add(1, Ordering::Relaxed);
            // Phase 3: Kalman latency update
            self.record_latency(cid, elapsed_ms);
        }
    }

    /// Phase 3: Record latency and update Kalman Filter predictive model
    fn record_latency(&self, cid: usize, elapsed_ms: u64) {
        if cid >= self.capacity {
            return;
        }
        let samples = self.latency_samples[cid].fetch_add(1, Ordering::Relaxed);

        let mut kf = self.latency_kalman[cid].lock().unwrap();

        if samples < 3 {
            // Build baseline from first 3 pieces
            let old = self.latency_baseline[cid].load(Ordering::Relaxed);
            let new_baseline = if old == 0 {
                elapsed_ms
            } else {
                (old + elapsed_ms) / 2
            };
            self.latency_baseline[cid].store(new_baseline, Ordering::Relaxed);
            if kf.x == 0.0 {
                kf.x = elapsed_ms as f64;
            } else {
                kf.update(elapsed_ms as f64);
            }
        } else {
            // Feed observation into Kalman Filter
            kf.update(elapsed_ms as f64);
        }
    }

    /// Phase 3: Predict if a circuit will stall using Kalman mathematics
    fn is_degrading(&self, cid: usize) -> bool {
        if cid >= self.capacity {
            return false;
        }
        let samples = self.latency_samples[cid].load(Ordering::Relaxed);
        if samples < 5 {
            return false;
        } // Need enough data

        let baseline = self.latency_baseline[cid].load(Ordering::Relaxed) as f64;
        if baseline == 0.0 {
            return false;
        }

        let kf = self.latency_kalman[cid].lock().unwrap();
        let prediction = kf.predict();

        // If predicted latency + uncertainty deviation > 2.5x baseline, it is stalling!
        let deviation = kf.p.sqrt();
        (prediction + (deviation * 1.5)) > (baseline * 2.5)
    }

    /// Compute Thompson Sampling score for a circuit (higher = should get more pieces)
    fn thompson_score(&self, cid: usize) -> f64 {
        if cid >= self.capacity {
            return 0.0;
        }
        let n = self.pieces_completed[cid].load(Ordering::Relaxed);
        if n == 0 {
            return f64::MAX; // Untested = infinite score (explore first)
        }
        let total_b = self.total_bytes[cid].load(Ordering::Relaxed) as f64;
        let total_ms = self.total_elapsed_ms[cid].load(Ordering::Relaxed).max(1) as f64;
        let avg_speed = total_b / total_ms; // bytes per ms

        // The Kalman filter tracks latency. We use its covariance (uncertainty) to drive exploration.
        let mut variance = {
            let kf = self.latency_kalman[cid].lock().unwrap();
            kf.p
        };
        if variance < 0.001 {
            variance = 0.001;
        }

        // Box-Muller transform for normal distribution N(mean, variance) Lock-Free
        let std_dev = variance.sqrt();
        let time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let u1 = (((time ^ (time >> 12)) % 10000) as f64 / 10000.0).max(0.0001);
        let u2 = (((time ^ (time >> 20)) % 10000) as f64 / 10000.0).max(0.0001);

        let z0 = (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos();
        let thompson_scaling_factor = 0.01;

        avg_speed + (z0 * std_dev * thompson_scaling_factor)
    }

    /// Compute average speed in MB/s for a circuit
    fn avg_speed_mbps(&self, cid: usize) -> f64 {
        if cid >= self.capacity {
            return 0.0;
        }
        let total_b = self.total_bytes[cid].load(Ordering::Relaxed) as f64;
        let total_ms = self.total_elapsed_ms[cid].load(Ordering::Relaxed).max(1) as f64;
        (total_b / total_ms) * 1000.0 / 1_048_576.0 // Convert bytes/ms to MB/s
    }

    /// Phase 140: Returns circuit IDs with avg speed >= threshold_mbps, sorted fastest-first.
    /// Only considers circuits with ≥3 recorded pieces for statistical validity.
    fn fast_circuits_above_threshold(
        &self,
        threshold_mbps: f64,
    ) -> Vec<(usize, f64)> {
        let mut qualified: Vec<(usize, f64)> = (0..self.capacity)
            .filter_map(|cid| {
                let pieces = self.pieces_completed[cid].load(Ordering::Relaxed);
                if pieces < 3 {
                    return None;
                }
                let speed = self.avg_speed_mbps(cid);
                if speed >= threshold_mbps && !self.is_degrading(cid) {
                    Some((cid, speed))
                } else {
                    None
                }
            })
            .collect();
        qualified.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        qualified
    }

    /// How long a circuit should wait before claiming the next piece.
    /// Fast circuits (≥0.3 MB/s): 0ms. Slow circuits (<0.3 MB/s): 3-5s.
    /// Phase 140: Aggressive speed-threshold enforcement — circuits below
    /// 0.3 MB/s are penalized with long delays so fast circuits grab work first.
    fn yield_delay(&self, cid: usize) -> Duration {
        if cid >= self.capacity {
            return Duration::ZERO;
        }
        let pieces = self.pieces_completed[cid].load(Ordering::Relaxed);
        if pieces == 0 {
            return Duration::ZERO; // Untested, let it prove itself
        }

        // Phase 140: Hard speed threshold — circuits below 0.3 MB/s get heavy penalty
        let speed = self.avg_speed_mbps(cid);
        if pieces >= 3 && speed < 0.3 {
            // Below threshold: 3-5s delay proportional to how far below 0.3 MB/s
            let severity = ((0.3 - speed) / 0.3).clamp(0.0, 1.0);
            let delay_ms = 3000 + (severity * 2000.0) as u64; // 3000-5000ms
            return Duration::from_millis(delay_ms.min(5000));
        }

        let my_score = self.thompson_score(cid);
        if my_score == f64::MAX {
            return Duration::ZERO;
        }

        // Collect scores of all active circuits
        let mut scores: Vec<f64> = (0..self.capacity)
            .filter(|&i| self.pieces_completed[i].load(Ordering::Relaxed) > 0)
            .map(|i| self.thompson_score(i))
            .collect();
        if scores.is_empty() {
            return Duration::ZERO;
        }

        scores.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
        let best = scores.first().copied().unwrap_or(1.0);
        if best <= 0.0 {
            return Duration::ZERO;
        }

        // Ratio: 0.0 (worst) to 1.0 (best)
        let ratio = (my_score / best).clamp(0.0, 1.0);

        // Map: top 50% → 0ms, bottom 50% → 0-1500ms proportional
        if ratio > 0.5 {
            Duration::ZERO
        } else {
            let delay_ms = ((0.5 - ratio) * 3000.0) as u64; // 0-1500ms
            Duration::from_millis(delay_ms.min(1500))
        }
    }

    /// Find the slowest active circuit by average speed
    fn slowest_circuit(&self) -> Option<usize> {
        (0..self.capacity)
            .filter(|&i| self.pieces_completed[i].load(Ordering::Relaxed) > 0)
            .min_by(|&a, &b| {
                self.avg_speed_mbps(a)
                    .partial_cmp(&self.avg_speed_mbps(b))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    }

    /// Phase 130: Record a successful download chunk for CUSUM tracking.
    /// Feeds the shared CircuitHealth module to track cumulative degradation.
    fn record_download_success(&self, cid: usize) {
        if cid < self.capacity {
            self.circuit_health[cid].record_success();
        }
    }

    /// Phase 130: Record a failed download attempt for CUSUM tracking.
    fn record_download_failure(&self, cid: usize) {
        if cid < self.capacity {
            self.circuit_health[cid].record_failure();
        }
    }

    /// Phase 130: Check if CUSUM has detected a change-point on this circuit.
    /// Returns true if the circuit should be recycled with a fresh identity.
    fn should_recycle(&self, cid: usize) -> bool {
        if cid < self.capacity {
            self.circuit_health[cid].cusum_triggered()
        } else {
            false
        }
    }

    /// Phase 130: Reset CUSUM after circuit recycling (fresh slate for new identity).
    fn reset_health(&self, cid: usize) {
        if cid < self.capacity {
            self.circuit_health[cid].reset_cusum();
        }
    }
}

use crate::bbr::BbrController;

/// Exponential backoff: min(2^retries * 500ms, 30s)
fn backoff_duration(retries: usize) -> Duration {
    let base_ms = 500u64 * (1u64 << retries.min(6));
    Duration::from_millis(base_ms.min(30_000))
}

#[derive(Clone, Default)]
struct IntervalTracker {
    intervals: Vec<(u64, u64)>,
}

impl IntervalTracker {
    fn new() -> Self {
        Self {
            intervals: Vec::new(),
        }
    }

    fn add(&mut self, start: u64, end: u64) {
        if start >= end {
            return;
        }

        let mut new_intervals = Vec::with_capacity(self.intervals.len() + 1);
        let mut merged = false;
        let mut cs = start;
        let mut ce = end;

        for &i in &self.intervals {
            if merged {
                new_intervals.push(i);
            } else if ce < i.0 {
                new_intervals.push((cs, ce));
                new_intervals.push(i);
                merged = true;
            } else if cs > i.1 {
                new_intervals.push(i);
            } else {
                cs = cs.min(i.0);
                ce = ce.max(i.1);
            }
        }
        if !merged {
            new_intervals.push((cs, ce));
        }
        self.intervals = new_intervals;
    }

    fn contiguous_up_to(&self) -> u64 {
        if self.intervals.is_empty() {
            0
        } else if self.intervals[0].0 == 0 {
            self.intervals[0].1
        } else {
            0
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct DownloadState {
    pub completed_chunks: Vec<bool>,
    #[serde(default)]
    pub current_offsets: Vec<u64>,
    pub num_circuits: usize,
    pub chunk_size: u64,
    pub content_length: u64,
    #[serde(default)]
    pub piece_mode: bool,
    #[serde(default)]
    pub completed_pieces: Vec<bool>,
    #[serde(default)]
    pub total_pieces: usize,
    #[serde(default)]
    pub etag: Option<String>,
    #[serde(default)]
    pub last_modified: Option<String>,
}

pub struct WriteMsg {
    pub filepath: String,
    pub offset: u64,
    pub data: bytes::Bytes,
    pub mmap_written: bool,
    pub close_file: bool,
    pub chunk_id: usize,
    pub piece_end: usize,
}

#[derive(Clone, Serialize)]
pub struct TorStatusEvent {
    pub state: String,
    pub message: String,
    pub daemon_count: usize,
}

#[derive(Clone)]
pub struct LockFreeMmap {
    ptr: usize,
    len: usize,
}

unsafe impl Send for LockFreeMmap {}
unsafe impl Sync for LockFreeMmap {}

impl LockFreeMmap {
    pub fn new(mmap: &memmap2::MmapMut) -> Self {
        Self {
            ptr: mmap.as_ptr() as usize,
            len: mmap.len(),
        }
    }

    pub fn write_slice(&self, offset: usize, data: &[u8]) {
        if offset + data.len() <= self.len {
            unsafe {
                std::ptr::copy_nonoverlapping(
                    data.as_ptr(),
                    (self.ptr as *mut u8).add(offset),
                    data.len(),
                );
            }
        }
    }
}

#[derive(Clone, Serialize)]
pub struct DownloadCompleteEvent {
    pub url: String,
    pub path: String,
    pub hash: String,
    pub time_taken_secs: f64,
}

#[derive(Clone, Serialize)]
pub struct DownloadInterruptedEvent {
    pub url: String,
    pub path: String,
    pub reason: String,
}

#[derive(Clone)]
pub struct DownloadControl {
    pause_requested: Arc<AtomicBool>,
    stop_requested: Arc<AtomicBool>,
}

impl DownloadControl {
    fn new() -> Self {
        Self {
            pause_requested: Arc::new(AtomicBool::new(false)),
            stop_requested: Arc::new(AtomicBool::new(false)),
        }
    }

    fn interruption_reason(&self) -> Option<&'static str> {
        if self.stop_requested.load(Ordering::Relaxed) {
            Some("Stopped")
        } else if self.pause_requested.load(Ordering::Relaxed) {
            Some("Paused")
        } else {
            None
        }
    }
}

static ACTIVE_CONTROL: OnceLock<Mutex<Option<DownloadControl>>> = OnceLock::new();

fn active_control_slot() -> &'static Mutex<Option<DownloadControl>> {
    ACTIVE_CONTROL.get_or_init(|| Mutex::new(None))
}

pub fn activate_download_control() -> Option<DownloadControl> {
    let mut guard = active_control_slot().lock().ok()?;
    if guard.is_some() {
        return None;
    }

    let control = DownloadControl::new();
    *guard = Some(control.clone());
    Some(control)
}

pub fn clear_download_control() {
    if let Ok(mut guard) = active_control_slot().lock() {
        *guard = None;
    }
}

pub fn request_pause() -> bool {
    let guard = match active_control_slot().lock() {
        Ok(guard) => guard,
        Err(_) => return false,
    };

    if let Some(control) = guard.as_ref() {
        control.pause_requested.store(true, Ordering::Relaxed);
        true
    } else {
        false
    }
}

pub fn request_stop() -> bool {
    let guard = match active_control_slot().lock() {
        Ok(guard) => guard,
        Err(_) => return false,
    };

    if let Some(control) = guard.as_ref() {
        control.stop_requested.store(true, Ordering::Relaxed);
        control.pause_requested.store(false, Ordering::Relaxed);
        true
    } else {
        false
    }
}

fn telemetry_handle(app: &AppHandle) -> Option<crate::runtime_metrics::RuntimeTelemetry> {
    app.try_state::<crate::AppState>()
        .map(|state| state.telemetry.clone())
}

fn publish_batch_progress(app: &AppHandle, progress: BatchProgressEvent) {
    crate::telemetry_bridge::publish_batch_progress(
        app,
        crate::telemetry_bridge::BridgeBatchProgress {
            completed: progress.completed,
            failed: progress.failed,
            total: progress.total,
            current_file: progress.current_file,
            speed_mbps: progress.speed_mbps,
            downloaded_bytes: progress.downloaded_bytes,
            active_circuits: progress.active_circuits,
            bbr_bottleneck_mbps: None,
            ekf_covariance: None,
        },
    );
}

fn batch_speed_mbps(downloaded_bytes: u64, batch_started_at: Instant) -> f64 {
    let elapsed_secs = batch_started_at.elapsed().as_secs_f64();
    if elapsed_secs > 0.0 {
        (downloaded_bytes as f64 / elapsed_secs) / 1_048_576.0
    } else {
        0.0
    }
}

fn batch_swarm_send_timeout(is_onion: bool) -> Duration {
    let default_secs = if is_onion { 45 } else { 30 };
    Duration::from_secs(
        env_usize("CRAWLI_BATCH_SWARM_SEND_TIMEOUT_SECS")
            .unwrap_or(default_secs)
            .max(5) as u64,
    )
}

fn batch_swarm_body_timeout(is_onion: bool) -> Duration {
    let default_secs = if is_onion { 90 } else { 60 };
    Duration::from_secs(
        env_usize("CRAWLI_BATCH_SWARM_BODY_TIMEOUT_SECS")
            .unwrap_or(default_secs)
            .max(10) as u64,
    )
}

fn batch_swarm_first_byte_timeout(is_onion: bool) -> Duration {
    let default_secs = if is_onion { 18 } else { 10 };
    Duration::from_secs(
        env_usize("CRAWLI_BATCH_SWARM_FIRST_BYTE_TIMEOUT_SECS")
            .unwrap_or(default_secs)
            .max(3) as u64,
    )
}

/// Phase 126: Adaptive first-byte timeout using host capability EWMA.
/// If the host has recorded `first_byte_ewma_ms > 0`, uses `max(3 × ewma, 3000ms)`
/// capped at the fixed default. Otherwise falls back to the fixed timeout.
///
/// This brings the same adaptive TTFB intelligence from the crawl side
/// (CircuitHealth::adaptive_ttfb_ms) to the download transport layer.
/// Typical savings: 200ms EWMA host → 3s timeout vs 18s fixed = 6× faster failure detection.
fn adaptive_first_byte_timeout(url: &str, is_onion: bool) -> Duration {
    let fixed = batch_swarm_first_byte_timeout(is_onion);
    let snapshot = host_capability_snapshot(url, is_onion);
    if let Some(cap) = snapshot {
        if cap.first_byte_ewma_ms > 0.0 {
            let adaptive_ms = (cap.first_byte_ewma_ms * 3.0) as u64;
            let adaptive = Duration::from_millis(adaptive_ms.clamp(3_000, fixed.as_millis() as u64));
            return adaptive;
        }
    }
    fixed
}

fn batch_no_byte_requeue_limit() -> u8 {
    env_usize("CRAWLI_BATCH_NO_BYTE_REQUEUE_LIMIT")
        .unwrap_or(2)
        .clamp(0, 8) as u8
}

fn batch_first_wave_width(parallelism: usize) -> usize {
    parallelism.max(1).saturating_mul(4)
}

fn batch_entry_host(url: &str) -> Option<String> {
    reqwest::Url::parse(url)
        .ok()?
        .host_str()
        .map(|host| host.to_ascii_lowercase())
}

fn batch_probe_promotion_budget_bytes() -> usize {
    env_usize("CRAWLI_BATCH_PROBE_PROMOTION_CACHE_MIB")
        .unwrap_or(DEFAULT_BATCH_PROBE_PROMOTION_CACHE_MIB)
        .clamp(4, 512)
        * 1_048_576
}

fn probe_promotion_size_bytes() -> usize {
    env_usize("CRAWLI_PROBE_PROMOTION_KIB")
        .unwrap_or((PROBE_PROMOTION_SIZE / 1024) as usize)
        .clamp(4, 256)
        * 1024
}

#[derive(Clone, Debug)]
struct PrefetchedProbeData {
    bytes: bytes::Bytes,
    end_offset: u64,
    complete_file: bool,
}

impl PrefetchedProbeData {
    fn len(&self) -> usize {
        self.bytes.len()
    }

    fn next_offset(&self) -> u64 {
        self.end_offset.saturating_add(1)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
enum ResumeValidatorKind {
    #[default]
    None,
    Etag,
    LastModified,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct HostCapabilityState {
    supports_ranges: Option<bool>,
    validator_kind: ResumeValidatorKind,
    recent_successes: u32,
    recent_failures: u32,
    low_speed_aborts: u32,
    consecutive_low_speed_aborts: u32,
    consecutive_connect_failures: u32,
    safe_parallelism_cap: usize,
    connect_rtt_ewma_ms: f64,
    first_byte_ewma_ms: f64,
    last_productive_epoch_ms: u64,
    quarantine_until_epoch_ms: u64,
}

impl Default for HostCapabilityState {
    fn default() -> Self {
        Self {
            supports_ranges: None,
            validator_kind: ResumeValidatorKind::None,
            recent_successes: 0,
            recent_failures: 0,
            low_speed_aborts: 0,
            consecutive_low_speed_aborts: 0,
            consecutive_connect_failures: 0,
            safe_parallelism_cap: 32,
            connect_rtt_ewma_ms: 0.0,
            first_byte_ewma_ms: 0.0,
            last_productive_epoch_ms: 0,
            quarantine_until_epoch_ms: 0,
        }
    }
}

impl HostCapabilityState {
    fn reuse_allowed(&self, is_onion: bool) -> bool {
        if !is_onion {
            return true;
        }
        let now_ms = current_epoch_ms();
        host_is_productive(
            self.recent_successes,
            self.recent_failures,
            self.consecutive_connect_failures,
            self.last_productive_epoch_ms,
            is_onion,
            now_ms,
        ) && self.recent_successes >= 2
            && self.consecutive_low_speed_aborts == 0
            && self.quarantine_until_epoch_ms <= now_ms
    }
}

#[derive(Clone, Copy, Debug)]
enum HostFailureKind {
    Connect,
    Timeout,
    LowSpeed,
}

#[derive(Clone, Copy, Debug)]
struct LowSpeedPolicy {
    min_bytes_per_sec: u64,
    window: Duration,
}

impl LowSpeedPolicy {
    fn enabled(self) -> bool {
        self.min_bytes_per_sec > 0 && !self.window.is_zero()
    }
}

#[derive(Clone, Copy, Debug)]
struct LowSpeedTracker {
    window_started_at: Instant,
    bytes_in_window: u64,
}

impl LowSpeedTracker {
    fn new(now: Instant, initial_bytes: u64) -> Self {
        Self {
            window_started_at: now,
            bytes_in_window: initial_bytes,
        }
    }

    fn observe_progress(&mut self, now: Instant, bytes: u64, policy: LowSpeedPolicy) -> bool {
        if !policy.enabled() {
            return false;
        }
        self.bytes_in_window = self.bytes_in_window.saturating_add(bytes);
        let elapsed = now.saturating_duration_since(self.window_started_at);
        if elapsed < policy.window {
            return false;
        }
        let speed = self.bytes_in_window as f64 / elapsed.as_secs_f64().max(0.001);
        if speed < policy.min_bytes_per_sec as f64 {
            return true;
        }
        self.window_started_at = now;
        self.bytes_in_window = 0;
        false
    }

    fn should_abort_on_idle(&self, now: Instant, policy: LowSpeedPolicy) -> bool {
        if !policy.enabled() {
            return false;
        }
        let elapsed = now.saturating_duration_since(self.window_started_at);
        if elapsed < policy.window {
            return false;
        }
        let speed = self.bytes_in_window as f64 / elapsed.as_secs_f64().max(0.001);
        speed < policy.min_bytes_per_sec as f64
    }
}

static DOWNLOAD_HOST_CAPABILITIES: OnceLock<StdRwLock<HashMap<String, HostCapabilityState>>> =
    OnceLock::new();
static DOWNLOAD_ACTIVE_HOST_CONNECTIONS: OnceLock<Mutex<HashMap<String, usize>>> = OnceLock::new();
static HOST_CAPABILITY_SLED: OnceLock<Option<sled::Db>> = OnceLock::new();

/// Phase 136: Initialize the host capability persistence store.
/// Loads previously observed host capabilities from sled so we don't lose
/// range support knowledge, RTT EWMAs, and parallelism caps across restarts.
pub fn initialize_host_capability_store() {
    HOST_CAPABILITY_SLED.get_or_init(|| {
        let mut path = std::env::var("HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::path::PathBuf::from("."));
        path.push(".crawli");
        let _ = std::fs::create_dir_all(&path);
        path.push("host_capabilities.sled");
        match sled::open(&path) {
            Ok(db) => {
                // Hydrate in-memory map from sled
                let mut loaded = 0usize;
                let guard = download_host_capabilities();
                if let Ok(mut map) = guard.write() {
                    for entry in db.iter().flatten() {
                        if let Ok(key) = std::str::from_utf8(&entry.0) {
                            if let Ok(state) = serde_json::from_slice::<HostCapabilityState>(&entry.1) {
                                // Only restore recent entries (last 24h)
                                let age_ms = current_epoch_ms().saturating_sub(state.last_productive_epoch_ms);
                                if age_ms <= 24 * 60 * 60 * 1_000 {
                                    map.insert(key.to_string(), state);
                                    loaded += 1;
                                }
                            }
                        }
                    }
                }
                if loaded > 0 {
                    eprintln!("[Phase 136] Loaded {} host capabilities from disk cache", loaded);
                }
                Some(db)
            }
            Err(e) => {
                eprintln!("[Phase 136] Failed to open host capability sled: {}", e);
                None
            }
        }
    });
}

/// Phase 136: Write-through a host capability to sled (non-blocking best-effort).
fn persist_host_capability(key: &str, state: &HostCapabilityState) {
    if let Some(Some(db)) = HOST_CAPABILITY_SLED.get() {
        if let Ok(val) = serde_json::to_vec(state) {
            let _ = db.insert(key.as_bytes(), val);
            // Async flush — don't block the hot path
            let db = db.clone();
            tokio::spawn(async move { let _ = db.flush_async().await; });
        }
    }
}

fn download_host_capabilities() -> &'static StdRwLock<HashMap<String, HostCapabilityState>> {
    DOWNLOAD_HOST_CAPABILITIES.get_or_init(|| StdRwLock::new(HashMap::new()))
}

fn download_active_host_connections() -> &'static Mutex<HashMap<String, usize>> {
    DOWNLOAD_ACTIVE_HOST_CONNECTIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn host_capability_key(url: &str, is_onion: bool) -> Option<String> {
    let parsed = reqwest::Url::parse(url).ok()?;
    let host = parsed.host_str()?.to_ascii_lowercase();
    let port = parsed.port_or_known_default().unwrap_or(0);
    Some(format!(
        "{}|{}|{}|{}",
        if is_onion { "onion" } else { "direct" },
        parsed.scheme(),
        host,
        port
    ))
}

fn current_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn productive_host_freshness_secs(is_onion: bool) -> u64 {
    if is_onion {
        env_usize("CRAWLI_DOWNLOAD_PRODUCTIVE_HOST_MEMORY_SECS_ONION")
            .unwrap_or(120)
            .clamp(30, 600) as u64
    } else {
        env_usize("CRAWLI_DOWNLOAD_PRODUCTIVE_HOST_MEMORY_SECS_CLEARNET")
            .unwrap_or(300)
            .clamp(30, 1_800) as u64
    }
}

fn host_is_productive(
    recent_successes: u32,
    recent_failures: u32,
    consecutive_connect_failures: u32,
    last_productive_epoch_ms: u64,
    is_onion: bool,
    now_ms: u64,
) -> bool {
    if recent_successes == 0 || last_productive_epoch_ms == 0 {
        return false;
    }

    let freshness_window_ms = productive_host_freshness_secs(is_onion) * 1_000;
    let fresh = now_ms.saturating_sub(last_productive_epoch_ms) <= freshness_window_ms;
    fresh
        && recent_failures <= recent_successes.saturating_add(1)
        && consecutive_connect_failures < 2
}

fn update_ewma(target: &mut f64, sample_ms: u64) {
    let sample = sample_ms as f64;
    if *target <= 0.0 {
        *target = sample;
    } else {
        *target = (*target * 0.7) + (sample * 0.3);
    }
}

fn probe_quarantine_base_secs(is_onion: bool) -> u64 {
    if is_onion {
        env_usize("CRAWLI_DOWNLOAD_PROBE_QUARANTINE_SECS_ONION")
            .unwrap_or(18)
            .clamp(8, 120) as u64
    } else {
        env_usize("CRAWLI_DOWNLOAD_PROBE_QUARANTINE_SECS_CLEARNET")
            .unwrap_or(0)
            .clamp(0, 30) as u64
    }
}

fn host_capability_snapshot(url: &str, is_onion: bool) -> Option<HostCapabilityState> {
    let key = host_capability_key(url, is_onion)?;
    let guard = download_host_capabilities().read().ok()?;
    guard.get(&key).cloned()
}

fn record_probe_host_capability(url: &str, is_onion: bool, probe: &ProbeResult, connect_ms: u64) {
    let Some(key) = host_capability_key(url, is_onion) else {
        return;
    };
    let Ok(mut guard) = download_host_capabilities().write() else {
        return;
    };
    let state = guard.entry(key.clone()).or_default();
    state.supports_ranges = Some(probe.supports_ranges);
    state.validator_kind = if probe.etag.is_some() {
        ResumeValidatorKind::Etag
    } else if probe.last_modified.is_some() {
        ResumeValidatorKind::LastModified
    } else {
        ResumeValidatorKind::None
    };
    update_ewma(&mut state.connect_rtt_ewma_ms, connect_ms);
    if probe.content_length > 0 {
        state.recent_successes = state.recent_successes.saturating_add(1);
        state.recent_failures = 0;
        state.consecutive_connect_failures = 0;
        state.quarantine_until_epoch_ms = 0;
        state.last_productive_epoch_ms = current_epoch_ms();
    }
    // Phase 136: write-through to sled
    persist_host_capability(&key, state);
}

fn record_host_success(
    url: &str,
    is_onion: bool,
    bytes_transferred: u64,
    first_byte_ms: Option<u64>,
) {
    let Some(key) = host_capability_key(url, is_onion) else {
        return;
    };
    let Ok(mut guard) = download_host_capabilities().write() else {
        return;
    };
    let state = guard.entry(key.clone()).or_default();
    state.recent_successes = state.recent_successes.saturating_add(1);
    state.recent_failures = 0;
    state.consecutive_low_speed_aborts = 0;
    state.consecutive_connect_failures = 0;
    state.quarantine_until_epoch_ms = 0;
    if let Some(first_byte_ms) = first_byte_ms {
        update_ewma(&mut state.first_byte_ewma_ms, first_byte_ms);
    }
    if bytes_transferred >= 32 * 1024 {
        state.last_productive_epoch_ms = current_epoch_ms();
        state.safe_parallelism_cap = state
            .safe_parallelism_cap
            .saturating_add(1)
            .min(if is_onion { 16 } else { 32 });
    }
    // Phase 136: write-through to sled (only on productive transfers to avoid churn)
    if bytes_transferred >= 32 * 1024 {
        persist_host_capability(&key, state);
    }
}

fn record_host_failure(url: &str, is_onion: bool, kind: HostFailureKind) {
    let Some(key) = host_capability_key(url, is_onion) else {
        return;
    };
    let Ok(mut guard) = download_host_capabilities().write() else {
        return;
    };
    let state = guard.entry(key).or_default();
    state.recent_failures = state.recent_failures.saturating_add(1);
    let now_ms = current_epoch_ms();
    let recently_productive = host_is_productive(
        state.recent_successes,
        state.recent_failures.saturating_sub(1),
        state.consecutive_connect_failures,
        state.last_productive_epoch_ms,
        is_onion,
        now_ms,
    );
    match kind {
        HostFailureKind::LowSpeed => {
            state.low_speed_aborts = state.low_speed_aborts.saturating_add(1);
            state.consecutive_low_speed_aborts =
                state.consecutive_low_speed_aborts.saturating_add(1);
            state.safe_parallelism_cap = state.safe_parallelism_cap.saturating_sub(1).max(1);
            if is_onion && state.consecutive_low_speed_aborts >= 2 {
                let quarantine_secs = probe_quarantine_base_secs(true).max(12);
                state.quarantine_until_epoch_ms = state
                    .quarantine_until_epoch_ms
                    .max(now_ms + quarantine_secs * 1_000);
            }
        }
        HostFailureKind::Connect | HostFailureKind::Timeout => {
            state.consecutive_low_speed_aborts = 0;
            state.consecutive_connect_failures =
                state.consecutive_connect_failures.saturating_add(1);
            state.recent_successes = state.recent_successes.saturating_sub(1);
            state.safe_parallelism_cap = state.safe_parallelism_cap.saturating_sub(1).max(1);
            if is_onion {
                if !recently_productive || state.consecutive_connect_failures >= 2 {
                    state.last_productive_epoch_ms = 0;
                }
                if !recently_productive || state.consecutive_connect_failures >= 1 {
                    let streak = state.consecutive_connect_failures.clamp(1, 5) as u64;
                    let base_secs = probe_quarantine_base_secs(true);
                    let multiplier = if recently_productive {
                        streak.saturating_add(1)
                    } else {
                        streak.saturating_add(2)
                    }
                    .clamp(2, 6);
                    let quarantine_secs = base_secs.saturating_mul(multiplier.max(1));
                    state.quarantine_until_epoch_ms = state
                        .quarantine_until_epoch_ms
                        .max(now_ms + quarantine_secs * 1_000);
                }
            }
        }
    }
}

fn onion_reuse_allowed_for_host(
    url: &str,
    telemetry: Option<&crate::runtime_metrics::RuntimeTelemetry>,
) -> bool {
    let Some(key) = host_capability_key(url, true) else {
        return false;
    };
    let Ok(guard) = download_host_capabilities().read() else {
        return false;
    };
    let Some(state) = guard.get(&key) else {
        return false;
    };
    if let Some(telemetry) = telemetry {
        telemetry.record_download_host_cache_hit();
    }
    state.reuse_allowed(true)
}

fn configured_download_host_connection_cap(is_onion: bool) -> usize {
    if is_onion {
        env_usize("CRAWLI_DOWNLOAD_MAX_HOST_CONNECTIONS_ONION")
            .unwrap_or(DEFAULT_DOWNLOAD_MAX_HOST_CONNECTIONS_ONION)
            .clamp(1, 32)
    } else {
        env_usize("CRAWLI_DOWNLOAD_MAX_HOST_CONNECTIONS_CLEARNET")
            .unwrap_or(DEFAULT_DOWNLOAD_MAX_HOST_CONNECTIONS_CLEARNET)
            .clamp(1, 64)
    }
}

pub fn download_host_connection_cap_for_url(url: &str, is_onion: bool) -> usize {
    let configured_cap = configured_download_host_connection_cap(is_onion);
    let Some(key) = host_capability_key(url, is_onion) else {
        return configured_cap;
    };
    let Ok(guard) = download_host_capabilities().read() else {
        return configured_cap;
    };
    guard
        .get(&key)
        .map(|state| state.safe_parallelism_cap.max(1).min(configured_cap))
        .unwrap_or(configured_cap)
}

struct DownloadHostPermit {
    key: Option<String>,
}

impl Drop for DownloadHostPermit {
    fn drop(&mut self) {
        let Some(key) = self.key.as_ref() else {
            return;
        };
        let Ok(mut guard) = download_active_host_connections().lock() else {
            return;
        };
        if let Some(active) = guard.get_mut(key) {
            *active = active.saturating_sub(1);
            if *active == 0 {
                guard.remove(key);
            }
        }
    }
}

fn try_acquire_download_host_permit(url: &str, is_onion: bool) -> Option<DownloadHostPermit> {
    let Some(key) = host_capability_key(url, is_onion) else {
        return Some(DownloadHostPermit { key: None });
    };
    let cap = download_host_connection_cap_for_url(url, is_onion).max(1);
    let Ok(mut guard) = download_active_host_connections().lock() else {
        return None;
    };
    let active = guard.get(&key).copied().unwrap_or(0);
    if active >= cap {
        return None;
    }
    guard.insert(key.clone(), active + 1);
    Some(DownloadHostPermit { key: Some(key) })
}

async fn acquire_download_host_permit(
    url: &str,
    is_onion: bool,
    control: Option<&DownloadControl>,
) -> Option<DownloadHostPermit> {
    loop {
        if let Some(permit) = try_acquire_download_host_permit(url, is_onion) {
            return Some(permit);
        }
        if let Some(control) = control {
            if control.interruption_reason().is_some() {
                return None;
            }
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

enum QueueDispatchResult {
    Ready(QueuedBatchFile, DownloadHostPermit),
    Saturated,
    Empty,
}

fn take_next_dispatchable_file(
    queue: &mut VecDeque<QueuedBatchFile>,
    is_onion: bool,
) -> QueueDispatchResult {
    if queue.is_empty() {
        return QueueDispatchResult::Empty;
    }

    let scan_len = queue.len();
    for _ in 0..scan_len {
        let Some(file) = queue.pop_front() else {
            break;
        };
        if let Some(permit) = try_acquire_download_host_permit(&file.entry.url, is_onion) {
            return QueueDispatchResult::Ready(file, permit);
        }
        queue.push_back(file);
    }

    QueueDispatchResult::Saturated
}

fn apply_download_connection_policy(
    req: crate::arti_client::ArtiRequestBuilder,
    url: &str,
    is_onion: bool,
    telemetry: Option<&crate::runtime_metrics::RuntimeTelemetry>,
) -> crate::arti_client::ArtiRequestBuilder {
    if !is_onion {
        return req;
    }
    if onion_reuse_allowed_for_host(url, telemetry) {
        req
    } else {
        req.header("Connection", "close")
    }
}

fn download_low_speed_policy(is_onion: bool) -> LowSpeedPolicy {
    if is_onion {
        LowSpeedPolicy {
            min_bytes_per_sec: env_usize("CRAWLI_DOWNLOAD_LOW_SPEED_BPS_ONION").unwrap_or(8 * 1024)
                as u64,
            window: Duration::from_secs(
                env_usize("CRAWLI_DOWNLOAD_LOW_SPEED_WINDOW_SECS_ONION")
                    .unwrap_or(12)
                    .clamp(4, 60) as u64,
            ),
        }
    } else {
        LowSpeedPolicy {
            min_bytes_per_sec: env_usize("CRAWLI_DOWNLOAD_LOW_SPEED_BPS_CLEARNET")
                .unwrap_or(64 * 1024) as u64,
            window: Duration::from_secs(
                env_usize("CRAWLI_DOWNLOAD_LOW_SPEED_WINDOW_SECS_CLEARNET")
                    .unwrap_or(8)
                    .clamp(3, 30) as u64,
            ),
        }
    }
}

fn remap_queued_file_to_next_alternate(
    queued_file: &mut QueuedBatchFile,
) -> Option<(String, String)> {
    let next_url = queued_file
        .entry
        .alternate_urls
        .get(queued_file.alternate_url_cursor)?
        .clone();
    let old_host = batch_entry_host(&queued_file.entry.url)?;
    let new_host = batch_entry_host(&next_url)?;
    if old_host == new_host {
        queued_file.alternate_url_cursor = queued_file.alternate_url_cursor.saturating_add(1);
        queued_file.entry.url = next_url;
        return None;
    }
    queued_file.entry.url = next_url;
    queued_file.alternate_url_cursor = queued_file.alternate_url_cursor.saturating_add(1);
    Some((old_host, new_host))
}

fn stable_probe_rotation_index(key: &str, len: usize) -> usize {
    use std::hash::{Hash, Hasher};

    if len <= 1 {
        return 0;
    }

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut hasher);
    (hasher.finish() as usize) % len
}

#[derive(Clone, Debug)]
struct ProbeCandidate {
    url: String,
    host: String,
    rotation_rank: usize,
    recent_successes: u32,
    recent_failures: u32,
    consecutive_connect_failures: u32,
    last_productive_epoch_ms: u64,
    quarantine_until_epoch_ms: u64,
}

impl ProbeCandidate {
    fn quarantined(&self, now_ms: u64) -> bool {
        self.quarantine_until_epoch_ms > now_ms
    }

    fn productive(&self, is_onion: bool, now_ms: u64) -> bool {
        host_is_productive(
            self.recent_successes,
            self.recent_failures,
            self.consecutive_connect_failures,
            self.last_productive_epoch_ms,
            is_onion,
            now_ms,
        )
    }
}

fn ordered_probe_candidates(entry: &BatchFileEntry, is_onion: bool) -> Vec<ProbeCandidate> {
    let mut urls = Vec::with_capacity(entry.alternate_urls.len().saturating_add(1));
    urls.push(entry.url.clone());
    urls.extend(entry.alternate_urls.iter().cloned());

    let mut seen = HashSet::new();
    urls.retain(|url| seen.insert(url.clone()));

    if urls.len() > 1 {
        let rotation = stable_probe_rotation_index(&entry.path, urls.len());
        urls.rotate_left(rotation);
    }

    let mut candidates = urls
        .into_iter()
        .enumerate()
        .map(|(rotation_rank, url)| {
            let host = batch_entry_host(&url).unwrap_or_else(|| "-".to_string());
            let snapshot = host_capability_snapshot(&url, is_onion).unwrap_or_default();
            ProbeCandidate {
                url,
                host,
                rotation_rank,
                recent_successes: snapshot.recent_successes,
                recent_failures: snapshot.recent_failures,
                consecutive_connect_failures: snapshot.consecutive_connect_failures,
                last_productive_epoch_ms: snapshot.last_productive_epoch_ms,
                quarantine_until_epoch_ms: snapshot.quarantine_until_epoch_ms,
            }
        })
        .collect::<Vec<_>>();

    let now_ms = current_epoch_ms();
    let has_productive_active = candidates
        .iter()
        .any(|candidate| !candidate.quarantined(now_ms) && candidate.productive(is_onion, now_ms));

    candidates.sort_by(|left, right| {
        left.quarantined(now_ms)
            .cmp(&right.quarantined(now_ms))
            .then_with(|| {
                if has_productive_active {
                    right
                        .productive(is_onion, now_ms)
                        .cmp(&left.productive(is_onion, now_ms))
                        .then_with(|| right.recent_successes.cmp(&left.recent_successes))
                        .then_with(|| {
                            right
                                .last_productive_epoch_ms
                                .cmp(&left.last_productive_epoch_ms)
                        })
                        .then_with(|| left.recent_failures.cmp(&right.recent_failures))
                } else {
                    let left_is_snapshot = left.url == entry.url;
                    let right_is_snapshot = right.url == entry.url;
                    right_is_snapshot
                        .cmp(&left_is_snapshot)
                        .then_with(|| left.rotation_rank.cmp(&right.rotation_rank))
                        .then_with(|| left.recent_failures.cmp(&right.recent_failures))
                }
            })
            .then_with(|| left.host.cmp(&right.host))
    });

    candidates
}

fn reseed_probe_alternates(
    entry: &mut BatchFileEntry,
    ordered: &[ProbeCandidate],
    selected_url: &str,
) {
    let mut next = Vec::with_capacity(ordered.len().saturating_sub(1));
    let mut seen = HashSet::new();
    for candidate in ordered {
        if candidate.url == selected_url || !seen.insert(candidate.url.clone()) {
            continue;
        }
        next.push(candidate.url.clone());
    }
    entry.alternate_urls = next;
}

fn diversify_first_wave_by_host(
    files: Vec<ScheduledBatchFile>,
    wave_width: usize,
) -> Vec<ScheduledBatchFile> {
    if files.len() <= 1 || wave_width <= 1 {
        return files;
    }

    let original = files;
    let mut host_buckets: HashMap<String, VecDeque<usize>> = HashMap::new();
    let mut host_order = Vec::new();
    let mut unknown_indices = VecDeque::new();

    for (idx, file) in original.iter().enumerate() {
        if let Some(host) = batch_entry_host(&file.entry.url) {
            let bucket = host_buckets.entry(host.clone()).or_insert_with(|| {
                host_order.push(host.clone());
                VecDeque::new()
            });
            bucket.push_back(idx);
        } else {
            unknown_indices.push_back(idx);
        }
    }

    let mut first_wave = Vec::new();
    let target = wave_width.min(original.len());

    while first_wave.len() < target {
        let mut advanced = false;
        for host in &host_order {
            if first_wave.len() >= target {
                break;
            }
            if let Some(bucket) = host_buckets.get_mut(host) {
                if let Some(idx) = bucket.pop_front() {
                    first_wave.push(idx);
                    advanced = true;
                }
            }
        }
        if first_wave.len() >= target {
            break;
        }
        if let Some(idx) = unknown_indices.pop_front() {
            first_wave.push(idx);
            advanced = true;
        }
        if !advanced {
            break;
        }
    }

    if first_wave.is_empty() {
        return original;
    }

    let picked: HashSet<usize> = first_wave.iter().copied().collect();
    let mut diversified = Vec::with_capacity(original.len());
    for idx in first_wave {
        diversified.push(original[idx].clone());
    }
    for (idx, file) in original.into_iter().enumerate() {
        if !picked.contains(&idx) {
            diversified.push(file);
        }
    }

    diversified
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BatchBodyReadFailure {
    FirstByteTimeout,
    BodyTimeout,
    LowSpeed,
    Stream,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BatchBodyReadStats {
    first_byte_ms: u64,
}

async fn read_batch_response_body(
    resp: crate::arti_client::ArtiResponse,
    first_byte_timeout: Duration,
    body_timeout: Duration,
    low_speed_policy: LowSpeedPolicy,
) -> std::result::Result<(Vec<u8>, BatchBodyReadStats), BatchBodyReadFailure> {
    use futures::StreamExt;

    let started_at = Instant::now();
    let mut stream = resp.bytes_stream();
    let mut body = Vec::new();

    let first_chunk = match tokio::time::timeout(first_byte_timeout, stream.next()).await {
        Ok(Some(Ok(chunk))) if !chunk.is_empty() => chunk,
        Ok(Some(Ok(_))) => return Err(BatchBodyReadFailure::Stream),
        Ok(Some(Err(_))) | Ok(None) => return Err(BatchBodyReadFailure::Stream),
        Err(_) => return Err(BatchBodyReadFailure::FirstByteTimeout),
    };
    let first_byte_ms = started_at.elapsed().as_millis() as u64;
    body.extend_from_slice(&first_chunk);
    let mut low_speed_tracker = LowSpeedTracker::new(Instant::now(), first_chunk.len() as u64);

    let deadline = tokio::time::Instant::now() + body_timeout;
    loop {
        match tokio::time::timeout_at(deadline, stream.next()).await {
            Ok(Some(Ok(chunk))) => {
                if !chunk.is_empty() {
                    let now = Instant::now();
                    if low_speed_tracker.observe_progress(now, chunk.len() as u64, low_speed_policy)
                    {
                        return Err(BatchBodyReadFailure::LowSpeed);
                    }
                    body.extend_from_slice(&chunk);
                }
            }
            Ok(Some(Err(_))) => return Err(BatchBodyReadFailure::Stream),
            Ok(None) => return Ok((body, BatchBodyReadStats { first_byte_ms })),
            Err(_) => {
                if low_speed_tracker.should_abort_on_idle(Instant::now(), low_speed_policy) {
                    return Err(BatchBodyReadFailure::LowSpeed);
                }
                return Err(BatchBodyReadFailure::BodyTimeout);
            }
        }
    }
}

fn publish_download_progress(
    app: &AppHandle,
    progress: crate::telemetry_bridge::BridgeDownloadProgress,
) {
    crate::telemetry_bridge::publish_download_progress(app, progress);
}

// Removed unused ManagedTorProcess and TorProcessGuard
#[derive(Debug)]
enum TaskOutcome {
    Completed,
    Interrupted(&'static str),
    Failed(String),
}

#[derive(Clone, Debug, Serialize)]
pub struct ProbeResult {
    pub content_length: u64,
    pub supports_ranges: bool,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    #[serde(skip_serializing, skip_deserializing, default)]
    prefetched_probe: Option<PrefetchedProbeData>,
}

#[derive(Clone, Debug)]
struct ProbeAttemptResult {
    probe: ProbeResult,
    timed_out: bool,
    request_failed: bool,
}

fn extract_resume_validators(
    resp: &crate::arti_client::ArtiResponse,
) -> (Option<String>, Option<String>) {
    let etag = resp
        .headers()
        .get("etag")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty() && !value.starts_with("W/"))
        .map(|value| value.to_string());
    let last_modified = resp
        .headers()
        .get("last-modified")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string());
    (etag, last_modified)
}

fn preferred_if_range(etag: &Option<String>, last_modified: &Option<String>) -> Option<String> {
    etag.clone().or_else(|| last_modified.clone())
}

fn parse_content_range_total(header_value: &str) -> Option<u64> {
    // Example formats:
    // Standard: "bytes 0-1/1048576" -> 1048576
    // Qilin Masked: "bytes 0-1/*" -> Unknown size

    let total_str = header_value.split('/').next_back()?;

    if total_str.trim() == "*" {
        // The server explicitly supports ranges but masks the total length.
        // We cannot parse an integer, but we must not fail the entire probe.
        return Some(0); // 0 indicates unknown size, triggering stream-mode
    }

    total_str.parse::<u64>().ok()
}

static DIRECT_IO_WARNING_EMITTED: AtomicBool = AtomicBool::new(false);

fn open_file_with_adaptive_io(
    path: &Path,
    read: bool,
    write: bool,
    create: bool,
    truncate: bool,
    app: &AppHandle,
    logger: Option<&DownloadLogger>,
) -> Result<File> {
    fn configure_options(
        opts: &mut OpenOptions,
        read: bool,
        write: bool,
        create: bool,
        truncate: bool,
    ) {
        opts.read(read)
            .write(write)
            .create(create)
            .truncate(truncate);
    }

    let mut direct_open_error: Option<std::io::Error> = None;
    let mut attempted_direct = false;

    if crate::io_vanguard::should_try_direct_io() {
        attempted_direct = true;
        let mut opts = OpenOptions::new();
        configure_options(&mut opts, read, write, create, truncate);
        let _ = crate::io_vanguard::apply_direct_io_if_enabled(&mut opts);
        match opts.open(path) {
            Ok(file) => {
                crate::io_vanguard::post_open_config(&file);
                return Ok(file);
            }
            Err(err) => {
                direct_open_error = Some(err);
            }
        }
    }

    if let Some(err) = direct_open_error {
        crate::io_vanguard::mark_direct_io_degraded();
        if !DIRECT_IO_WARNING_EMITTED.swap(true, Ordering::Relaxed) {
            let warn = format!(
                "[IO] Direct I/O open failed for {} ({}). Falling back to buffered writes (policy={}).",
                path.display(),
                err,
                crate::io_vanguard::direct_io_policy_label()
            );
            let _ = app.emit("log", warn.clone());
            if let Some(log) = logger {
                log.log(app, warn);
            }
        }
        if crate::io_vanguard::direct_io_policy() == crate::io_vanguard::DirectIoPolicy::Always {
            return Err(anyhow!(
                "direct I/O policy is 'always' and open failed for {}: {}",
                path.display(),
                err
            ));
        }
    } else if attempted_direct && crate::io_vanguard::is_direct_io_degraded() {
        // Keep state explicit in logs when auto mode is already degraded.
        let _ = app.emit(
            "log",
            format!(
                "[IO] Direct I/O remains degraded; using buffered writes for {}.",
                path.display()
            ),
        );
    }

    let mut fallback_opts = OpenOptions::new();
    configure_options(&mut fallback_opts, read, write, create, truncate);
    let file = fallback_opts.open(path)?;
    crate::io_vanguard::post_open_config(&file);
    Ok(file)
}

// Phase 44: Legacy terminate_pid/cleanup_tor_data_dir/cleanup_stale_tor_daemons
// removed — arti manages Tor in-process, no child processes to clean.

fn onion_probe_timeout_for_attempt(attempt: usize, has_alternates: bool) -> Duration {
    let base_secs = env_usize("CRAWLI_BATCH_PROBE_TIMEOUT_SECS")
        .unwrap_or(8)
        .clamp(5, 30) as u64;
    if !has_alternates {
        return Duration::from_secs(base_secs.max(10));
    }

    let graded_secs = base_secs
        .saturating_add((attempt as u64).saturating_mul(4))
        .clamp(base_secs, 24);
    Duration::from_secs(graded_secs)
}

fn probe_timeout_for_attempt(url: &str, attempt: usize, has_alternates: bool) -> Duration {
    if crate::url_targets_onion(url) {
        onion_probe_timeout_for_attempt(attempt, has_alternates)
    } else {
        Duration::from_secs(8)
    }
}

async fn collect_prefetched_probe_data(
    resp: crate::arti_client::ArtiResponse,
    promotion_cap: usize,
    read_timeout: Duration,
    content_length: u64,
) -> Option<PrefetchedProbeData> {
    if promotion_cap == 0 {
        return None;
    }

    use futures::StreamExt;

    let mut stream = resp.bytes_stream();
    let mut collected = Vec::with_capacity(promotion_cap.min(PROBE_SIZE as usize));
    let deadline = tokio::time::Instant::now() + read_timeout;
    loop {
        if collected.len() >= promotion_cap {
            break;
        }
        match tokio::time::timeout_at(deadline, stream.next()).await {
            Ok(Some(Ok(chunk))) => {
                if chunk.is_empty() {
                    continue;
                }
                let remaining = promotion_cap.saturating_sub(collected.len());
                if remaining == 0 {
                    break;
                }
                let take = chunk.len().min(remaining);
                collected.extend_from_slice(&chunk[..take]);
                if take < chunk.len() {
                    break;
                }
            }
            Ok(Some(Err(_))) | Ok(None) | Err(_) => break,
        }
    }

    if collected.is_empty() {
        return None;
    }

    let end_offset = collected.len().saturating_sub(1) as u64;
    Some(PrefetchedProbeData {
        complete_file: content_length > 0 && collected.len() as u64 >= content_length,
        end_offset,
        bytes: bytes::Bytes::from(collected),
    })
}

async fn probe_target_with_timeout(
    client: &crate::arti_client::ArtiClient,
    url: &str,
    app: &AppHandle,
    timeout: Duration,
    promotion_cap: usize,
) -> Result<ProbeAttemptResult> {
    let is_onion = crate::url_targets_onion(url);
    let mut content_length = 0u64;
    let mut supports_ranges = false;
    let mut etag = None;
    let mut last_modified = None;
    let mut timed_out = false;
    let mut request_failed = false;
    let mut prefetched_probe = None;
    let request_started = Instant::now();
    let promotion_end = promotion_cap.saturating_sub(1).max(0);
    let range_header_value = format!("bytes=0-{promotion_end}");

    let telemetry = telemetry_handle(app);
    let request = apply_download_connection_policy(
        client.get(url).header("Range", &range_header_value),
        url,
        is_onion,
        telemetry.as_ref(),
    );
    let _host_permit = acquire_download_host_permit(url, is_onion, None).await;

    match tokio::time::timeout(timeout, request.send()).await {
        Ok(Ok(resp)) => {
            let status = resp.status();
            if status == StatusCode::PARTIAL_CONTENT {
                supports_ranges = true;
            }
            if etag.is_none() && last_modified.is_none() {
                (etag, last_modified) = extract_resume_validators(&resp);
            }

            if let Some(value) = resp
                .headers()
                .get(CONTENT_RANGE)
                .and_then(|value| value.to_str().ok())
            {
                if let Some(total) = parse_content_range_total(value) {
                    content_length = total;
                }
            }

            if content_length == 0 {
                content_length = resp.content_length().unwrap_or(0);
            }

            let connect_ms = request_started.elapsed().as_millis() as u64;
            let read_timeout = if is_onion {
                Duration::from_secs(5)
            } else {
                Duration::from_secs(3)
            };
            if matches!(status, StatusCode::PARTIAL_CONTENT | StatusCode::OK) {
                prefetched_probe = collect_prefetched_probe_data(
                    resp,
                    promotion_cap,
                    read_timeout,
                    content_length,
                )
                .await;
            }
            let probe = ProbeResult {
                content_length,
                supports_ranges: supports_ranges && content_length > 0,
                etag,
                last_modified,
                prefetched_probe,
            };
            record_probe_host_capability(url, is_onion, &probe, connect_ms);
            return Ok(ProbeAttemptResult {
                probe,
                timed_out,
                request_failed,
            });
        }
        Ok(Err(err)) => {
            request_failed = true;
            let _ = app.emit("log", format!("[!] GET Range probe failed: {err}"));
            supports_ranges = false;
            content_length = 0;
            record_host_failure(url, is_onion, HostFailureKind::Connect);
        }
        Err(_) => {
            timed_out = true;
            let _ = app.emit(
                "log",
                format!(
                    "[!] GET Range probe timed out after {}s. Forcing fallback stream mode...",
                    timeout.as_secs()
                ),
            );
            supports_ranges = false;
            content_length = 0;
            record_host_failure(url, is_onion, HostFailureKind::Timeout);
        }
    }

    Ok(ProbeAttemptResult {
        probe: ProbeResult {
            content_length,
            supports_ranges: supports_ranges && content_length > 0,
            etag,
            last_modified,
            prefetched_probe,
        },
        timed_out,
        request_failed,
    })
}

pub async fn probe_target(
    client: &crate::arti_client::ArtiClient,
    url: &str,
    app: &AppHandle,
) -> Result<ProbeResult> {
    Ok(probe_target_with_timeout(
        client,
        url,
        app,
        probe_timeout_for_attempt(url, 0, false),
        probe_promotion_size_bytes(),
    )
    .await?
    .probe)
}

pub async fn probe_target_with_alternates(
    client: &crate::arti_client::ArtiClient,
    entry: &mut BatchFileEntry,
    app: &AppHandle,
) -> Result<ProbeResult> {
    let is_onion = crate::url_targets_onion(&entry.url);
    let telemetry = telemetry_handle(app);
    let ordered_candidates = ordered_probe_candidates(entry, is_onion);
    if ordered_candidates.is_empty() {
        return probe_target(client, &entry.url, app).await;
    }

    let current_host = batch_entry_host(&entry.url).unwrap_or_else(|| "-".to_string());
    let now_ms = current_epoch_ms();
    let quarantined_count = ordered_candidates
        .iter()
        .filter(|candidate| candidate.quarantined(now_ms))
        .count();
    let quarantine_hit = ordered_candidates
        .first()
        .map(|candidate| candidate.url != entry.url && quarantined_count > 0)
        .unwrap_or(false)
        || quarantined_count == ordered_candidates.len();
    if quarantine_hit {
        if let Some(telemetry) = telemetry.as_ref() {
            telemetry.record_download_probe_quarantine_hit();
        }
    }
    if let Some(first_candidate) = ordered_candidates.first() {
        if first_candidate.url != entry.url {
            let _ = app.emit(
                "log",
                format!(
                    "[*] Probe rotation: {} starting {} instead of {} before transfer (quarantined_candidates={}/{})",
                    entry.path,
                    first_candidate.host,
                    current_host,
                    quarantined_count,
                    ordered_candidates.len()
                ),
            );
        }
    }
    if quarantined_count == ordered_candidates.len() {
        let _ = app.emit(
            "log",
            format!(
                "[!] Probe candidate set fully quarantined for {} ({}/{} candidates).",
                entry.path,
                quarantined_count,
                ordered_candidates.len()
            ),
        );
    }

    let has_alternates = ordered_candidates.len() > 1;
    let mut first_attempt: Option<ProbeAttemptResult> = None;

    for (idx, candidate) in ordered_candidates.iter().enumerate() {
        let attempt = probe_target_with_timeout(
            client,
            &candidate.url,
            app,
            probe_timeout_for_attempt(&candidate.url, idx, has_alternates),
            probe_promotion_size_bytes(),
        )
        .await?;
        if first_attempt.is_none() {
            first_attempt = Some(attempt.clone());
        }
        if attempt.probe.content_length == 0 {
            continue;
        }

        if candidate.url != entry.url {
            let first_attempt_ref = first_attempt.as_ref().expect("first attempt recorded");
            let _ = app.emit(
                "log",
                format!(
                    "[*] Probe remap: {} switched {} -> {} before transfer (first_probe_timeout={} first_probe_connect_fail={} selected_probe_timeout={} selected_probe_connect_fail={})",
                    entry.path,
                    current_host,
                    candidate.host,
                    first_attempt_ref.timed_out,
                    first_attempt_ref.request_failed,
                    attempt.timed_out,
                    attempt.request_failed
                ),
            );
        }

        let selected_url = candidate.url.clone();
        entry.url = selected_url.clone();
        reseed_probe_alternates(entry, &ordered_candidates, &selected_url);
        return Ok(attempt.probe);
    }

    let selected_url = ordered_candidates
        .iter()
        .find(|candidate| !candidate.quarantined(now_ms))
        .map(|candidate| candidate.url.clone())
        .unwrap_or_else(|| ordered_candidates[0].url.clone());
    if let Some(telemetry) = telemetry.as_ref() {
        telemetry.record_download_probe_candidate_exhaustion();
    }
    let exhausted_hosts = ordered_candidates
        .iter()
        .map(|candidate| candidate.host.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let _ = app.emit(
        "log",
        format!(
            "[!] Probe candidate exhaustion: {} exhausted {} candidates (quarantined={}/{} hosts=[{}]).",
            entry.path,
            ordered_candidates.len(),
            quarantined_count,
            ordered_candidates.len(),
            exhausted_hosts
        ),
    );
    if selected_url != entry.url {
        let selected_host = batch_entry_host(&selected_url).unwrap_or_else(|| "-".to_string());
        let _ = app.emit(
            "log",
            format!(
                "[*] Probe routing: {} arming {} instead of {} for transfer fallback after exhausted probe candidates.",
                entry.path,
                selected_host,
                current_host
            ),
        );
    }
    entry.url = selected_url.clone();
    reseed_probe_alternates(entry, &ordered_candidates, &selected_url);

    Ok(first_attempt
        .expect("probe candidates should not be empty")
        .probe)
}

fn get_arti_client(
    is_onion: bool,
    circuit_id: usize,
    clients: &[crate::tor_native::SharedTorClient],
) -> Result<ArtiClient> {
    if is_onion {
        if clients.is_empty() {
            return Err(anyhow::anyhow!("No active Tor clients available"));
        }
        let shared_client = &clients[circuit_id % clients.len()];
        let tor_client = shared_client.read().unwrap().clone();
        let isolation_token = arti_client::IsolationToken::new();
        Ok(crate::arti_client::ArtiClient::new(
            (*tor_client).clone(),
            Some(isolation_token),
        ))
    } else {
        Ok(crate::arti_client::ArtiClient::new_clearnet())
    }
}

// ===== BATCH FILE DOWNLOAD =====
// For downloading many files with a persistent circuit pool.
// Circuits are created ONCE, tournament runs ONCE, then all files
// are routed through the pool concurrently.

#[derive(Serialize, Deserialize, Clone)]
pub struct BatchFileEntry {
    pub url: String,
    pub path: String,
    pub size_hint: Option<u64>,
    pub jwt_exp: Option<u64>,
    #[serde(default)]
    pub alternate_urls: Vec<String>,
}

#[derive(Clone, Serialize)]
pub struct BatchProgressEvent {
    pub completed: usize,
    pub failed: usize,
    pub total: usize,
    pub current_file: String,
    pub speed_mbps: f64,
    pub downloaded_bytes: u64,
    pub active_circuits: Option<usize>,
}

/// Files at or below the micro threshold are fetched whole, one per request, inside the
/// background micro swarm. Mid-size files stay in the small-file swarm. Files above the large
/// threshold enter the parallel range pipeline.
const DEFAULT_BATCH_MICRO_THRESHOLD: u64 = 5 * 1_048_576; // 5MB
const DEFAULT_BATCH_LARGE_THRESHOLD_CLEARNET: u64 = 100 * 1_048_576; // 100MB
const DEFAULT_BATCH_LARGE_THRESHOLD_ONION: u64 = 24 * 1_048_576; // 24MB
const DEFAULT_BATCH_LARGE_THRESHOLD_ONION_HEAVY: u64 = 16 * 1_048_576; // 16MB

#[derive(Clone)]
struct ScheduledBatchFile {
    entry: BatchFileEntry,
    estimated_size: u64,
    enqueue_order: usize,
    prefetched_probe: Option<PrefetchedProbeData>,
}

#[derive(Clone)]
struct QueuedBatchFile {
    entry: BatchFileEntry,
    estimated_size: u64,
    requeue_count: u8,
    alternate_url_cursor: usize,
    prefetched_probe: Option<PrefetchedProbeData>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BatchLanePlan {
    micro_parallelism: usize,
    small_parallelism: usize,
    large_pipeline_circuits: usize,
    overlap_large_phase: bool,
}

impl BatchLanePlan {
    fn serial(budget: &crate::resource_governor::DownloadBudget) -> Self {
        Self {
            micro_parallelism: budget.micro_swarm_circuits.max(1),
            small_parallelism: budget.small_file_parallelism.max(1),
            large_pipeline_circuits: budget.circuit_cap.max(1),
            overlap_large_phase: false,
        }
    }
}

fn plan_batch_lanes(
    budget: &crate::resource_governor::DownloadBudget,
    is_onion: bool,
    total_files: usize,
    large_files: &[BatchFileEntry],
) -> BatchLanePlan {
    let mut plan = BatchLanePlan::serial(budget);

    if !is_onion || large_files.is_empty() || budget.circuit_cap < 8 {
        return plan;
    }

    let known_large_bytes: u64 = large_files.iter().filter_map(|file| file.size_hint).sum();
    let enough_large_work =
        large_files.len() >= 2 || known_large_bytes >= 128 * 1_048_576 || total_files >= 1_024;

    if !enough_large_work {
        return plan;
    }

    // Phase 133: Large pipeline clamp driven by DownloadMode.
    // Low=(2,8), Medium=(4,16), Aggressive=(6,24).
    let mode = crate::resource_governor::active_download_mode();
    let large_pipeline_circuits = if is_onion {
        let (clamp_min, clamp_max) = mode.large_pipeline_clamp();
        (budget.circuit_cap / 3)
            .clamp(clamp_min, clamp_max)
            .min(budget.circuit_cap.saturating_sub(2).max(1))
    } else {
        (budget.circuit_cap / 4)
            .clamp(3, 4)
            .min(budget.circuit_cap.saturating_sub(2).max(1))
    };
    let residual_parallelism = budget
        .circuit_cap
        .saturating_sub(large_pipeline_circuits)
        .max(2);
    let micro_parallelism = budget
        .micro_swarm_circuits
        .min(residual_parallelism.div_ceil(2))
        .max(1);
    let small_parallelism = budget
        .small_file_parallelism
        .min(
            residual_parallelism
                .saturating_sub(micro_parallelism)
                .max(1),
        )
        .max(1);

    plan.micro_parallelism = micro_parallelism;
    plan.small_parallelism = small_parallelism;
    plan.large_pipeline_circuits = large_pipeline_circuits;
    plan.overlap_large_phase = true;
    plan
}

fn srpt_scheduler_enabled() -> bool {
    match std::env::var("CRAWLI_BATCH_SRPT") {
        Ok(value) => {
            let normalized = value.to_ascii_lowercase();
            normalized == "1" || normalized == "true" || normalized == "on" || normalized == "yes"
        }
        Err(_) => true,
    }
}

fn srpt_starvation_interval() -> usize {
    std::env::var("CRAWLI_BATCH_STARVATION_INTERVAL")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(12)
}

fn env_threshold_bytes(name: &str) -> Option<u64> {
    env_usize(name).map(|value| value as u64 * 1_048_576)
}

fn batch_micro_threshold_bytes() -> u64 {
    env_threshold_bytes("CRAWLI_BATCH_MICRO_THRESHOLD_MIB")
        .unwrap_or(DEFAULT_BATCH_MICRO_THRESHOLD)
        .max(1_048_576)
}

fn batch_large_threshold_bytes(
    is_onion: bool,
    file_count: usize,
    requested_circuits: usize,
) -> u64 {
    let micro_threshold = batch_micro_threshold_bytes();
    let configured = env_threshold_bytes("CRAWLI_BATCH_LARGE_THRESHOLD_MIB");
    let default_threshold = if is_onion {
        if file_count >= 1024 || requested_circuits >= 16 {
            DEFAULT_BATCH_LARGE_THRESHOLD_ONION_HEAVY
        } else {
            DEFAULT_BATCH_LARGE_THRESHOLD_ONION
        }
    } else {
        DEFAULT_BATCH_LARGE_THRESHOLD_CLEARNET
    };

    configured
        .unwrap_or(default_threshold)
        .max(micro_threshold.saturating_add(1_048_576))
}

fn schedule_srpt_with_starvation(mut files: Vec<ScheduledBatchFile>) -> Vec<ScheduledBatchFile> {
    if files.len() <= 1 || !srpt_scheduler_enabled() {
        files.sort_by_key(|file| file.enqueue_order);
        return files;
    }

    let starvation_interval = srpt_starvation_interval();
    let mut dispatch_order = Vec::with_capacity(files.len());
    let mut dispatched = 0usize;

    while !files.is_empty() {
        let use_starvation_guard = dispatched > 0 && dispatched.is_multiple_of(starvation_interval);
        let selected_idx = if use_starvation_guard {
            files
                .iter()
                .enumerate()
                .min_by_key(|(_, file)| file.enqueue_order)
                .map(|(idx, _)| idx)
                .unwrap_or(0)
        } else {
            files
                .iter()
                .enumerate()
                .min_by_key(|(_, file)| (file.estimated_size.max(1), file.enqueue_order))
                .map(|(idx, _)| idx)
                .unwrap_or(0)
        };
        let selected = files.swap_remove(selected_idx);
        dispatch_order.push(selected);
        dispatched += 1;
    }

    dispatch_order
}

async fn process_swarm(
    phase_name: &str,
    app: AppHandle,
    files: Vec<ScheduledBatchFile>,
    parallelism: usize,
    active_ports: Vec<u16>,
    daemon_count: usize,
    is_onion: bool,
    overall_completed: Arc<AtomicUsize>,
    overall_failed: Arc<AtomicUsize>,
    overall_downloaded_bytes: Arc<AtomicU64>,
    active_batch_circuits: Arc<AtomicUsize>,
    control: DownloadControl,
    batch_telemetry: Option<crate::runtime_metrics::RuntimeTelemetry>,
    _jwt_cache: Arc<tokio::sync::RwLock<std::collections::HashMap<String, String>>>,
    total_files: usize,
    active_client_ptrs: Vec<crate::tor_native::SharedTorClient>,
    batch_started_at: Instant,
) {
    if files.is_empty() {
        return;
    }
    let total_sub = files.len();
    let queue = Arc::new(tokio::sync::Mutex::new(
        files
            .into_iter()
            .map(|scheduled| QueuedBatchFile {
                entry: scheduled.entry,
                estimated_size: scheduled.estimated_size,
                requeue_count: 0,
                alternate_url_cursor: 0,
                prefetched_probe: scheduled.prefetched_probe,
            })
            .collect::<VecDeque<_>>(),
    ));
    let phase_completed = Arc::new(AtomicUsize::new(0));
    let phase_bytes = Arc::new(AtomicU64::new(0));
    let phase_bbr = Arc::new(BbrController::new(parallelism, parallelism));
    let send_timeout = batch_swarm_send_timeout(is_onion);
    let first_byte_timeout = batch_swarm_first_byte_timeout(is_onion);
    let body_timeout = batch_swarm_body_timeout(is_onion);
    let no_byte_requeue_limit = batch_no_byte_requeue_limit();
    let low_speed_policy = download_low_speed_policy(is_onion);
    let host_cap_ceiling = configured_download_host_connection_cap(is_onion);

    let _ = app.emit(
        "log",
        format!(
            "[*] {}: {} files across {} circuits (send_timeout={}s first_byte_timeout={}s body_timeout={}s requeue_limit={} host_cap_ceiling={})",
            phase_name,
            total_sub,
            parallelism,
            send_timeout.as_secs(),
            first_byte_timeout.as_secs(),
            body_timeout.as_secs(),
            no_byte_requeue_limit,
            host_cap_ceiling
        ),
    );

    let mut tasks = JoinSet::new();
    for circuit_id in 0..parallelism {
        // Find which port this circuit maps to (simple round robin based on circuit_id)
        let _daemon_port = active_ports[circuit_id % daemon_count.max(1)] as usize;
        let client = match get_arti_client(is_onion, circuit_id, &active_client_ptrs) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let task_queue = Arc::clone(&queue);
        let task_phase_completed = Arc::clone(&phase_completed);
        let task_phase_bytes = Arc::clone(&phase_bytes);
        let task_overall_completed = Arc::clone(&overall_completed);
        let task_overall_failed = Arc::clone(&overall_failed);
        let task_overall_bytes = Arc::clone(&overall_downloaded_bytes);
        let task_active_batch_circuits = Arc::clone(&active_batch_circuits);
        let task_app = app.clone();
        let task_control = control.clone();
        let task_bbr = Arc::clone(&phase_bbr);
        let task_telemetry = batch_telemetry.clone();
        let task_send_timeout = send_timeout;
        let _task_first_byte_timeout = first_byte_timeout;
        let task_body_timeout = body_timeout;
        let task_is_onion = is_onion;
        let task_no_byte_requeue_limit = no_byte_requeue_limit;
        let task_phase_name = phase_name.to_string();
        let task_low_speed_policy = low_speed_policy;

        tasks.spawn(async move {
            let mut client = client;
            loop {
                if task_control.interruption_reason().is_some() {
                    break;
                }

                let res = {
                    let mut queue = task_queue.lock().await;
                    take_next_dispatchable_file(&mut queue, task_is_onion)
                };
                let (mut queued_file, _host_permit) = match res {
                    QueueDispatchResult::Ready(file, permit) => (file, permit),
                    QueueDispatchResult::Saturated => {
                        tokio::time::sleep(Duration::from_millis(25)).await;
                        continue;
                    }
                    QueueDispatchResult::Empty => break,
                };
                let entry = queued_file.entry.clone();

                if let Some(dir) = Path::new(&entry.path).parent() {
                    let _ = fs::create_dir_all(dir);
                }

                let _active_guard = ActiveCircuitGuard::new(Arc::clone(&task_active_batch_circuits));
                let mut retries = 0;
                let mut success = false;
                let mut downloaded_len = 0u64;
                let mut should_requeue = false;
                if let Some(prefetched) = queued_file.prefetched_probe.take() {
                    let expected_size = queued_file.estimated_size.max(prefetched.len() as u64);
                    if prefetched.complete_file
                        || expected_size > 0 && prefetched.len() as u64 >= expected_size
                    {
                        if fs::write(&entry.path, &prefetched.bytes).is_ok() {
                            downloaded_len = prefetched.len() as u64;
                            success = true;
                            if let Some(telemetry) = &task_telemetry {
                                telemetry.record_download_probe_promotion_hit();
                            }
                            record_host_success(
                                &entry.url,
                                task_is_onion,
                                downloaded_len,
                                Some(0),
                            );
                        }
                    } else {
                        queued_file.prefetched_probe = Some(prefetched);
                    }
                }
                while retries < 5 && !success {
                    let prefetched_probe = queued_file.prefetched_probe.clone();
                    let followup_start = prefetched_probe
                        .as_ref()
                        .map(|probe| probe.next_offset())
                        .filter(|offset| *offset < queued_file.estimated_size);
                    let request_started = Instant::now();
                    let mut req = client.get(&entry.url);
                    if let Some(start) = followup_start {
                        req = req.header(
                            "Range",
                            &format!("bytes={start}-{}", queued_file.estimated_size.saturating_sub(1)),
                        );
                    }
                    req = apply_download_connection_policy(
                        req,
                        &entry.url,
                        task_is_onion,
                        task_telemetry.as_ref(),
                    );
                    let resp = match tokio::time::timeout(task_send_timeout, req.send()).await {
                        Ok(Ok(r)) if r.status().is_success() => r,
                        Ok(Ok(r)) => {
                            let status = r.status();
                            if status == reqwest::StatusCode::TOO_MANY_REQUESTS
                                || status == reqwest::StatusCode::SERVICE_UNAVAILABLE
                            {
                                task_bbr.on_reject();
                                let _ = task_app.emit("log", format!("[🛡] Swarm Evasion: Small-file circuit {} hit HTTP {}. Re-isolating circuit locally...", circuit_id, status));
                                // With isolated_client() native regeneration, no global rotation is needed.
                                // Since we create a single `client` per circuit thread in process_swarm and
                                // don't share it, we don't actually need to rotate the token here if we
                                // regenerate the stream internally, but for pure idempotency we just pause.
                            } else {
                                task_bbr.on_reject();
                            }
                            retries += 1;
                            if task_is_onion && retries >= 2 {
                                client = client.new_isolated();
                            }
                            record_host_failure(&entry.url, task_is_onion, HostFailureKind::Timeout);
                            let active = task_bbr.current_active();
                            let base = backoff_duration(retries);
                            let bbr_pause = if circuit_id >= active {
                                Duration::from_millis(2000)
                            } else {
                                Duration::ZERO
                            };
                            tokio::time::sleep(base + bbr_pause).await;
                            continue;
                        }
                        Ok(Err(err)) => {
                            task_bbr.on_reject();
                            if queued_file.requeue_count < task_no_byte_requeue_limit {
                                let remap_note = remap_queued_file_to_next_alternate(&mut queued_file)
                                    .map(|(from_host, to_host)| {
                                        format!("; remapped {} -> {}", from_host, to_host)
                                    })
                                    .unwrap_or_default();
                                let _ = task_app.emit(
                                    "log",
                                    format!(
                                        "[~] {}: requeueing {} after connection stall on circuit {} ({}){}",
                                        task_phase_name,
                                        crate::path_utils::normalize_windows_device_path(&entry.path),
                                        circuit_id,
                                        err,
                                        remap_note
                                    ),
                                );
                                if task_is_onion {
                                    client = client.new_isolated();
                                }
                                record_host_failure(&entry.url, task_is_onion, HostFailureKind::Connect);
                                should_requeue = true;
                                break;
                            }
                            if err.to_string().contains("connect")
                                || err.to_string().contains("request")
                            {
                                let _ = task_app.emit("log", format!("[🛡] Swarm Evasion: Small-file circuit {} connection reset. Re-isolating circuit locally...", circuit_id));
                            }
                            retries += 1;
                            if task_is_onion && retries >= 2 {
                                client = client.new_isolated();
                            }
                            record_host_failure(&entry.url, task_is_onion, HostFailureKind::Connect);
                            let active = task_bbr.current_active();
                            let base = backoff_duration(retries);
                            let bbr_pause = if circuit_id >= active {
                                Duration::from_millis(2000)
                            } else {
                                Duration::ZERO
                            };
                            tokio::time::sleep(base + bbr_pause).await;
                            continue;
                        }
                        Err(_) => {
                            task_bbr.on_timeout();
                            if queued_file.requeue_count < task_no_byte_requeue_limit {
                                let remap_note = remap_queued_file_to_next_alternate(&mut queued_file)
                                    .map(|(from_host, to_host)| {
                                        format!("; remapped {} -> {}", from_host, to_host)
                                    })
                                    .unwrap_or_default();
                                let _ = task_app.emit(
                                    "log",
                                    format!(
                                        "[~] {}: requeueing {} after send timeout on circuit {}{}",
                                        task_phase_name,
                                        crate::path_utils::normalize_windows_device_path(&entry.path),
                                        circuit_id,
                                        remap_note
                                    ),
                                );
                                if task_is_onion {
                                    client = client.new_isolated();
                                }
                                record_host_failure(&entry.url, task_is_onion, HostFailureKind::Timeout);
                                should_requeue = true;
                                break;
                            }
                            retries += 1;
                            if task_is_onion {
                                client = client.new_isolated();
                            }
                            record_host_failure(&entry.url, task_is_onion, HostFailureKind::Timeout);
                            tokio::time::sleep(backoff_duration(retries)).await;
                            continue;
                        }
                    };
                    let response_status = resp.status();
                    let header_latency_ms = request_started.elapsed().as_millis() as u64;

                    // Phase 126: Per-URL adaptive first-byte timeout from host EWMA.
                    // If the host has recorded first_byte_ewma_ms, we use max(3×ewma, 3s)
                    // capped at the fixed default. Otherwise, use the fixed timeout.
                    let effective_first_byte_timeout = adaptive_first_byte_timeout(
                        &entry.url, task_is_onion,
                    );

                    match read_batch_response_body(
                        resp,
                        effective_first_byte_timeout,
                        task_body_timeout,
                        task_low_speed_policy,
                    )
                    .await
                    {
                        Ok((bytes, read_stats)) => {
                            let had_prefetched = prefetched_probe.is_some();
                            let prefetched_len = prefetched_probe
                                .as_ref()
                                .map(|probe| probe.len() as u64)
                                .unwrap_or(0);
                            let final_bytes = if let Some(prefetched) = prefetched_probe.as_ref() {
                                if response_status == StatusCode::PARTIAL_CONTENT {
                                    let mut combined =
                                        Vec::with_capacity(prefetched.len() + bytes.len());
                                    combined.extend_from_slice(&prefetched.bytes);
                                    combined.extend_from_slice(&bytes);
                                    combined
                                } else {
                                    bytes
                                }
                            } else {
                                bytes
                            };
                            let len = final_bytes.len() as u64;
                            if fs::write(&entry.path, &final_bytes).is_ok() {
                                downloaded_len = len;
                                success = true;
                                task_bbr.on_success(len, 1000);
                                if had_prefetched {
                                    if let Some(telemetry) = &task_telemetry {
                                        telemetry.record_download_probe_promotion_hit();
                                    }
                                }
                                record_host_success(
                                    &entry.url,
                                    task_is_onion,
                                    len.max(prefetched_len),
                                    Some(read_stats.first_byte_ms.max(header_latency_ms)),
                                );
                            }
                        }
                        Err(BatchBodyReadFailure::FirstByteTimeout) => {
                            task_bbr.on_timeout();
                            if queued_file.requeue_count < task_no_byte_requeue_limit {
                                let host =
                                    batch_entry_host(&entry.url).unwrap_or_else(|| "-".to_string());
                                let remap_note = remap_queued_file_to_next_alternate(&mut queued_file)
                                    .map(|(from_host, to_host)| {
                                        format!("; remapped {} -> {}", from_host, to_host)
                                    })
                                    .unwrap_or_default();
                                let _ = task_app.emit(
                                    "log",
                                    format!(
                                        "[~] {}: requeueing no-byte stall for {} on host {} (circuit {}, pass {}/{}){}",
                                        task_phase_name,
                                        crate::path_utils::normalize_windows_device_path(&entry.path),
                                        host,
                                        circuit_id,
                                        queued_file.requeue_count + 1,
                                        task_no_byte_requeue_limit,
                                        remap_note
                                    ),
                                );
                                if task_is_onion {
                                    client = client.new_isolated();
                                }
                                record_host_failure(&entry.url, task_is_onion, HostFailureKind::Timeout);
                                should_requeue = true;
                                break;
                            }
                            retries += 1;
                            if task_is_onion {
                                client = client.new_isolated();
                            }
                            tokio::time::sleep(backoff_duration(retries)).await;
                        }
                        Err(BatchBodyReadFailure::LowSpeed) => {
                            task_bbr.on_timeout();
                            if let Some(telemetry) = &task_telemetry {
                                telemetry.record_download_low_speed_abort();
                            }
                            record_host_failure(&entry.url, task_is_onion, HostFailureKind::LowSpeed);
                            retries += 1;
                            if task_is_onion {
                                client = client.new_isolated();
                            }
                            tokio::time::sleep(backoff_duration(retries)).await;
                        }
                        Err(BatchBodyReadFailure::BodyTimeout | BatchBodyReadFailure::Stream) => {
                            retries += 1;
                            if task_is_onion {
                                client = client.new_isolated();
                            }
                            record_host_failure(&entry.url, task_is_onion, HostFailureKind::Timeout);
                            tokio::time::sleep(backoff_duration(retries)).await;
                        }
                    }
                }

                if success {
                    task_phase_completed.fetch_add(1, Ordering::Relaxed);
                    task_phase_bytes.fetch_add(downloaded_len, Ordering::Relaxed);
                    task_overall_bytes.fetch_add(downloaded_len, Ordering::Relaxed);
                    let completed = task_overall_completed.fetch_add(1, Ordering::Relaxed) + 1;
                    let failed = task_overall_failed.load(Ordering::Relaxed);
                    let overall_bytes = task_overall_bytes.load(Ordering::Relaxed);
                    let speed_mbps = batch_speed_mbps(overall_bytes, batch_started_at);
                    publish_batch_progress(
                        &task_app,
                        BatchProgressEvent {
                            completed,
                            failed,
                            total: total_files,
                            current_file: entry.path.clone(),
                            speed_mbps,
                            downloaded_bytes: overall_bytes,
                            active_circuits: Some(
                                task_active_batch_circuits.load(Ordering::Relaxed),
                            ),
                        },
                    );
                    if let Some(telemetry) = &task_telemetry {
                        telemetry.set_active_circuits(
                            task_active_batch_circuits.load(Ordering::Relaxed),
                        );
                    }
                } else if should_requeue {
                    queued_file.requeue_count = queued_file.requeue_count.saturating_add(1);
                    let mut queue = task_queue.lock().await;
                    queue.push_back(queued_file);
                } else {
                    let failed = task_overall_failed.fetch_add(1, Ordering::Relaxed) + 1;
                    let completed = task_overall_completed.load(Ordering::Relaxed);
                    let overall_bytes = task_overall_bytes.load(Ordering::Relaxed);
                    let speed_mbps = batch_speed_mbps(overall_bytes, batch_started_at);
                    publish_batch_progress(
                        &task_app,
                        BatchProgressEvent {
                            completed,
                            failed,
                            total: total_files,
                            current_file: entry.path.clone(),
                            speed_mbps,
                            downloaded_bytes: overall_bytes,
                            active_circuits: Some(
                                task_active_batch_circuits.load(Ordering::Relaxed),
                            ),
                        },
                    );
                    if let Some(telemetry) = &task_telemetry {
                        telemetry.set_active_circuits(
                            task_active_batch_circuits.load(Ordering::Relaxed),
                        );
                    }
                }
            }
        });
    }

    while tasks.join_next().await.is_some() {}

    let done = phase_completed.load(Ordering::Relaxed);
    let bytes = phase_bytes.load(Ordering::Relaxed);
    let _ = app.emit(
        "log",
        format!(
            "[+] {} complete: {}/{} files ({:.2} GB)",
            phase_name,
            done,
            total_sub,
            bytes as f64 / 1_073_741_824.0
        ),
    );
}

async fn process_large_pipeline(
    app: AppHandle,
    files: Vec<BatchFileEntry>,
    requested_circuits: usize,
    force_tor: bool,
    output_dir: Option<String>,
    control: DownloadControl,
    overall_completed: Arc<AtomicUsize>,
    overall_failed: Arc<AtomicUsize>,
    overall_downloaded_bytes: Arc<AtomicU64>,
    active_batch_circuits: Arc<AtomicUsize>,
    batch_telemetry: Option<crate::runtime_metrics::RuntimeTelemetry>,
    jwt_cache: Arc<tokio::sync::RwLock<std::collections::HashMap<String, String>>>,
    total_files: usize,
    batch_started_at: Instant,
) {
    if files.is_empty() {
        return;
    }

    let _ = app.emit(
        "log",
        format!(
            "[*] Phase 2 (Large): {} files with reserved {}-circuit lane",
            files.len(),
            requested_circuits
        ),
    );

    for (i, file) in files.iter().enumerate() {
        if control.interruption_reason().is_some() {
            break;
        }

        if let Some(cutoff) = env_usize("CRAWLI_DOWNLOAD_CUTOFF_BYTES") {
            if overall_downloaded_bytes.load(Ordering::Relaxed) >= (cutoff as u64) {
                let remaining = total_files
                    .saturating_sub(overall_completed.load(Ordering::Relaxed))
                    .saturating_sub(overall_failed.load(Ordering::Relaxed));
                let _ = app.emit(
                    "log",
                    format!(
                        "[SYSTEM] Download scale limit reached ({} bytes cut-off). Halting remaining {} large files...",
                        cutoff, remaining
                    ),
                );
                break;
            }
        }

        let _ = app.emit(
            "log",
            format!(
                "[*] Phase 2: Large file {}/{}: {}",
                i + 1,
                files.len(),
                file.path
            ),
        );

        publish_batch_progress(
            &app,
            BatchProgressEvent {
                completed: overall_completed.load(Ordering::Relaxed),
                failed: overall_failed.load(Ordering::Relaxed),
                total: total_files,
                current_file: file.path.clone(),
                speed_mbps: batch_speed_mbps(
                    overall_downloaded_bytes.load(Ordering::Relaxed),
                    batch_started_at,
                ),
                downloaded_bytes: overall_downloaded_bytes.load(Ordering::Relaxed),
                active_circuits: Some(active_batch_circuits.load(Ordering::Relaxed)),
            },
        );

        let inner_control = DownloadControl::new();
        let _active_guard =
            ActiveCircuitGuard::with_load(Arc::clone(&active_batch_circuits), requested_circuits);
        let result = start_download(
            app.clone(),
            file.clone(),
            requested_circuits,
            force_tor,
            output_dir.clone(),
            inner_control,
            Arc::clone(&jwt_cache),
        )
        .await;

        match result {
            Ok(()) => {
                let bytes = std::fs::metadata(&file.path)
                    .map(|meta| meta.len())
                    .ok()
                    .or(file.size_hint)
                    .unwrap_or(0);
                if bytes > 0 {
                    overall_downloaded_bytes.fetch_add(bytes, Ordering::Relaxed);
                }
                let completed = overall_completed.fetch_add(1, Ordering::Relaxed) + 1;
                let overall_bytes = overall_downloaded_bytes.load(Ordering::Relaxed);
                publish_batch_progress(
                    &app,
                    BatchProgressEvent {
                        completed,
                        failed: overall_failed.load(Ordering::Relaxed),
                        total: total_files,
                        current_file: file.path.clone(),
                        speed_mbps: batch_speed_mbps(overall_bytes, batch_started_at),
                        downloaded_bytes: overall_bytes,
                        active_circuits: Some(active_batch_circuits.load(Ordering::Relaxed)),
                    },
                );
            }
            Err(err) => {
                let failed = overall_failed.fetch_add(1, Ordering::Relaxed) + 1;
                let _ = app.emit(
                    "log",
                    format!("[!] Large file failed: {} ({})", file.path, err),
                );
                publish_batch_progress(
                    &app,
                    BatchProgressEvent {
                        completed: overall_completed.load(Ordering::Relaxed),
                        failed,
                        total: total_files,
                        current_file: file.path.clone(),
                        speed_mbps: batch_speed_mbps(
                            overall_downloaded_bytes.load(Ordering::Relaxed),
                            batch_started_at,
                        ),
                        downloaded_bytes: overall_downloaded_bytes.load(Ordering::Relaxed),
                        active_circuits: Some(active_batch_circuits.load(Ordering::Relaxed)),
                    },
                );
            }
        }

        if let Some(telemetry) = &batch_telemetry {
            telemetry.set_active_circuits(active_batch_circuits.load(Ordering::Relaxed));
        }
    }
}

async fn wait_for_large_overlap_window(
    overall_completed: &Arc<AtomicUsize>,
    overall_downloaded_bytes: &Arc<AtomicU64>,
    control: &DownloadControl,
) -> bool {
    let min_completed = env_usize("CRAWLI_BATCH_LARGE_OVERLAP_MIN_COMPLETIONS").unwrap_or(24);
    let min_bytes = env_usize("CRAWLI_BATCH_LARGE_OVERLAP_MIN_BYTES")
        .map(|value| value as u64)
        .unwrap_or(32 * 1_048_576);
    let max_wait_secs = env_usize("CRAWLI_BATCH_LARGE_OVERLAP_MAX_WAIT_SECS")
        .unwrap_or(45)
        .max(5) as u64;
    let deadline = Instant::now() + Duration::from_secs(max_wait_secs);

    loop {
        if control.interruption_reason().is_some() {
            return false;
        }

        if overall_completed.load(Ordering::Relaxed) >= min_completed
            || overall_downloaded_bytes.load(Ordering::Relaxed) >= min_bytes
        {
            return true;
        }

        if Instant::now() >= deadline {
            return overall_completed.load(Ordering::Relaxed) > 0
                || overall_downloaded_bytes.load(Ordering::Relaxed) > 0;
        }

        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

pub async fn start_batch_download(
    app: AppHandle,
    files: Vec<BatchFileEntry>,
    num_circuits: usize,
    force_tor: bool,
    output_dir: Option<String>,
    control: DownloadControl,
) -> Result<()> {
    let _download_session_guard =
        telemetry_handle(&app).map(crate::runtime_metrics::DownloadSessionGuard::new);
    let batch_started_at = Instant::now();
    let requested_circuits = num_circuits.max(1);
    let overall_completed = Arc::new(AtomicUsize::new(0));
    let overall_failed = Arc::new(AtomicUsize::new(0));
    let overall_downloaded_bytes = Arc::new(AtomicU64::new(0));
    let active_batch_circuits = Arc::new(AtomicUsize::new(0));
    let batch_telemetry = telemetry_handle(&app);
    let jwt_cache: Arc<tokio::sync::RwLock<std::collections::HashMap<String, String>>> =
        Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()));
    let is_onion = files
        .first()
        .map(|f| crate::url_targets_onion(&f.url))
        .unwrap_or(false)
        || force_tor;
    let batch_download_budget = crate::resource_governor::recommend_download_budget(
        requested_circuits,
        None,
        is_onion,
        output_dir.as_deref().map(Path::new),
        batch_telemetry.as_ref(),
    );
    let batch_circuit_cap = batch_download_budget
        .circuit_cap
        .min(requested_circuits)
        .max(1);
    let small_file_parallelism = batch_download_budget
        .small_file_parallelism
        .min(batch_circuit_cap)
        .max(1);

    // Phase 48: Smart Local Download Validation (Topological Pre-flight)
    let mut pending_files = Vec::with_capacity(files.len());
    let mut skipped_perfect_matches = 0usize;
    let mut completed_bytes_skipped = 0u64;

    for file in files {
        let mut bypass = false;
        if let Some(hint) = file.size_hint {
            if hint > 0 {
                if let Ok(meta) = std::fs::metadata(&file.path) {
                    if meta.len() == hint {
                        bypass = true;
                        skipped_perfect_matches += 1;
                        completed_bytes_skipped += hint;
                    }
                }
            }
        }
        if bypass {
            let _ = app.emit(
                "log",
                format!(
                    "[✓] Smart Skip (Pre-flight): File already fully downloaded locally: {}",
                    file.path
                ),
            );
        } else {
            pending_files.push(file);
        }
    }

    if skipped_perfect_matches > 0 {
        let _ = app.emit(
            "log",
            format!(
                "[✨] Smart Validation Bypassed {} fully downloaded tracking entries.",
                skipped_perfect_matches
            ),
        );
        overall_completed.fetch_add(skipped_perfect_matches, Ordering::Relaxed);
        overall_downloaded_bytes.fetch_add(completed_bytes_skipped, Ordering::Relaxed);
    }

    let files = pending_files;
    let total_files = files.len() + skipped_perfect_matches; // Anchor UI progress bar correctly

    if files.is_empty() {
        let _ = app.emit(
            "log",
            "[✓] All files in batch perfectly match locally. Bypass complete.".to_string(),
        );
        publish_batch_progress(
            &app,
            BatchProgressEvent {
                completed: overall_completed.load(Ordering::Relaxed),
                failed: 0,
                total: total_files,
                current_file: "Batch fully cached".to_string(),
                speed_mbps: 0.0,
                downloaded_bytes: overall_downloaded_bytes.load(Ordering::Relaxed),
                active_circuits: Some(0),
            },
        );
        if let Some(telemetry) = telemetry_handle(&app) {
            telemetry.set_active_circuits(0);
        }
        return Ok(());
    }

    let _ = app.emit(
        "log",
        format!(
            "[*] Batch governor: circuit_cap={} small_parallel={} active_start={} tournament_cap={} pressure={:.2}",
            batch_circuit_cap,
            small_file_parallelism,
            batch_download_budget.initial_active_cap,
            batch_download_budget.tournament_cap,
            batch_download_budget.pressure.total_pressure
        ),
    );

    // Dynamically detect active Tor daemon ports
    let mut active_ports: Vec<u16> = Vec::new();
    let mut active_client_ptrs: Vec<crate::tor_native::SharedTorClient> = Vec::new();

    let daemon_count = if is_onion {
        let state = app.state::<crate::AppState>();
        let mut download_guard = state.download_swarm_guard.lock().await;

        let needs_bootstrap = match download_guard.as_ref() {
            Some(arc) => {
                arc.lock().await.native_swarm.is_none()
                    || arc.lock().await.get_arti_clients().is_empty()
            }
            None => true,
        };

        if needs_bootstrap {
            match crate::tor::bootstrap_tor_cluster_for_traffic(
                app.clone(),
                batch_circuit_cap,
                128,
                crate::tor::SwarmTrafficClass::OnionService,
            )
            .await
            {
                Ok((new_guard, _ports)) => {
                    *download_guard = Some(std::sync::Arc::new(tokio::sync::Mutex::new(new_guard)));
                }
                Err(err) => {
                    return Err(anyhow::anyhow!(
                        "Failed to bootstrap Aria Forge Tor cluster for batch download: {}",
                        err
                    ));
                }
            }
        }

        if let Some(arc) = download_guard.as_ref() {
            active_client_ptrs = arc.lock().await.get_arti_clients();
        }

        let live_clients = active_client_ptrs.len().max(1);
        active_ports = (0..live_clients).map(|idx| idx as u16).collect();
        live_clients
    } else {
        1
    };
    if active_ports.is_empty() {
        active_ports.push(0);
    }

    let micro_threshold = batch_micro_threshold_bytes();
    let large_threshold = batch_large_threshold_bytes(is_onion, files.len(), requested_circuits);

    // -- Probe all files and sort into small vs large --
    let sniff_client = get_arti_client(is_onion, 0, &active_client_ptrs)?;
    let mut micro_candidates: Vec<ScheduledBatchFile> = Vec::new();
    let mut small_candidates: Vec<ScheduledBatchFile> = Vec::new();
    let mut large_candidates: Vec<ScheduledBatchFile> = Vec::new();
    let mut enqueue_order = 0usize;
    let mut promotion_budget_remaining = batch_probe_promotion_budget_bytes();

    let _ = app.emit(
        "log",
        format!(
            "[*] Batch: probing {} files... thresholds micro<={:.1}MB large>{:.1}MB",
            files.len(),
            micro_threshold as f64 / 1_048_576.0,
            large_threshold as f64 / 1_048_576.0
        ),
    );

    for file in &files {
        if control.interruption_reason().is_some() {
            return Ok(());
        }

        // Smart Skip Idempotency (redundant fallback)
        if let Some(hint) = file.size_hint {
            if hint > 0 {
                if hint <= micro_threshold {
                    micro_candidates.push(ScheduledBatchFile {
                        entry: file.clone(),
                        estimated_size: hint,
                        enqueue_order,
                        prefetched_probe: None,
                    });
                } else if hint <= large_threshold {
                    small_candidates.push(ScheduledBatchFile {
                        entry: file.clone(),
                        estimated_size: hint,
                        enqueue_order,
                        prefetched_probe: None,
                    });
                } else {
                    large_candidates.push(ScheduledBatchFile {
                        entry: file.clone(),
                        estimated_size: hint,
                        enqueue_order,
                        prefetched_probe: None,
                    });
                }
                enqueue_order = enqueue_order.saturating_add(1);
                continue;
            }
        }

        let mut probed_file = file.clone();
        match probe_target_with_alternates(&sniff_client, &mut probed_file, &app).await {
            Ok(probe) => {
                let estimated_size = probed_file.size_hint.unwrap_or(probe.content_length);
                probed_file.size_hint = Some(estimated_size);
                let prefetched_probe = probe.prefetched_probe.clone().filter(|seed| {
                    seed.len() <= promotion_budget_remaining && estimated_size <= large_threshold
                });
                if let Some(seed) = &prefetched_probe {
                    promotion_budget_remaining =
                        promotion_budget_remaining.saturating_sub(seed.len());
                }
                if probe.content_length <= micro_threshold {
                    micro_candidates.push(ScheduledBatchFile {
                        entry: probed_file.clone(),
                        estimated_size,
                        enqueue_order,
                        prefetched_probe,
                    });
                } else if probe.content_length <= large_threshold {
                    small_candidates.push(ScheduledBatchFile {
                        entry: probed_file.clone(),
                        estimated_size,
                        enqueue_order,
                        prefetched_probe,
                    });
                } else {
                    large_candidates.push(ScheduledBatchFile {
                        entry: probed_file.clone(),
                        estimated_size,
                        enqueue_order,
                        prefetched_probe: None,
                    });
                }
            }
            Err(_) => small_candidates.push(ScheduledBatchFile {
                entry: probed_file.clone(),
                estimated_size: probed_file
                    .size_hint
                    .unwrap_or(large_threshold.saturating_sub(1)),
                enqueue_order,
                prefetched_probe: None,
            }),
        }
        enqueue_order = enqueue_order.saturating_add(1);
    }

    let scheduler_enabled = srpt_scheduler_enabled();
    let starvation_interval = srpt_starvation_interval();
    let micro_files = schedule_srpt_with_starvation(micro_candidates);
    let small_files = schedule_srpt_with_starvation(small_candidates);
    let large_files = schedule_srpt_with_starvation(large_candidates);
    let large_file_entries = large_files
        .iter()
        .map(|file| file.entry.clone())
        .collect::<Vec<_>>();
    let lane_plan = plan_batch_lanes(
        &batch_download_budget,
        is_onion,
        total_files,
        &large_file_entries,
    );
    let micro_files = diversify_first_wave_by_host(
        micro_files,
        batch_first_wave_width(lane_plan.micro_parallelism),
    );
    let small_files = diversify_first_wave_by_host(
        small_files,
        batch_first_wave_width(lane_plan.small_parallelism),
    );

    let _ = app.emit(
        "log",
        format!(
            "[*] Batch scheduler: {} (starvation guard every {} picks)",
            if scheduler_enabled {
                "SRPT+Aging"
            } else {
                "FIFO"
            },
            starvation_interval
        ),
    );

    let _ = app.emit(
        "log",
        format!(
            "[+] Batch routing: {} micro (bg) + {} small (concurrent) + {} large ({})",
            micro_files.len(),
            small_files.len(),
            large_files.len(),
            if lane_plan.overlap_large_phase {
                "overlapped pipeline"
            } else {
                "serial pipeline"
            }
        ),
    );

    let _ = app.emit(
        "log",
        format!(
            "[*] Batch lane plan: micro_parallel={} small_parallel={} large_lane={} overlap={}",
            lane_plan.micro_parallelism,
            lane_plan.small_parallelism,
            lane_plan.large_pipeline_circuits,
            lane_plan.overlap_large_phase
        ),
    );

    // -- Phase 0: Download micro files strictly < 5MB concurrently in background --
    let micro_swarm_handle = if !micro_files.is_empty() {
        let app_clone = app.clone();
        let micro_files_clone = micro_files.clone();
        let active_ports_clone = active_ports.clone();
        let is_onion_clone = is_onion;
        let overall_completed_clone = Arc::clone(&overall_completed);
        let overall_failed_clone = Arc::clone(&overall_failed);
        let overall_downloaded_bytes_clone = Arc::clone(&overall_downloaded_bytes);
        let active_batch_circuits_clone = Arc::clone(&active_batch_circuits);
        let control_clone = control.clone();
        let batch_telemetry_clone = batch_telemetry.clone();
        let micro_parallelism = lane_plan.micro_parallelism;
        let total = total_files;
        let micro_jwt_cache = Arc::clone(&jwt_cache);
        let active_client_ptrs_clone = active_client_ptrs.clone();

        tokio::spawn(async move {
            process_swarm(
                "Phase 0 (Micro)",
                app_clone,
                micro_files_clone,
                micro_parallelism,
                active_ports_clone,
                daemon_count,
                is_onion_clone,
                overall_completed_clone,
                overall_failed_clone,
                overall_downloaded_bytes_clone,
                active_batch_circuits_clone,
                control_clone,
                batch_telemetry_clone,
                micro_jwt_cache,
                total,
                active_client_ptrs_clone,
                batch_started_at,
            )
            .await;
        })
    } else {
        tokio::spawn(async move {}) // No-op if empty
    };

    let large_pipeline_handle = if lane_plan.overlap_large_phase && !large_files.is_empty() {
        let app_clone = app.clone();
        let large_files_clone = large_files.clone();
        let control_clone = control.clone();
        let overall_completed_clone = Arc::clone(&overall_completed);
        let overall_failed_clone = Arc::clone(&overall_failed);
        let overall_downloaded_bytes_clone = Arc::clone(&overall_downloaded_bytes);
        let active_batch_circuits_clone = Arc::clone(&active_batch_circuits);
        let batch_telemetry_clone = batch_telemetry.clone();
        let jwt_cache_clone = Arc::clone(&jwt_cache);
        let output_dir_clone = output_dir.clone();
        let large_pipeline_circuits = lane_plan.large_pipeline_circuits;

        tokio::spawn(async move {
            let overlap_ready = wait_for_large_overlap_window(
                &overall_completed_clone,
                &overall_downloaded_bytes_clone,
                &control_clone,
            )
            .await;
            if !overlap_ready {
                let _ = app_clone.emit(
                    "log",
                    "[*] Phase 2 overlap parked; no early useful completions yet. Large files stay in serial fallback.".to_string(),
                );
                return false;
            }
            let _ = app_clone.emit(
                "log",
                "[*] Phase 2 overlap armed after early useful completions.".to_string(),
            );
            process_large_pipeline(
                app_clone,
                large_files_clone
                    .into_iter()
                    .map(|file| file.entry)
                    .collect::<Vec<_>>(),
                large_pipeline_circuits,
                force_tor,
                output_dir_clone,
                control_clone,
                overall_completed_clone,
                overall_failed_clone,
                overall_downloaded_bytes_clone,
                active_batch_circuits_clone,
                batch_telemetry_clone,
                jwt_cache_clone,
                total_files,
                batch_started_at,
            )
            .await;
            true
        })
    } else {
        tokio::spawn(async move { false })
    };

    // -- Phase 1: Download small files concurrently (one file per circuit) --
    if !small_files.is_empty() {
        process_swarm(
            "Phase 1 (Small)",
            app.clone(),
            small_files,
            lane_plan.small_parallelism,
            active_ports.clone(),
            daemon_count,
            is_onion,
            Arc::clone(&overall_completed),
            Arc::clone(&overall_failed),
            Arc::clone(&overall_downloaded_bytes),
            Arc::clone(&active_batch_circuits),
            control.clone(),
            batch_telemetry.clone(),
            Arc::clone(&jwt_cache),
            total_files,
            active_client_ptrs.clone(),
            batch_started_at,
        )
        .await;
    }

    let overlap_processed = large_pipeline_handle.await.unwrap_or(false);

    if ((!lane_plan.overlap_large_phase) || !overlap_processed) && !large_files.is_empty() {
        process_large_pipeline(
            app.clone(),
            large_files
                .into_iter()
                .map(|file| file.entry)
                .collect::<Vec<_>>(),
            batch_circuit_cap,
            force_tor,
            output_dir.clone(),
            control.clone(),
            Arc::clone(&overall_completed),
            Arc::clone(&overall_failed),
            Arc::clone(&overall_downloaded_bytes),
            Arc::clone(&active_batch_circuits),
            batch_telemetry.clone(),
            Arc::clone(&jwt_cache),
            total_files,
            batch_started_at,
        )
        .await;
    }

    // Phase 0: Ensure micro background swarm has finished
    let _ = micro_swarm_handle.await;
    let completed = overall_completed.load(Ordering::Relaxed);
    let failed = overall_failed.load(Ordering::Relaxed);
    if let Some(telemetry) = &batch_telemetry {
        telemetry.set_active_circuits(0);
    }

    publish_batch_progress(
        &app,
        BatchProgressEvent {
            completed,
            failed,
            total: total_files,
            current_file: "Batch complete".to_string(),
            speed_mbps: batch_speed_mbps(
                overall_downloaded_bytes.load(Ordering::Relaxed),
                batch_started_at,
            ),
            downloaded_bytes: overall_downloaded_bytes.load(Ordering::Relaxed),
            active_circuits: Some(0),
        },
    );
    let _ = app.emit(
        "log",
        format!(
            "[✓] Batch complete: {} processed ({} success, {} failed)",
            completed + failed,
            completed,
            failed
        ),
    );
    Ok(())
}

pub async fn start_download(
    app: AppHandle,
    mut entry: BatchFileEntry,
    num_circuits: usize,
    force_tor: bool,
    _output_dir: Option<String>,
    control: DownloadControl,
    jwt_cache: Arc<tokio::sync::RwLock<std::collections::HashMap<String, String>>>,
) -> Result<()> {
    let _download_session_guard =
        telemetry_handle(&app).map(crate::runtime_metrics::DownloadSessionGuard::new);
    let requested_circuits = num_circuits.max(1);
    let is_onion = crate::url_targets_onion(&entry.url) || force_tor;
    let download_telemetry = telemetry_handle(&app);
    let state_file_path = format!("{}.ariaforge_state", entry.path);
    // Phase 135: Download directly to final path — no temp .ariaforge extension.
    // Resume metadata is tracked via .ariaforge_state sidecar. This eliminates
    // orphaned .ariaforge files on cancellation and simplifies the pipeline.

    // Create log file in the output directory
    let output_dir = Path::new(&entry.path)
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| ".".to_string());
    let filename_hint = Path::new(&entry.path)
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_else(|| "download".to_string());
    let logger = DownloadLogger::new(&output_dir, &filename_hint);
    logger.log(&app, "[*] Aria Forge Download Session".to_string());
    logger.log(&app, format!("[*] URL: {}", entry.url));
    logger.log(&app, format!("[*] Output: {}", entry.path));
    logger.log(
        &app,
        format!("[*] Circuits: {} | Tor: {}", requested_circuits, is_onion),
    );
    let governor_profile = crate::resource_governor::detect_profile(Some(Path::new(&entry.path)));
    let _io_policy_guard = crate::io_vanguard::RuntimeDirectIoOverrideGuard::new(Some(
        governor_profile.direct_io_policy,
    ));
    logger.log(
        &app,
        format!(
            "[*] Resource Governor: cpu={} total_gib={} avail_gib={} storage={} arti_cap={} direct_io={}",
            governor_profile.cpu_cores,
            governor_profile.total_memory_bytes / (1024 * 1024 * 1024),
            governor_profile.available_memory_bytes / (1024 * 1024 * 1024),
            crate::resource_governor::storage_class_label(governor_profile.storage_class),
            governor_profile.recommended_arti_cap,
            crate::io_vanguard::direct_io_policy_label()
        ),
    );
    let bootstrap_budget = crate::resource_governor::recommend_download_budget(
        requested_circuits,
        None,
        is_onion,
        Some(Path::new(&entry.path)),
        download_telemetry.as_ref(),
    );
    logger.log(
        &app,
        format!(
            "[*] Download bootstrap budget: circuit_cap={} active_start={} tournament_cap={} pressure={:.2}",
            bootstrap_budget.circuit_cap,
            bootstrap_budget.initial_active_cap,
            bootstrap_budget.tournament_cap,
            bootstrap_budget.pressure.total_pressure
        ),
    );

    // Detect or bootstrap Tor daemons dynamically
    let mut daemon_count = 0usize;
    let mut active_ports: Vec<u16> = Vec::new();
    let mut active_client_ptrs: Vec<crate::tor_native::SharedTorClient> = Vec::new();

    if is_onion {
        let state = app.state::<crate::AppState>();
        let mut download_guard = state.download_swarm_guard.lock().await;

        let needs_bootstrap = match download_guard.as_ref() {
            Some(arc) => {
                arc.lock().await.native_swarm.is_none()
                    || arc.lock().await.get_arti_clients().is_empty()
            }
            None => true,
        };

        if needs_bootstrap {
            logger.log(
                &app,
                "[*] No active TorForge client pool detected. Bootstrapping fresh Aria Forge cluster..."
                    .to_string(),
            );

            match crate::tor::bootstrap_tor_cluster_for_traffic(
                app.clone(),
                bootstrap_budget.circuit_cap,
                128,
                crate::tor::SwarmTrafficClass::OnionService,
            )
            .await
            {
                Ok((new_guard, _ports)) => {
                    *download_guard = Some(std::sync::Arc::new(tokio::sync::Mutex::new(new_guard)));
                    logger.log(
                        &app,
                        "[✓] Aria Forge TorForge client pool ready".to_string(),
                    );
                }
                Err(e) => {
                    return Err(anyhow::anyhow!(
                        "Failed to bootstrap Aria Forge Tor cluster: {}",
                        e
                    ));
                }
            }
        } else {
            logger.log(&app, "[✓] Reusing active TorForge client slots".to_string());
        }

        if let Some(arc) = download_guard.as_ref() {
            active_client_ptrs = arc.lock().await.get_arti_clients();
        }

        daemon_count = active_client_ptrs.len().max(1);
        active_ports = (0..daemon_count).map(|idx| idx as u16).collect();

        let _ = app.emit(
            "tor_status",
            TorStatusEvent {
                state: "ready".to_string(),
                message: format!("Aria Forge: {} TorForge client slots active", daemon_count),
                daemon_count,
            },
        );
    } else {
        let _ = app.emit(
            "tor_status",
            TorStatusEvent {
                state: "clearnet".to_string(),
                message: "Clearnet target detected. Tor bootstrap skipped.".to_string(),
                daemon_count: 0,
            },
        );
    }

    // Safety: ensure active_ports is never empty for downstream indexing
    if active_ports.is_empty() {
        active_ports.push(0); // Dummy slot for clearnet
        daemon_count = 1;
    }

    let _primary_port = active_ports.first().copied().unwrap_or(9051);
    let sniff_client = get_arti_client(is_onion, 0, &active_client_ptrs)?;
    let active_clients_arc = Arc::new(active_client_ptrs.clone());

    // =========================================================
    // Phase 58: JWT Expiration Refresh (Pre-Flight Intercept)
    // =========================================================
    if let Some(exp) = entry.jwt_exp {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        if exp < now + 60 {
            logger.log(&app, format!("[🛡] Downloader intercepted expired JSON Web Token ({} < {}). Invoking adapter refresh layer...", exp, now));

            // Check cross-thread JwtRefreshCache logic
            let mut newly_extracted = None;

            {
                let cache = jwt_cache.read().await;
                // Basic parent-path evaluation
                if let Some(parent) = Path::new(&entry.path)
                    .parent()
                    .map(|p| p.to_string_lossy().to_string())
                {
                    if let Some(fresh_token_map) = cache.get(&parent) {
                        logger.log(
                            &app,
                            "[✨] JWT Cache hit! Fusing retrieved token inline.".to_string(),
                        );

                        // We do a basic string replacement matching DragonForce's &token= standard or identical forms.
                        // Assuming the fresh_token_map contains the Raw URL directly for this sibling
                        let raw_url = fresh_token_map.clone();
                        entry.url = raw_url;
                        newly_extracted = Some(entry.url.clone());
                    }
                }
            }

            // Cache Miss – Adapter API trigger
            if newly_extracted.is_none() {
                let dummy = crate::adapters::FileEntry {
                    jwt_exp: Some(exp),
                    path: entry.path.clone(),
                    size_bytes: entry.size_hint,
                    entry_type: crate::adapters::EntryType::File,
                    raw_url: entry.url.clone(),
                };

                let adapter = crate::adapters::dragonforce::DragonForceAdapter::new();
                let fp = crate::adapters::SiteFingerprint {
                    url: entry.url.clone(),
                    status: 200,
                    headers: http::header::HeaderMap::new(),
                    body: "token=".to_string(), // Forces dragonforce adapter capability evaluation
                };
                if adapter.can_handle(&fp).await {
                    if let Ok(Some(fresh)) = adapter.refresh_jwt(&dummy, &sniff_client).await {
                        entry.url = fresh.raw_url.clone();
                        entry.jwt_exp = fresh.jwt_exp;

                        // Deposit logic into swarm cache
                        if let Some(parent) = Path::new(&entry.path)
                            .parent()
                            .map(|p| p.to_string_lossy().to_string())
                        {
                            let mut w_cache = jwt_cache.write().await;
                            // We store the whole raw_url under the specific file's name inside cache using key `parent|filename`
                            w_cache.insert(format!("{}|{}", parent, entry.path), fresh.raw_url);
                        }
                        logger.log(
                            &app,
                            "[+] Succeeded adapter JWT refresh re-hydration.".to_string(),
                        );
                    }
                }
            }
        }
    }

    let probe = probe_target_with_alternates(&sniff_client, &mut entry, &app).await?;
    let range_mode = probe.supports_ranges;
    let download_budget = crate::resource_governor::recommend_download_budget(
        requested_circuits,
        Some(probe.content_length),
        is_onion,
        Some(Path::new(&entry.path)),
        download_telemetry.as_ref(),
    );
    let host_connection_cap = download_host_connection_cap_for_url(&entry.url, is_onion);
    logger.log(
        &app,
        format!(
            "[*] Download governor: range_mode={} content_length={} circuit_cap={} host_cap={} active_start={} tournament_cap={} pressure={:.2}",
            range_mode,
            probe.content_length,
            download_budget.circuit_cap,
            host_connection_cap,
            download_budget.initial_active_cap,
            download_budget.tournament_cap,
            download_budget.pressure.total_pressure
        ),
    );

    if probe.content_length > 0 {
        if let Ok(meta) = std::fs::metadata(&entry.path) {
            if meta.len() == probe.content_length {
                logger.log(
                    &app,
                    format!(
                        "[✓] Smart Skip: File already completes locally ({} bytes).",
                        probe.content_length
                    ),
                );
                return Ok(());
            }
        }
    }

    let effective_circuits = if range_mode {
        requested_circuits
            .min(download_budget.circuit_cap.max(1))
            .min(host_connection_cap.max(1))
            .min(probe.content_length.max(1) as usize)
            .max(1)
    } else {
        1
    };

    if !range_mode {
        let _ = app.emit(
            "log",
            "[!] Byte-range support unavailable. Falling back to single-stream mode.".to_string(),
        );
    }

    let mut state = DownloadState {
        completed_chunks: vec![false; effective_circuits],
        current_offsets: vec![0; effective_circuits],
        num_circuits: effective_circuits,
        chunk_size: if range_mode {
            probe.content_length / effective_circuits as u64
        } else {
            0
        },
        content_length: if range_mode { probe.content_length } else { 0 },
        piece_mode: false,
        completed_pieces: Vec::new(),
        total_pieces: 0,
        etag: probe.etag.clone(),
        last_modified: probe.last_modified.clone(),
    };
    let piece_size = if range_mode {
        compute_piece_size(state.content_length, effective_circuits)
    } else {
        0
    };
    let total_pieces = if range_mode {
        state.content_length.div_ceil(piece_size) as usize
    } else {
        0
    };
    if range_mode {
        normalize_download_state(&mut state, effective_circuits, piece_size, total_pieces);
    }

    let mut is_resuming = false;
    let mut starting_total_downloaded = 0u64;
    if range_mode && Path::new(&state_file_path).exists() {
        if let Ok(content) = fs::read_to_string(&state_file_path) {
            if let Ok(mut parsed) = serde_json::from_str::<DownloadState>(&content) {
                if parsed.num_circuits == effective_circuits
                    && parsed.content_length == state.content_length
                    && parsed.completed_chunks.len() == effective_circuits
                    && parsed.etag == state.etag
                    && parsed.last_modified == state.last_modified
                {
                    normalize_download_state(
                        &mut parsed,
                        effective_circuits,
                        piece_size,
                        total_pieces,
                    );
                    state = parsed;
                    is_resuming = true;
                    starting_total_downloaded =
                        estimate_downloaded_bytes(&state, effective_circuits);
                    let done = state.completed_chunks.iter().filter(|done| **done).count();
                    let _ = app.emit(
                        "log",
                        format!("[+] Resuming from saved state ({done}/{effective_circuits} chunks complete)."),
                    );
                } else {
                    let _ = app.emit(
                        "log",
                        "[!] Resume validators/state mismatch detected. Discarding stale partial state and restarting clean.".to_string(),
                    );
                    let _ = fs::remove_file(&state_file_path);
                    let _ = fs::remove_file(&entry.path);
                }
            }
        }
    }

    if let Some(parent_dir) = Path::new(&entry.path).parent() {
        fs::create_dir_all(parent_dir)?;
    }

    if range_mode {
        normalize_download_state(&mut state, effective_circuits, piece_size, total_pieces);
    }

    let resume_if_range = preferred_if_range(&state.etag, &state.last_modified);
    if let Some(validator) = &resume_if_range {
        logger.log(&app, format!("[*] Resume validator active: {}", validator));
    }

    let mut file_for_writer: Option<File> = None;
    let mut initial_mmap_for_writer: Option<memmap2::MmapMut> = None;
    let mut shared_mmap: Option<Arc<LockFreeMmap>> = None;

    if range_mode && state.content_length > 0 {
        if let Ok(file) = open_file_with_adaptive_io(
            Path::new(&entry.path),
            !is_resuming, // create/truncate if fresh
            true,
            true,
            true,
            &app,
            Some(&logger),
        ) {
            if !is_resuming {
                let (prealloc_result, mmap_safe) = preallocate_windows_nt_blocks(&file, state.content_length);
                let _ = prealloc_result;
                let _ = app.emit(
                    "log",
                    format!(
                        "[+] Pre-allocated {:.2} GB on disk",
                        state.content_length as f64 / 1_073_741_824.0
                    ),
                );
                // Phase 128: Only create mmap when SetFileValidData succeeded.
                if mmap_safe {
                    if let Ok(m) = unsafe { memmap2::MmapOptions::new().map_mut(&file) } {
                        shared_mmap = Some(Arc::new(LockFreeMmap::new(&m)));
                        initial_mmap_for_writer = Some(m);
                    }
                }
            } else if let Ok(m) = unsafe { memmap2::MmapOptions::new().map_mut(&file) } {
                // Resume path: file already exists with valid data, mmap is safe
                shared_mmap = Some(Arc::new(LockFreeMmap::new(&m)));
                initial_mmap_for_writer = Some(m);
            }
            file_for_writer = Some(file);
        }
    }

    if range_mode {
        fs::write(&state_file_path, serde_json::to_string(&state)?)?;
    } else {
        let _ = fs::remove_file(&state_file_path);
    }

    let ring_buffer = Arc::new(crossbeam_queue::ArrayQueue::<WriteMsg>::new(1_000_000));
    let _ring_capacity: usize = 1_000_000; // Phase 105: Massive RAM queuing for Tor bursts
    let tx = Arc::clone(&ring_buffer);
    let rx = Arc::clone(&ring_buffer);
    let state_for_writer = if range_mode {
        Some((state.clone(), state_file_path.clone()))
    } else {
        None
    };

    let writer_app = app.clone();
    let writer_logger = logger.clone();

    // Phase 105: Offload SHA256 Sync constraints to isolated Tokio Task using Shared Memory
    enum HashPayload {
        Mmap(usize, usize),
        Bytes(Vec<u8>),
    }
    let hash_rx = Arc::new(crossbeam_queue::ArrayQueue::<HashPayload>::new(10_000));
    let hash_tx = hash_rx.clone();

    let hash_mmap_ref = shared_mmap.clone();
    let _hash_task_handle = tokio::task::spawn_blocking(move || {
        #[allow(unused_imports)]
        use sha2::Digest;
        let mut h = sha2::Sha256::new();
        loop {
            match hash_rx.pop() {
                Some(HashPayload::Mmap(start, end)) => {
                    if let Some(mmap) = hash_mmap_ref.as_ref() {
                        unsafe {
                            let slice = std::slice::from_raw_parts(
                                (mmap.ptr as *const u8).add(start),
                                end - start,
                            );
                            h.update(slice);
                        }
                    }
                }
                Some(HashPayload::Bytes(b)) => {
                    if b.is_empty() {
                        break; // EOF
                    }
                    h.update(&b);
                }
                None => {
                    std::thread::yield_now();
                }
            }
        }
        h
    });

    let writer_final_target = entry.path.clone();
    let writer_handle = tokio::task::spawn_blocking(move || -> Result<Option<String>> {
        let mut active_filepath = String::new();
        // Phase 130: Write coalescing — wrap raw File in 256KB BufWriter to reduce
        // NTFS journal commits by 4-8× for sequential piece writes. On non-admin Windows
        // where mmap is disabled, every piece write previously triggered a separate
        // NTFS metadata update. BufWriter coalesces sequential writes into single flushes.
        let mut active_file: Option<std::io::BufWriter<File>> = file_for_writer
            .map(|f| std::io::BufWriter::with_capacity(256 * 1024, f));
        let mut active_mmap: Option<memmap2::MmapMut> = initial_mmap_for_writer;
        if active_file.is_some() {
            active_filepath = writer_final_target;
        }
        let mut local_state = state_for_writer;
        let mut last_flush = Instant::now();
        let mut pieces_since_flush = 0u32; // Throttle state saves
        let mut last_write_end: u64 = u64::MAX; // Phase 4.5: track for write coalescing
        let mut idle_polls = 0u32;

        let mut hash_byte_offset: u64 = 0;
        let mut last_msync = Instant::now();
        #[allow(unused_imports)]
        use sha2::Digest;
        let hasher = sha2::Sha256::new();
        let mut disk_intervals = IntervalTracker::new();

        if let Some((st, _)) = &local_state {
            if st.piece_mode && !st.completed_pieces.is_empty() && st.chunk_size > 0 {
                for (i, &done) in st.completed_pieces.iter().enumerate() {
                    if done {
                        let p_start = i as u64 * st.chunk_size;
                        let p_end = (p_start + st.chunk_size).min(st.content_length);
                        disk_intervals.add(p_start, p_end);
                    }
                }
            } else if !st.piece_mode && !st.completed_chunks.is_empty() && st.chunk_size > 0 {
                for (i, &done) in st.completed_chunks.iter().enumerate() {
                    if done {
                        let c_start = i as u64 * st.chunk_size;
                        let c_end = (c_start + st.chunk_size).min(st.content_length);
                        disk_intervals.add(c_start, c_end);
                    }
                }
            }
        }

        loop {
            let msg = match rx.pop() {
                Some(m) => {
                    idle_polls = 0;
                    m
                }
                None => {
                    idle_polls = idle_polls.saturating_add(1);
                    if idle_polls <= 32 {
                        std::hint::spin_loop();
                    } else {
                        // Periodic MS_ASYNC OS page flush for massive Memory-Mapped buffers
                        if last_msync.elapsed() >= std::time::Duration::from_secs(5) {
                            if let Some(mmap) = active_mmap.as_mut() {
                                let _ = mmap.flush_async();
                            }
                            last_msync = Instant::now();
                        }
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    continue;
                }
            };
            if msg.chunk_id == usize::MAX && msg.close_file {
                if let Some(mmap) = active_mmap.as_mut() {
                    let _ = mmap.flush();
                }
                break; // EOF signal stops lock-free background writer
            }
            let mut should_flush = false;

            if !msg.data.is_empty() || msg.mmap_written {
                if active_filepath != msg.filepath || active_file.is_none() {
                    if let Some(mmap) = active_mmap.as_mut() {
                        let _ = mmap.flush();
                    }
                    // Phase 130: Flush BufWriter before switching files
                    if let Some(ref mut bw) = active_file {
                        let _ = bw.flush();
                    }
                    active_mmap = None;

                    if let Some(dir) = Path::new(&msg.filepath).parent() {
                        fs::create_dir_all(dir)?;
                    }
                    if active_filepath != msg.filepath || active_file.is_none() {
                        let fout = open_file_with_adaptive_io(
                            Path::new(&msg.filepath),
                            true,
                            true,
                            true,
                            false,
                            &writer_app,
                            Some(&writer_logger),
                        )?;
                        // Phase 130: Wrap in 256KB BufWriter for write coalescing
                        active_file = Some(std::io::BufWriter::with_capacity(256 * 1024, fout));
                    }

                    if let Some((st, _)) = &local_state {
                        if st.content_length > 0 && active_mmap.is_none() {
                            if let Some(bw) = active_file.as_ref() {
                                // Phase 130: Use .get_ref() to access inner File for prealloc/mmap
                                let f = bw.get_ref();
                                let (prealloc_result, mmap_safe) = preallocate_windows_nt_blocks(f, st.content_length);
                                let _ = prealloc_result;
                                // Phase 128: Only create mmap when SetFileValidData succeeded.
                                // Without it, valid data length < file size, and mmap writes
                                // beyond the valid region hit uncommitted NT pages → ACCESS_VIOLATION.
                                if mmap_safe {
                                    if let Ok(m) = unsafe { memmap2::MmapOptions::new().map_mut(f) } {
                                        active_mmap = Some(m);
                                    }
                                }
                            }
                        }
                    }

                    active_filepath = msg.filepath.clone();
                    last_write_end = u64::MAX; // Reset on new file
                }

                if !msg.mmap_written {
                    if let Some(mmap) = active_mmap.as_mut() {
                        let start = msg.offset as usize;
                        let end = start + msg.data.len();
                        if end <= mmap.len() {
                            // Phase 7: Zero-Copy ram write!
                            mmap[start..end].copy_from_slice(&msg.data);
                        } else if let Some(file) = active_file.as_mut() {
                            if msg.offset != last_write_end {
                                file.seek(SeekFrom::Start(msg.offset))?;
                            }
                            file.write_all(&msg.data)?;
                            last_write_end = msg.offset + msg.data.len() as u64;
                        }
                    } else if let Some(file) = active_file.as_mut() {
                        if msg.offset != last_write_end {
                            use std::io::{Seek, SeekFrom};
                            file.seek(SeekFrom::Start(msg.offset))?;
                        }
                        use std::io::Write;
                        file.write_all(&msg.data)?;
                        last_write_end = msg.offset + msg.data.len() as u64;
                    }
                    disk_intervals.add(msg.offset, msg.offset + msg.data.len() as u64);
                } else {
                    disk_intervals.add(msg.offset, msg.piece_end as u64);
                }

                let new_contiguous = disk_intervals.contiguous_up_to();
                if new_contiguous > hash_byte_offset {
                    let to_hash_len = (new_contiguous - hash_byte_offset) as usize;
                    if active_mmap.is_some() {
                        let start_idx = hash_byte_offset as usize;
                        let end_idx = start_idx + to_hash_len;
                        let mut payload = HashPayload::Mmap(start_idx, end_idx);
                        while let Err(ret) = hash_tx.push(payload) {
                            payload = ret;
                            std::thread::yield_now();
                        }
                        hash_byte_offset = new_contiguous;
                    } else if active_file.is_some() {
                        if let Ok(mut r) = File::open(&active_filepath) {
                            use std::io::{Read, Seek, SeekFrom};
                            if r.seek(SeekFrom::Start(hash_byte_offset)).is_ok() {
                                let mut buffer = vec![0u8; 1_048_576];
                                let mut bytes_to_read = to_hash_len;
                                while bytes_to_read > 0 {
                                    let chunk_size = bytes_to_read.min(buffer.len());
                                    if let Ok(n) = r.read(&mut buffer[..chunk_size]) {
                                        if n == 0 {
                                            break;
                                        }
                                        let mut payload = HashPayload::Bytes(buffer[..n].to_vec());
                                        while let Err(ret) = hash_tx.push(payload) {
                                            payload = ret;
                                            std::thread::yield_now();
                                        }
                                        bytes_to_read -= n;
                                        hash_byte_offset += n as u64;
                                    } else {
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }

                if let Some((state, _)) = local_state.as_mut() {
                    if msg.chunk_id < state.current_offsets.len() {
                        let chunk_start = msg.chunk_id as u64 * state.chunk_size;
                        let written_global = msg.offset + msg.data.len() as u64;
                        let chunk_offset = written_global.saturating_sub(chunk_start);
                        let piece_len = piece_len_for_index(
                            state.content_length,
                            state.chunk_size,
                            msg.chunk_id,
                        );

                        if chunk_offset > state.current_offsets[msg.chunk_id] {
                            state.current_offsets[msg.chunk_id] = if piece_len > 0 {
                                chunk_offset.min(piece_len)
                            } else {
                                chunk_offset
                            };
                        }
                    }
                }
            }

            if last_flush.elapsed() >= Duration::from_secs(5) {
                should_flush = true;
                last_flush = Instant::now();
                if let Some(mmap) = active_mmap.as_mut() {
                    let _ = mmap.flush_async();
                }
            }

            if msg.close_file {
                if let Some((state, _)) = local_state.as_mut() {
                    if msg.chunk_id < state.completed_chunks.len() {
                        state.completed_chunks[msg.chunk_id] = true;
                    }
                    if state.piece_mode
                        && msg.chunk_id < state.completed_pieces.len()
                        && msg.chunk_id <= msg.piece_end
                    {
                        let capped_end = msg.piece_end.min(state.completed_pieces.len() - 1);
                        for piece_idx in msg.chunk_id..=capped_end {
                            state.completed_pieces[piece_idx] = true;
                            if piece_idx < state.current_offsets.len() {
                                state.current_offsets[piece_idx] = piece_len_for_index(
                                    state.content_length,
                                    state.chunk_size,
                                    piece_idx,
                                );
                            }
                        }
                    }
                    pieces_since_flush += 1;
                    // Only flush to disk every 10 pieces (or on time interval)
                    if pieces_since_flush >= 10 {
                        should_flush = true;
                        pieces_since_flush = 0;
                    }
                }
                active_filepath.clear();
                active_file = None;
            }

            if should_flush {
                if let Some((state, path)) = local_state.as_mut() {
                    // Phase 49: Atomic WAL write (crash-safe)
                    let tmp_path = format!("{}.tmp", path);
                    let data = serde_json::to_string(state).unwrap_or_default();
                    if fs::write(&tmp_path, &data).is_ok() {
                        let _ = fs::rename(&tmp_path, path.as_str());
                    }
                }
            }
        }

        if let Some((state, path)) = local_state.as_mut() {
            // Phase 49: Atomic WAL write (final flush, crash-safe)
            let tmp_path = format!("{}.tmp", path);
            let data = serde_json::to_string(state).unwrap_or_default();
            if fs::write(&tmp_path, &data).is_ok() {
                let _ = fs::rename(&tmp_path, path.as_str());
            }
        }

        let final_hash = if let Some((st, _)) = &local_state {
            if hash_byte_offset >= st.content_length && st.content_length > 0 {
                Some(hex::encode(hasher.finalize()))
            } else {
                None
            }
        } else {
            None
        };

        Ok(final_hash)
    });

    let total_downloaded = Arc::new(AtomicU64::new(starting_total_downloaded));
    let run_flag = Arc::new(AtomicBool::new(true));
    let start_time = Instant::now();

    let watcher_total = Arc::clone(&total_downloaded);
    let active_circuits = Arc::new(AtomicUsize::new(0));
    let watcher_active = Arc::clone(&active_circuits);
    let watcher_running = Arc::clone(&run_flag);
    let watcher_app = app.clone();
    let watcher_telemetry = download_telemetry.clone();
    let watcher_content_length = probe.content_length;
    let watcher_path = entry.path.clone();
    let speed_handle = tokio::spawn(async move {
        while watcher_running.load(Ordering::Relaxed) {
            tokio::time::sleep(Duration::from_millis(500)).await;
            let downloaded = watcher_total.load(Ordering::Relaxed);
            let elapsed = start_time.elapsed().as_secs_f64();
            let bytes_per_sec = if elapsed > 0.0 {
                downloaded as f64 / elapsed
            } else {
                0.0
            };
            publish_download_progress(
                &watcher_app,
                crate::telemetry_bridge::BridgeDownloadProgress {
                    path: watcher_path.clone(),
                    bytes_downloaded: downloaded,
                    total_bytes: if watcher_content_length > 0 {
                        Some(watcher_content_length)
                    } else {
                        None
                    },
                    speed_bps: bytes_per_sec as u64,
                    active_circuits: watcher_active.load(Ordering::Relaxed),
                },
            );
            if let Some(telemetry) = &watcher_telemetry {
                telemetry.set_active_circuits(watcher_active.load(Ordering::Relaxed));
            }
        }
    });

    let mut tasks = JoinSet::new();

    if range_mode {
        let content_length = state.content_length;

        // Phase 4.1: Adaptive piece sizing
        let piece_size = compute_piece_size(content_length, effective_circuits);
        let total_pieces = content_length.div_ceil(piece_size) as usize;

        logger.log(
            &app,
            format!(
                "[*] Phase 4.1: Adaptive piece size: {:.1} MB ({} pieces)",
                piece_size as f64 / 1_048_576.0,
                total_pieces
            ),
        );

        // Build or restore the piece completion tracker
        let piece_completed: Vec<bool> =
            if state.piece_mode && state.completed_pieces.len() == total_pieces {
                state.completed_pieces.clone()
            } else {
                vec![false; total_pieces]
            };

        // Save piece_mode state
        state.piece_mode = true;
        state.total_pieces = total_pieces;
        state.completed_pieces = piece_completed.clone();
        fs::write(&state_file_path, serde_json::to_string(&state)?)?;

        let pieces_done_count = piece_completed.iter().filter(|&&done| done).count();
        if pieces_done_count > 0 {
            let _ = app.emit(
                "log",
                format!(
                    "[+] Resuming: {}/{} pieces already complete.",
                    pieces_done_count, total_pieces
                ),
            );
        }

        let coalesce_limit = if pieces_done_count > 0 {
            resume_coalesce_piece_limit()
        } else {
            1
        };
        let piece_spans =
            build_piece_spans(&piece_completed, piece_size, content_length, coalesce_limit);
        let total_spans = piece_spans.len();
        if coalesce_limit > 1 && total_spans > 0 {
            logger.log(
                &app,
                format!(
                    "[*] Resume span coalescing active: {} remaining pieces packed into {} spans (limit={} pieces/span).",
                    total_pieces.saturating_sub(pieces_done_count),
                    total_spans,
                    coalesce_limit
                ),
            );
        }

        // ===== CONTINUOUS AUTO-SCALING =====
        // Formula: circuits = clamp(total_pieces, 1, max_circuits)
        // This naturally handles everything:
        //   1KB file  → 1 piece  → 1 circuit
        //   10MB file → 1 piece  → 1 circuit
        //   50MB file → 5 pieces → 5 circuits
        //   500MB file → 50 pieces → 50 circuits
        //   5GB file  → 500 pieces → 120 circuits (capped)
        let scaled_circuits = total_spans.clamp(1, effective_circuits);

        // Tournament: candidate pool is adaptive and capped to avoid handshake storms.
        // The production path now uses tor.rs telemetry to decide how much over-subscription
        // is worthwhile instead of always racing a fixed 2x pool.
        let tournament_pool = if total_spans >= scaled_circuits * 3 {
            download_tournament_candidate_count(scaled_circuits, is_onion)
                .min(download_budget.tournament_cap.max(scaled_circuits))
        } else {
            scaled_circuits // Not enough work — skip tournament, direct assign
        };
        let max_promoted = scaled_circuits;
        let skip_tournament = tournament_pool <= scaled_circuits;
        let initial_active_budget =
            download_initial_active_budget(scaled_circuits, total_spans, is_onion)
                .min(download_budget.initial_active_cap.max(1))
                .clamp(1, scaled_circuits);

        // Label the tier for UI logging
        let tier = if total_spans <= 1 {
            "tiny"
        } else if scaled_circuits <= 10 {
            "small"
        } else if scaled_circuits <= 50 {
            "medium"
        } else {
            "large"
        };

        // Shared state: atomic next-piece index and piece completion array
        let work_spans = Arc::new(piece_spans);
        let next_span = Arc::new(AtomicUsize::new(0));
        let piece_flags: Arc<Vec<AtomicBool>> = Arc::new(
            piece_completed
                .iter()
                .map(|&done| AtomicBool::new(done))
                .collect(),
        );
        // Track which circuit owns each in-progress piece (for kill-after-steal)
        let piece_owner: Arc<Vec<AtomicUsize>> = Arc::new(
            (0..total_pieces)
                .map(|_| AtomicUsize::new(usize::MAX))
                .collect(),
        );
        // Kill flags: when a circuit gets stolen from, it's marked for death
        let circuit_killed: Arc<Vec<AtomicBool>> = Arc::new(
            (0..tournament_pool)
                .map(|_| AtomicBool::new(false))
                .collect(),
        );
        // Per-circuit byte counters for health monitoring
        let circuit_bytes: Arc<Vec<AtomicU64>> =
            Arc::new((0..tournament_pool).map(|_| AtomicU64::new(0)).collect());
        // Global server health: rises when many circuits fail, triggers coordinated pause
        let server_fail_count = Arc::new(AtomicUsize::new(0));

        // Phase 3: UCB1 Multi-Armed Bandit scorer
        let circuit_scorer = Arc::new(CircuitScorer::new(tournament_pool));

        // Phase 4.4: AIMD/BBR active-window controller.
        // Start below the full promoted set and let measured throughput expand the window.
        let aimd = Arc::new(BbrController::new(initial_active_budget, scaled_circuits));

        let _ = app.emit(
            "log",
            if skip_tournament {
                format!(
                    "[+] {} file ({} pieces) → {} circuits (no tournament) | Active window start: {}/{}",
                    tier, total_spans, scaled_circuits, initial_active_budget, scaled_circuits
                )
            } else {
                format!(
                    "[+] {} file ({} spans / {} pieces) → {} circuits | Tournament: {} racing for {} slots | Active window start: {}/{}",
                    tier,
                    total_spans,
                    total_pieces,
                    scaled_circuits,
                    tournament_pool,
                    max_promoted,
                    initial_active_budget,
                    scaled_circuits
                )
            },
        );

        // Phase 129: Log mirror striping status
        if !entry.alternate_urls.is_empty() {
            let mirror_count = entry.alternate_urls.len().min(3) + 1;
            let _ = app.emit(
                "log",
                format!(
                    "[+] Phase 129: Mirror Striping ACTIVE — {} circuits across {} mirrors (primary + {} alternates)",
                    scaled_circuits, mirror_count, mirror_count - 1
                ),
            );
        }

        // ===== TOURNAMENT-STYLE CIRCUIT RACING =====
        let promoted_count = Arc::new(AtomicUsize::new(0));

        // ===== PROACTIVE HEALTH MONITOR =====
        // Runs every 15s: computes per-circuit speed, kills circuits below 20% of median
        {
            let mon_killed = Arc::clone(&circuit_killed);
            let mon_bytes = Arc::clone(&circuit_bytes);
            let mon_running = Arc::clone(&run_flag);
            let mon_app = app.clone();
            let mon_pool_size = tournament_pool;

            tokio::spawn(async move {
                // Wait for circuits to warm up before monitoring
                tokio::time::sleep(Duration::from_secs(45)).await;

                let mut prev_bytes = vec![0u64; mon_pool_size];

                while mon_running.load(Ordering::Relaxed) {
                    tokio::time::sleep(Duration::from_secs(HEALTH_CHECK_INTERVAL_SECS)).await;
                    if !mon_running.load(Ordering::Relaxed) {
                        break;
                    }

                    // Compute per-circuit speed since last check
                    let mut speeds: Vec<(usize, f64)> = Vec::new();
                    for cid in 0..mon_pool_size {
                        if mon_killed[cid].load(Ordering::Relaxed) {
                            continue;
                        }
                        let current = mon_bytes[cid].load(Ordering::Relaxed);
                        let delta = current.saturating_sub(prev_bytes[cid]);
                        prev_bytes[cid] = current;
                        if current > 0 {
                            // Only track circuits that have downloaded something
                            let speed = delta as f64 / HEALTH_CHECK_INTERVAL_SECS as f64;
                            speeds.push((cid, speed));
                        }
                    }

                    if speeds.len() < 3 {
                        continue;
                    } // Need enough data points

                    // Compute median speed
                    let mut sorted_speeds: Vec<f64> = speeds.iter().map(|(_, s)| *s).collect();
                    sorted_speeds
                        .sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                    let median = sorted_speeds[sorted_speeds.len() / 2];
                    let threshold = median * MIN_SPEED_RATIO;

                    // Kill circuits below threshold
                    for (cid, speed) in &speeds {
                        if *speed < threshold && *speed < 50_000.0 {
                            // Below 20% of median AND below ~50KB/s absolute
                            mon_killed[*cid].store(true, Ordering::Relaxed);
                            let _ = mon_app.emit("log", format!(
                                "[!] Health monitor: killed circuit {} ({:.0} B/s vs {:.0} B/s median)",
                                cid, speed, median
                            ));
                        }
                    }
                }
            });
        }

        // ===== PHASE 3.2: OPTIMISTIC UNCHOKE =====
        // Every 30s, test a fresh circuit. If faster than the slowest active → swap them.
        // This discovers improved network conditions during long downloads.
        if total_pieces >= 20 {
            let unchoke_killed = Arc::clone(&circuit_killed);
            let unchoke_scorer = Arc::clone(&circuit_scorer);
            let unchoke_running = Arc::clone(&run_flag);
            let unchoke_app = app.clone();
            let unchoke_url = entry.url.clone();
            let unchoke_is_onion = is_onion;
            let unchoke_content_length = content_length;
            let unchoke_active_ports = active_ports.clone();
            let unchoke_if_range = resume_if_range.clone();
            let unchoke_active_client_ptrs = Arc::clone(&active_clients_arc);

            tokio::spawn(async move {
                // Wait for initial circuits to warm up
                tokio::time::sleep(Duration::from_secs(60)).await;
                let mut unchoke_id = 9000usize; // High IDs to avoid collision

                while unchoke_running.load(Ordering::Relaxed) {
                    tokio::time::sleep(Duration::from_secs(UNCHOKE_INTERVAL_SECS)).await;
                    if !unchoke_running.load(Ordering::Relaxed) {
                        break;
                    }

                    unchoke_id += 1;
                    let _port = if unchoke_is_onion {
                        unchoke_active_ports[0] as usize // Use first daemon from active_ports
                    } else {
                        9051 // Fallback for non-onion, though unchoke_is_onion should handle this
                    };

                    let client = match get_arti_client(
                        unchoke_is_onion,
                        unchoke_id,
                        &unchoke_active_client_ptrs,
                    ) {
                        Ok(c) => c,
                        Err(_) => continue,
                    };

                    // Download a 100KB probe to measure speed
                    let probe_start = 0u64;
                    let probe_end = (PROBE_SIZE - 1).min(unchoke_content_length.saturating_sub(1));
                    let probe_timer = Instant::now();

                    let probe_ok = match tokio::time::timeout(Duration::from_secs(15), {
                        let mut req = apply_download_connection_policy(
                            client
                                .get(&unchoke_url)
                                .header("Range", &format!("bytes={probe_start}-{probe_end}")),
                            &unchoke_url,
                            unchoke_is_onion,
                            None,
                        );
                        if let Some(if_range) = &unchoke_if_range {
                            req = req.header("If-Range", if_range);
                        }
                        req.send()
                    })
                    .await
                    {
                        Ok(Ok(resp))
                            if resp.status() == StatusCode::PARTIAL_CONTENT
                                || resp.status() == StatusCode::OK =>
                        {
                            use futures::StreamExt;
                            let mut stream = resp.bytes_stream();
                            let mut bytes = 0u64;
                            while let Ok(Some(Ok(chunk))) =
                                tokio::time::timeout(Duration::from_secs(7), stream.next()).await
                            {
                                bytes += chunk.len() as u64;
                                if bytes >= PROBE_SIZE {
                                    break;
                                }
                            }
                            bytes > 0
                        }
                        _ => false,
                    };

                    if !probe_ok {
                        continue;
                    }

                    let probe_ms = probe_timer.elapsed().as_millis() as u64;
                    let unchoke_speed = PROBE_SIZE as f64 / probe_ms.max(1) as f64; // bytes/ms

                    // Compare to slowest active circuit
                    if let Some(slowest_cid) = unchoke_scorer.slowest_circuit() {
                        let slowest_speed = {
                            let total_b = unchoke_scorer.total_bytes[slowest_cid]
                                .load(Ordering::Relaxed)
                                as f64;
                            let total_ms = unchoke_scorer.total_elapsed_ms[slowest_cid]
                                .load(Ordering::Relaxed)
                                .max(1) as f64;
                            total_b / total_ms
                        };

                        if unchoke_speed > slowest_speed * 1.3 {
                            // 30% faster threshold
                            // Kill the slowest circuit → it will recycle with fresh identity
                            if slowest_cid < unchoke_killed.len() {
                                unchoke_killed[slowest_cid].store(true, Ordering::Relaxed);
                                let _ = unchoke_app.emit("log", format!(
                                    "[↻] Unchoke: fresh circuit {:.1} KB/s > circuit {} at {:.1} KB/s → recycled",
                                    unchoke_speed * 1000.0 / 1024.0,
                                    slowest_cid,
                                    slowest_speed * 1000.0 / 1024.0
                                ));
                            }
                        }
                    }
                }
            });
        }

        // ===== PHASE 1.1: SOCKS5 HANDSHAKE PRE-FILTER =====
        // Time the SOCKS5 handshake for all circuits in parallel.
        // The bottom 50% by latency are culled before any data is downloaded.
        // This costs 0 bytes and takes ~200ms total (all parallel).
        let mut circuit_candidates: Vec<(usize, crate::arti_client::ArtiClient, usize)> =
            Vec::new();
        {
            logger.log(
                &app,
                format!(
                    "[*] Phase 1: Handshake pre-filter — racing {} circuits...",
                    tournament_pool
                ),
            );

            let mut handshake_tasks = JoinSet::new();
            let circuit_active_client_ptrs = Arc::clone(&active_clients_arc);
            for cid in 0..tournament_pool {
                let probe_url = entry.url.clone();
                let c = cid;
                let is_onion_clone = is_onion;
                let active_ports_clone = active_ports.clone();
                let circuit_active_client_ptrs_clone = Arc::clone(&circuit_active_client_ptrs);
                handshake_tasks.spawn(async move {
                    let port = active_ports_clone[c % daemon_count.max(1)] as usize;
                    let start = Instant::now();
                    let client =
                        match get_arti_client(is_onion_clone, c, &circuit_active_client_ptrs_clone)
                        {
                            Ok(c) => c,
                            Err(_) => return (c, port, None, u128::MAX),
                        };
                    let _host_permit =
                        acquire_download_host_permit(&probe_url, is_onion_clone, None).await;
                    // HEAD request to force the SOCKS5 handshake through Tor
                    let latency = match tokio::time::timeout(
                        Duration::from_secs(15),
                        client.head(&probe_url).send(),
                    )
                    .await
                    {
                        Ok(Ok(_)) => start.elapsed().as_millis(),
                        _ => u128::MAX, // Failed — assign worst latency
                    };
                    (cid, port, Some(client), latency)
                });
            }

            // Collect all results
            let mut results: Vec<(usize, usize, Option<crate::arti_client::ArtiClient>, u128)> =
                Vec::new();
            // Phase 126C: Hard outer deadline — Arti cancellation-safety fix
            let handshake_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
            loop {
                match tokio::time::timeout_at(handshake_deadline, handshake_tasks.join_next()).await {
                    Ok(Some(Ok(result))) => results.push(result),
                    Ok(Some(_)) => continue,
                    Ok(None) => break,
                    Err(_) => {
                        eprintln!("[ARIA] Handshake probe deadline exceeded (30s) — aborting remaining");
                        handshake_tasks.abort_all();
                        break;
                    }
                }
            }

            let ready_durations_ms: Vec<u64> = results
                .iter()
                .filter_map(|(_, _, _, latency)| {
                    if *latency < u128::MAX {
                        Some((*latency).min(u64::MAX as u128) as u64)
                    } else {
                        None
                    }
                })
                .collect();

            // Sort by latency (fastest first)
            results.sort_by_key(|r| r.3);

            // Keep top circuits (cull bottom HANDSHAKE_CULL_RATIO)
            let keep_count = handshake_keep_count(results.len(), scaled_circuits, is_onion);
            let cutoff_latency = results.get(keep_count).map(|r| r.3).unwrap_or(u128::MAX);

            for (i, (cid, port, client_opt, _latency)) in results.into_iter().enumerate() {
                if i < keep_count {
                    if let Some(client) = client_opt {
                        circuit_candidates.push((cid, client, port));
                    }
                }
            }

            crate::tor::update_tournament_telemetry(
                &ready_durations_ms,
                circuit_candidates.len(),
                ready_durations_ms.len(),
            );

            logger.log(
                &app,
                format!(
                    "[+] Handshake filter: {} survived / {} culled (cutoff: {}ms)",
                    circuit_candidates.len(),
                    tournament_pool - circuit_candidates.len(),
                    if cutoff_latency < u128::MAX {
                        cutoff_latency.to_string()
                    } else {
                        "∞".to_string()
                    }
                ),
            );
        }

        for (circuit_rank, (circuit_id, circuit_client, daemon_port)) in
            circuit_candidates.into_iter().enumerate()
        {
            let task_tx = tx.clone();
            let task_app = app.clone();
            // Phase 129: IDM-style Mirror Striping — assign different mirror URLs
            // to different circuits so segments download from independent mirrors
            // simultaneously, stacking bandwidth across multiple Tor relay paths.
            let task_url = if !entry.alternate_urls.is_empty() {
                let mirror_pool_size = entry.alternate_urls.len().min(3) + 1; // primary + up to 3 mirrors
                let mirror_idx = circuit_rank % mirror_pool_size;
                if mirror_idx == 0 {
                    entry.url.clone()
                } else {
                    entry.alternate_urls[mirror_idx - 1].clone()
                }
            } else {
                entry.url.clone()
            };
            let task_path = entry.path.clone();
            let task_control = control.clone();
            let task_running = Arc::clone(&run_flag);
            let task_total = Arc::clone(&total_downloaded);
            let task_next_span = Arc::clone(&next_span);
            let task_work_spans = Arc::clone(&work_spans);
            let task_piece_flags = Arc::clone(&piece_flags);
            let task_piece_owner = Arc::clone(&piece_owner);
            let task_circuit_killed = Arc::clone(&circuit_killed);
            let task_circuit_bytes = Arc::clone(&circuit_bytes);
            let task_server_fails = Arc::clone(&server_fail_count);
            let task_total_pieces = total_pieces;
            let task_total_spans = total_spans;
            let task_content_length = content_length;
            let task_promoted = Arc::clone(&promoted_count);
            let task_max_promoted = max_promoted;
            let task_skip_tournament = skip_tournament;
            let task_is_onion = is_onion;
            let task_daemon_port = daemon_port;
            let task_tournament_pool = tournament_pool;

            // Phase 3: UCB1 scorer for adaptive piece assignment
            let task_scorer = Arc::clone(&circuit_scorer);
            let task_piece_size = piece_size; // Phase 4.1
            let task_aimd = Arc::clone(&aimd); // Phase 4.4
            let task_active_circuits = Arc::clone(&active_circuits);
            let task_circuit_rank = circuit_rank;
            let task_if_range = resume_if_range.clone();
            let task_active_client_ptrs = Arc::clone(&active_clients_arc);
            let task_download_telemetry = download_telemetry.clone();
            let task_shared_mmap = shared_mmap.clone();

            tasks.spawn(async move {
                use futures::StreamExt;
                let mut circuit_client = circuit_client; // Mutable for recycling
                let mut ddos_guard = crate::adapters::qilin_ddos_guard::DdosGuard::new();
                let task_low_speed_policy = download_low_speed_policy(task_is_onion);

                // === TOURNAMENT PROBE PHASE ===
                if !task_skip_tournament {
                    // Phase 1.2: 100KB micro-probe (instead of 1MB)
                    // TCP slow-start stabilizes at ~50KB through Tor, so 100KB
                    // captures 80% of the throughput signal in 10% of the time.
                    let probe_start = (circuit_id as u64 % task_total_pieces as u64) * task_piece_size;
                    let probe_end = (probe_start + PROBE_SIZE - 1).min(task_content_length.saturating_sub(1));

                    let probe_result = async {
                        let Some(_host_permit) =
                            acquire_download_host_permit(&task_url, task_is_onion, Some(&task_control)).await
                        else {
                            return false;
                        };
                        let resp = tokio::time::timeout(Duration::from_secs(30), {
                            let mut req = apply_download_connection_policy(
                                circuit_client
                                    .get(&task_url)
                                    .header("Range", &format!("bytes={probe_start}-{probe_end}")),
                                &task_url,
                                task_is_onion,
                                task_download_telemetry.as_ref(),
                            );
                            if let Some(if_range) = &task_if_range {
                                req = req.header("If-Range", if_range);
                            }
                            req.send()
                        })
                        .await;

                        match resp {
                            Ok(Ok(r)) if r.status() == StatusCode::PARTIAL_CONTENT => {
                                let mut stream = r.bytes_stream();
                                let mut bytes = 0u64;
                                loop {
                                    match tokio::time::timeout(Duration::from_secs(STREAM_TIMEOUT_SECS), stream.next()).await {
                                        Ok(Some(Ok(chunk))) => {
                                            bytes += chunk.len() as u64;
                                            if bytes >= PROBE_SIZE { return true; }
                                        }
                                        _ => return bytes > 0,
                                    }
                                }
                            }
                            _ => false,
                        }
                    }.await;

                    if !probe_result {
                        return TaskOutcome::Completed; // Failed probe — exit silently
                    }

                    // Try to claim a promotion slot
                    let my_slot = task_promoted.fetch_add(1, Ordering::Relaxed);
                    if my_slot >= task_max_promoted {
                        return TaskOutcome::Completed; // All slots taken — exit
                    }
                } else {
                    // Small file: no tournament, auto-promote all circuits
                    let my_slot = task_promoted.fetch_add(1, Ordering::Relaxed);
                    if my_slot >= task_max_promoted {
                        return TaskOutcome::Completed; // More circuits than pieces — exit
                    }
                }

                // === WORK QUEUE PHASE (promoted circuits only) ===
                let _active_guard = ActiveCircuitGuard::new(task_active_circuits); // Track active downloading connection
                let mut pieces_completed = 0usize;
                let mut stalls = 0usize;
                let mut stealing = false;
                let mut recycle_count = 0usize;

                loop {
                    if !task_running.load(Ordering::Relaxed) {
                        break;
                    }
                    let active_window = task_aimd.current_active().max(1);
                    if task_circuit_rank >= active_window {
                        tokio::time::sleep(Duration::from_millis(125)).await;
                        continue;
                    }
                    // Check if this circuit was killed → RECYCLE with fresh identity
                    if circuit_id < task_circuit_killed.len() && task_circuit_killed[circuit_id].load(Ordering::Relaxed) {
                        recycle_count += 1;
                        let new_socks_id = circuit_id + recycle_count * task_tournament_pool;
                        match get_arti_client(task_is_onion, new_socks_id, &task_active_client_ptrs) {
                            Ok(new_client) => {
                                circuit_client = new_client;
                                task_circuit_killed[circuit_id].store(false, Ordering::Relaxed);
                                task_circuit_bytes[circuit_id].store(0, Ordering::Relaxed);
                                stalls = 0;
                                let _ = task_app.emit("log", format!(
                                    "[♻] Circuit {} recycled → fresh identity (recycle #{})", circuit_id, recycle_count
                                ));
                                continue;
                            }
                            Err(_) => {
                                let _ = task_app.emit("log", format!("[x] Circuit {} killed (recycle failed)", circuit_id));
                                return TaskOutcome::Completed;
                            }
                        }
                    }
                    if let Some(reason) = task_control.interruption_reason() {
                        task_running.store(false, Ordering::Relaxed);
                        return TaskOutcome::Interrupted(reason);
                    }

                    // Phase 3: UCB1 adaptive yield — slow circuits wait, fast circuits proceed immediately
                    if pieces_completed > 0 {
                        let delay = task_scorer.yield_delay(circuit_id);
                        if !delay.is_zero() {
                            tokio::time::sleep(delay).await;
                        }
                    }

                    // Grab next work span — normal mode uses pre-built spans, steal mode falls back to single pieces.
                    let mut stolen_from = usize::MAX;
                    let piece_span = if !stealing {
                        let idx = task_next_span.fetch_add(1, Ordering::Relaxed);
                        if idx >= task_total_spans {
                            stealing = true;

                            // Phase 2.2: HEDGED REQUESTS for last 10%
                            let done_count = task_piece_flags
                                .iter()
                                .filter(|f| f.load(Ordering::Relaxed))
                                .count();
                            let remaining_pct =
                                100.0 * (1.0 - done_count as f64 / task_total_pieces as f64);

                            if remaining_pct > 10.0 {
                                let delay_ms = (circuit_id % 20) as u64 * 100;
                                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                            }
                            continue;
                        }
                        let span = task_work_spans[idx].clone();
                        if (span.start_piece..=span.end_piece)
                            .all(|piece_idx| task_piece_flags[piece_idx].load(Ordering::Relaxed))
                        {
                            continue;
                        }
                        span
                    } else {
                        let scan_start = circuit_id % task_total_pieces;
                        let found = (0..task_total_pieces)
                            .map(|i| (scan_start + i) % task_total_pieces)
                            .find(|&i| !task_piece_flags[i].load(Ordering::Relaxed));
                        let piece_idx = match found {
                            Some(idx) => {
                                // Register ownership for kill-after-steal
                                stolen_from = task_piece_owner[idx].load(Ordering::Relaxed);
                                idx
                            }
                            None => {
                                // Phase 129: Dynamic Bisection — if no unstarted pieces remain,
                                // find the slowest in-progress circuit and bisect its active piece.
                                // This handles the IDM scenario where one segment stalls while all
                                // others are complete.
                                let bisect_target = (0..task_total_pieces)
                                    .filter(|&i| {
                                        !task_piece_flags[i].load(Ordering::Relaxed)
                                            && task_piece_owner[i].load(Ordering::Relaxed) != usize::MAX
                                            && task_piece_owner[i].load(Ordering::Relaxed) != circuit_id
                                    })
                                    .next();
                                match bisect_target {
                                    Some(idx) => {
                                        stolen_from = task_piece_owner[idx].load(Ordering::Relaxed);
                                        idx
                                    }
                                    None => break, // truly nothing left
                                }
                            }
                        };
                        PieceSpan {
                            start_piece: piece_idx,
                            end_piece: piece_idx,
                            start_byte: piece_idx as u64 * task_piece_size,
                            end_byte: (((piece_idx as u64) + 1) * task_piece_size)
                                .saturating_sub(1)
                                .min(task_content_length.saturating_sub(1)),
                        }
                    };

                    for owned_piece in piece_span.start_piece..=piece_span.end_piece {
                        task_piece_owner[owned_piece].store(circuit_id, Ordering::Relaxed);
                    }

                    let piece_idx = piece_span.start_piece;
                    let piece_end_idx = piece_span.end_piece;
                    let piece_start = piece_span.start_byte;
                    let piece_end = piece_span.end_byte;
                    let mut current_offset = piece_start;
                    let piece_timer = Instant::now();

                    while current_offset <= piece_end && task_running.load(Ordering::Relaxed) {
                        // In steal mode, check if original owner finished this piece
                        if stealing && task_piece_flags[piece_idx].load(Ordering::Relaxed) {
                            break; // Original owner won the race — move to next
                        }

                        if let Some(reason) = task_control.interruption_reason() {
                            task_running.store(false, Ordering::Relaxed);
                            return TaskOutcome::Interrupted(reason);
                        }

                        let bbr_chunk_size = task_aimd.recommended_chunk_size();
                        let current_chunk_end = (current_offset + bbr_chunk_size - 1).min(piece_end);
                        let Some(_host_permit) =
                            acquire_download_host_permit(&task_url, task_is_onion, Some(&task_control)).await
                        else {
                            task_running.store(false, Ordering::Relaxed);
                            return TaskOutcome::Interrupted("download stopped");
                        };

                        let response_future = {
                            let mut req = apply_download_connection_policy(
                                circuit_client
                                    .get(&task_url)
                                    .header("Range", &format!("bytes={current_offset}-{current_chunk_end}")),
                                &task_url,
                                task_is_onion,
                                task_download_telemetry.as_ref(),
                            );
                            if let Some(if_range) = &task_if_range {
                                req = req.header("If-Range", if_range);
                            }
                            req.send()
                        };

                        let response = match tokio::time::timeout(Duration::from_secs(45), response_future).await {
                            Ok(Ok(resp)) => {
                                // Reset global fail counter on success
                                task_server_fails.store(0, Ordering::Relaxed);
                                task_aimd.on_success_blind(); // Phase 4.4
                                task_scorer.record_download_success(circuit_id); // Phase 130: CUSUM
                                resp
                            }
                            Ok(Err(err)) => {
                                stalls += 1;
                                task_aimd.on_reject(); // Phase 4.4
                                task_scorer.record_download_failure(circuit_id); // Phase 130: CUSUM
                                let fails = task_server_fails.fetch_add(1, Ordering::Relaxed);

                                if err.to_string().contains("connect") || err.to_string().contains("request") {
                                    let _ = task_app.emit("log", format!("[🛡] Swarm Evasion: Circuit {} connection reset. Rotating client slot {}...", circuit_id, task_daemon_port));
                                }

                                // Phase 131: Collective 503 back-off — when global fails exceed
                                // circuit count, all circuits are being rejected. Pause 5-8s to let
                                // the server cool down instead of frantically recycling identities
                                // (each rebuild costs 2-3s of Tor handshake time).
                                if fails > 30 {
                                    let cooldown = Duration::from_secs(5 + (fails as u64 / 50).min(3));
                                    tokio::time::sleep(cooldown).await;
                                    continue;
                                }

                                // Phase 130: CUSUM-triggered early recycling — detect degradation
                                // before MAX_STALL_RETRIES accumulate (typically ~3-4 failures vs 5).
                                if task_scorer.should_recycle(circuit_id) {
                                    let _ = task_app.emit("log", format!("[⚡] CUSUM: Circuit {} degraded. Recycling identity...", circuit_id));
                                    circuit_client = circuit_client.new_isolated();
                                    task_scorer.reset_health(circuit_id);
                                    stalls = 0;
                                    continue;
                                }

                                if stalls > MAX_STALL_RETRIES {
                                    let _ = task_app.emit("log", format!("[↻] Supervisor self-healing: Circuit {} rejected on piece {}. Rebuilding identity...", circuit_id, piece_idx));
                                    circuit_client = circuit_client.new_isolated();
                                    stalls = 0;
                                    continue;
                                }
                                tokio::time::sleep(backoff_duration(stalls)).await;
                                continue;
                            }
                            Err(_) => {
                                stalls += 1;
                                task_aimd.on_timeout(); // Phase 4.4
                                task_scorer.record_download_failure(circuit_id); // Phase 130: CUSUM
                                let fails = task_server_fails.fetch_add(1, Ordering::Relaxed);
                                // Phase 130: CUSUM-triggered early recycling on timeout
                                if task_scorer.should_recycle(circuit_id) {
                                    let _ = task_app.emit("log", format!("[⚡] CUSUM: Circuit {} timeout-degraded. Recycling...", circuit_id));
                                    circuit_client = circuit_client.new_isolated();
                                    task_scorer.reset_health(circuit_id);
                                    stalls = 0;
                                    continue;
                                }
                                if stalls > MAX_STALL_RETRIES {
                                    let _ = task_app.emit("log", format!("[↻] Supervisor self-healing: Circuit {} header timeout on piece {}. Rebuilding identity...", circuit_id, piece_idx));
                                    circuit_client = circuit_client.new_isolated();
                                    stalls = 0;
                                    continue;
                                }
                                // Phase 131: Collective timeout back-off
                                if fails > 30 {
                                    let cooldown = Duration::from_secs(5 + (fails as u64 / 50).min(3));
                                    tokio::time::sleep(cooldown).await;
                                    continue;
                                }
                                tokio::time::sleep(backoff_duration(stalls)).await;
                                continue;
                            }
                        };

                        if let Some(delay) = ddos_guard.record_response_legacy(response.status().as_u16()) {
                            tokio::time::sleep(delay).await;
                        }

                        if response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
                            stalls += 1;
                            task_server_fails.fetch_add(1, Ordering::Relaxed);
                            task_aimd.on_reject(); // Phase 4.4: bad status = server pushback

                            let status = response.status();
                            if status == reqwest::StatusCode::TOO_MANY_REQUESTS
                                || status == reqwest::StatusCode::SERVICE_UNAVAILABLE
                            {
                                let _ = task_app.emit("log", format!("[🛡] Swarm Evasion: Circuit {} hit HTTP {}. Rotating client slot {}...", circuit_id, status, task_daemon_port));
                            }

                            // Phase 131: On 503/429 status, check for collective back-off
                            // before wasting time recycling the circuit identity.
                            let fails = task_server_fails.load(Ordering::Relaxed);
                            if fails > 30 {
                                let cooldown = Duration::from_secs(5 + (fails as u64 / 50).min(3));
                                tokio::time::sleep(cooldown).await;
                                continue;
                            }

                            if stalls > MAX_STALL_RETRIES {
                                let _ = task_app.emit("log", format!("[↻] Supervisor self-healing: Circuit {} bad status on piece {}. Rebuilding identity...", circuit_id, piece_idx));
                                circuit_client = circuit_client.new_isolated();
                                stalls = 0;
                                continue;
                            }
                            tokio::time::sleep(backoff_duration(stalls)).await;
                            continue;
                        }

                        let mut stream = response.bytes_stream();
                        let mut progressed = false;
                        let mut low_speed_tracker: Option<LowSpeedTracker> = None;
                        let mut low_speed_abort = false;

                        loop {
                            // Check if original owner won the race (steal mode)
                            if stealing && task_piece_flags[piece_idx].load(Ordering::Relaxed) {
                                drop(stream);
                                break;
                            }

                            if let Some(reason) = task_control.interruption_reason() {
                                task_running.store(false, Ordering::Relaxed);
                                return TaskOutcome::Interrupted(reason);
                            }

                            match tokio::time::timeout(
                                Duration::from_secs(STREAM_TIMEOUT_SECS),
                                stream.next(),
                            )
                            .await
                            {
                                Ok(Some(Ok(chunk))) => {
                                    if chunk.is_empty() {
                                        continue;
                                    }
                                    progressed = true;
                                    stalls = 0;
                                    let now = Instant::now();
                                    if let Some(tracker) = low_speed_tracker.as_mut() {
                                        if tracker.observe_progress(
                                            now,
                                            chunk.len() as u64,
                                            task_low_speed_policy,
                                        ) {
                                            low_speed_abort = true;
                                            if let Some(telemetry) = &task_download_telemetry {
                                                telemetry.record_download_low_speed_abort();
                                            }
                                            record_host_failure(
                                                &task_url,
                                                task_is_onion,
                                                HostFailureKind::LowSpeed,
                                            );
                                            break;
                                        }
                                    } else {
                                        low_speed_tracker =
                                            Some(LowSpeedTracker::new(now, chunk.len() as u64));
                                    }

                                    let len = chunk.len() as u64;
                                    let mut mmap_written = false;
                                    let mut final_chunk = chunk;
                                    if let Some(mmap) = &task_shared_mmap {
                                        mmap.write_slice(current_offset as usize, &final_chunk);
                                        mmap_written = true;
                                        final_chunk = bytes::Bytes::new();
                                    }

                                    let mut m = WriteMsg {
                                        filepath: task_path.clone(),
                                        offset: current_offset,
                                        data: final_chunk,
                                        mmap_written,
                                        close_file: false,
                                        chunk_id: piece_idx,
                                        piece_end: piece_end_idx,
                                    };
                                    while let Err(err) = task_tx.push(m) {
                                        m = err;
                                        // Phase 105: Yield back to async reactor to avoid starving TCP receive buffers
                                        tokio::task::yield_now().await;
                                    }

                                    current_offset = current_offset.saturating_add(len);
                                    task_total.fetch_add(len, Ordering::Relaxed);
                                    // Track per-circuit bytes for health monitor
                                    if circuit_id < task_circuit_bytes.len() {
                                        task_circuit_bytes[circuit_id].fetch_add(len, Ordering::Relaxed);
                                    }

                                    if current_offset > current_chunk_end {
                                        break;
                                    }
                                }
                                Ok(Some(Err(err))) => {
                                    let _ = task_app.emit(
                                        "log",
                                        format!("[*] Circuit {} transient error: {}. Restarting piece {}...", circuit_id, err, piece_idx),
                                    );
                                    drop(stream);
                                    tokio::time::sleep(Duration::from_millis(500)).await;
                                    break;
                                }
                                Ok(None) => {
                                    if current_offset <= piece_end && current_offset <= current_chunk_end {
                                        let _ = task_app.emit(
                                            "log",
                                            format!("[*] Circuit {} stream dropped prematurely on piece {}. Re-establishing...", circuit_id, piece_idx),
                                        );
                                        tokio::time::sleep(Duration::from_millis(500)).await;
                                    }
                                    drop(stream);
                                    break;
                                }
                                Err(_) => {
                                    if low_speed_tracker
                                        .as_ref()
                                        .map(|tracker| {
                                            tracker.should_abort_on_idle(
                                                Instant::now(),
                                                task_low_speed_policy,
                                            )
                                        })
                                        .unwrap_or(false)
                                    {
                                        low_speed_abort = true;
                                        if let Some(telemetry) = &task_download_telemetry {
                                            telemetry.record_download_low_speed_abort();
                                        }
                                        record_host_failure(
                                            &task_url,
                                            task_is_onion,
                                            HostFailureKind::LowSpeed,
                                        );
                                        break;
                                    }
                                    let _ = task_app.emit(
                                        "log",
                                        format!("[!] Circuit {} stalled {}s on piece {}. Reconnecting...", circuit_id, STREAM_TIMEOUT_SECS, piece_idx),
                                    );
                                    drop(stream);
                                    tokio::time::sleep(Duration::from_millis(500)).await;
                                    break;
                                }
                            }
                        }

                        if low_speed_abort {
                            stalls += 1;
                            if task_is_onion {
                                circuit_client = circuit_client.new_isolated();
                            }
                            tokio::time::sleep(backoff_duration(stalls)).await;
                            continue;
                        }

                        if !progressed {
                            stalls += 1;
                            if stalls > MAX_STALL_RETRIES {
                                let _ = task_app.emit("log", format!("[↻] Supervisor self-healing: Circuit {} max stalls streaming piece {}. Rebuilding identity...", circuit_id, piece_idx));
                                circuit_client = circuit_client.new_isolated();
                                stalls = 0;
                                continue;
                            }
                            tokio::time::sleep(backoff_duration(stalls)).await;
                        }
                    }

                    // Piece completed — but only mark if we actually finished it (not stolen from under us)
                    if current_offset > piece_end {
                        let mut newly_completed_pieces = 0usize;
                        let mut newly_completed_bytes = 0u64;
                        for completed_piece in piece_idx..=piece_end_idx {
                            if !task_piece_flags[completed_piece].swap(true, Ordering::Relaxed) {
                                newly_completed_pieces += 1;
                                let completed_start = completed_piece as u64 * task_piece_size;
                                let completed_end = (((completed_piece as u64) + 1) * task_piece_size)
                                    .saturating_sub(1)
                                    .min(task_content_length.saturating_sub(1));
                                newly_completed_bytes +=
                                    completed_end.saturating_sub(completed_start) + 1;
                            }
                        }

                        if newly_completed_pieces == 0 {
                            continue;
                        }

                        pieces_completed += newly_completed_pieces;

                        // Phase 3: Record piece stats in UCB1 scorer
                        let piece_bytes = newly_completed_bytes;
                        let piece_ms = piece_timer.elapsed().as_millis() as u64;
                        task_scorer.record_piece(circuit_id, piece_bytes, piece_ms);
                        task_aimd.on_success(piece_bytes, piece_ms.max(1));
                        record_host_success(
                            &task_url,
                            task_is_onion,
                            piece_bytes,
                            None,
                        );

                        // Phase 4.2: Predictive pre-warming — kill degrading circuits early
                        if task_scorer.is_degrading(circuit_id) && circuit_id < task_circuit_killed.len() {
                            task_circuit_killed[circuit_id].store(true, Ordering::Relaxed);
                            let _ = task_app.emit("log", format!(
                                "[⚡] Phase 4.2: Circuit {} degrading (latency 2.5× baseline) → pre-emptive recycle",
                                circuit_id
                            ));
                        }

                        let mut m = WriteMsg {
                            filepath: task_path.clone(),
                            offset: 0,
                            data: bytes::Bytes::new(),
                            mmap_written: false,
                            close_file: true,
                            chunk_id: piece_idx,
                            piece_end: piece_end_idx,
                        };
                        while let Err(err) = task_tx.push(m) {
                            m = err;
                            // Phase 105: Disk backpressure non-blocking yield
                            tokio::task::yield_now().await;
                        }

                        if stealing {
                            // Kill the original slow circuit
                            if stolen_from != usize::MAX
                                && stolen_from != circuit_id
                                && stolen_from < task_circuit_killed.len()
                            {
                                task_circuit_killed[stolen_from].store(true, Ordering::Relaxed);
                                let _ = task_app.emit(
                                    "log",
                                    format!(
                                        "[+] Circuit {} STOLE piece {} → killed slow circuit {}",
                                        circuit_id, piece_idx, stolen_from
                                    ),
                                );
                            } else {
                                let _ = task_app.emit(
                                    "log",
                                    format!("[+] Circuit {} STOLE piece {}", circuit_id, piece_idx),
                                );
                            }
                        }
                    }

                }

                TaskOutcome::Completed
            });
        }
    } else {
        logger.log(
            &app,
            "[!] Engaging 1-Circuit Fallback Stream Mode".to_string(),
        );
        let stream_client = get_arti_client(is_onion, 0, &active_client_ptrs)?;
        let task_tx = tx.clone();
        let task_app = app.clone();
        let task_url = entry.url.clone();
        let task_path = entry.path.clone();
        let task_control = control.clone();
        let task_running = Arc::clone(&run_flag);
        let task_total = Arc::clone(&total_downloaded);
        let total_hint = probe.content_length;

        tasks.spawn(async move {
            use futures::StreamExt;

            let mut current_offset = 0u64;
            let mut retries = 0usize;
            let low_speed_policy = download_low_speed_policy(is_onion);

            while task_running.load(Ordering::Relaxed) {
                if let Some(reason) = task_control.interruption_reason() {
                    task_running.store(false, Ordering::Relaxed);
                    return TaskOutcome::Interrupted(reason);
                }

                let Some(_host_permit) =
                    acquire_download_host_permit(&task_url, is_onion, Some(&task_control)).await
                else {
                    task_running.store(false, Ordering::Relaxed);
                    return TaskOutcome::Interrupted("download stopped");
                };
                let response_future = apply_download_connection_policy(
                    stream_client.get(&task_url),
                    &task_url,
                    is_onion,
                    download_telemetry.as_ref(),
                )
                .send();

                let response =
                    match tokio::time::timeout(Duration::from_secs(45), response_future).await {
                        Ok(Ok(resp)) => resp,
                        Ok(Err(err)) => {
                            retries += 1;
                            if retries > MAX_STALL_RETRIES {
                                return TaskOutcome::Failed(format!(
                                    "stream request failed repeatedly: {err}"
                                ));
                            }
                            tokio::time::sleep(backoff_duration(retries)).await;
                            continue;
                        }
                        Err(_) => {
                            retries += 1;
                            if retries > MAX_STALL_RETRIES {
                                return TaskOutcome::Failed(
                                    "stream request timeout (header wait)".to_string(),
                                );
                            }
                            tokio::time::sleep(backoff_duration(retries)).await;
                            continue;
                        }
                    };

                if !response.status().is_success() {
                    retries += 1;
                    if retries > MAX_STALL_RETRIES {
                        return TaskOutcome::Failed(format!(
                            "stream returned non-success status: {}",
                            response.status()
                        ));
                    }
                    tokio::time::sleep(backoff_duration(retries)).await;
                    continue;
                }

                let mut stream = response.bytes_stream();
                let mut progressed = false;
                let mut low_speed_tracker: Option<LowSpeedTracker> = None;
                let mut low_speed_abort = false;

                loop {
                    if let Some(reason) = task_control.interruption_reason() {
                        task_running.store(false, Ordering::Relaxed);
                        return TaskOutcome::Interrupted(reason);
                    }

                    match tokio::time::timeout(
                        Duration::from_secs(STREAM_TIMEOUT_SECS),
                        stream.next(),
                    )
                    .await
                    {
                        Ok(Some(Ok(chunk))) => {
                            if chunk.is_empty() {
                                continue;
                            }

                            progressed = true;
                            retries = 0;
                            let now = Instant::now();
                            if let Some(tracker) = low_speed_tracker.as_mut() {
                                if tracker.observe_progress(
                                    now,
                                    chunk.len() as u64,
                                    low_speed_policy,
                                ) {
                                    low_speed_abort = true;
                                    if let Some(telemetry) = &download_telemetry {
                                        telemetry.record_download_low_speed_abort();
                                    }
                                    record_host_failure(
                                        &task_url,
                                        is_onion,
                                        HostFailureKind::LowSpeed,
                                    );
                                    break;
                                }
                            } else {
                                low_speed_tracker =
                                    Some(LowSpeedTracker::new(now, chunk.len() as u64));
                            }

                            let len = chunk.len() as u64;
                            let mut m = WriteMsg {
                                filepath: task_path.clone(),
                                offset: current_offset,
                                data: chunk,
                                mmap_written: false,
                                close_file: false,
                                chunk_id: 0,
                                piece_end: 0,
                            };
                            while let Err(err) = task_tx.push(m) {
                                m = err;
                                tokio::task::yield_now().await;
                            }

                            current_offset = current_offset.saturating_add(len);
                            task_total.fetch_add(len, Ordering::Relaxed);
                        }
                        Ok(Some(Err(err))) => {
                            let _ = task_app.emit(
                                "log",
                                format!("[*] Stream transient error: {err}. Re-establishing..."),
                            );
                            drop(stream);
                            tokio::time::sleep(Duration::from_millis(500)).await;
                            break;
                        }
                        Ok(None) => {
                            if current_offset >= total_hint && total_hint > 0 {
                                let mut m = WriteMsg {
                                    filepath: task_path.clone(),
                                    offset: 0,
                                    data: bytes::Bytes::new(),
                                    mmap_written: false,
                                    close_file: true,
                                    chunk_id: 0,
                                    piece_end: 0,
                                };
                                while let Err(err) = task_tx.push(m) {
                                    m = err;
                                    tokio::task::yield_now().await;
                                }

                                return TaskOutcome::Completed;
                            } else {
                                let _ = task_app.emit(
                                    "log",
                                    "[*] Stream dropped prematurely. Reconnecting...".to_string(),
                                );
                                drop(stream);
                                tokio::time::sleep(Duration::from_millis(500)).await;
                                break;
                            }
                        }
                        Err(_) => {
                            if low_speed_tracker
                                .as_ref()
                                .map(|tracker| {
                                    tracker.should_abort_on_idle(Instant::now(), low_speed_policy)
                                })
                                .unwrap_or(false)
                            {
                                low_speed_abort = true;
                                if let Some(telemetry) = &download_telemetry {
                                    telemetry.record_download_low_speed_abort();
                                }
                                record_host_failure(&task_url, is_onion, HostFailureKind::LowSpeed);
                                break;
                            }
                            let _ = task_app.emit(
                                "log",
                                format!(
                                    "[!] Stream stalled for {}s. Reconnecting...",
                                    STREAM_TIMEOUT_SECS
                                ),
                            );
                            drop(stream);
                            tokio::time::sleep(Duration::from_millis(500)).await;
                            break;
                        }
                    }
                }

                if low_speed_abort {
                    retries += 1;
                    if retries > MAX_STALL_RETRIES {
                        return TaskOutcome::Failed(
                            "stream low-speed aborted too many times".to_string(),
                        );
                    }
                    tokio::time::sleep(backoff_duration(retries)).await;
                    continue;
                }

                if !progressed {
                    retries += 1;
                    if retries > MAX_STALL_RETRIES {
                        return TaskOutcome::Failed("stream stalled too many times".to_string());
                    }
                } else {
                    record_host_success(&task_url, is_onion, current_offset.max(1), None);
                }

                tokio::time::sleep(backoff_duration(retries)).await;
            }

            if let Some(reason) = task_control.interruption_reason() {
                TaskOutcome::Interrupted(reason)
            } else {
                TaskOutcome::Failed("stream stopped before completion".to_string())
            }
        });
    }

    // drop(tx); // Removed because ArrayQueue uses EOF poison pill

    let mut interruption: Option<&'static str> = None;
    let mut failure: Option<String> = None;

    while let Some(res) = tasks.join_next().await {
        match res {
            Ok(TaskOutcome::Completed) => {}
            Ok(TaskOutcome::Interrupted(reason)) => {
                interruption.get_or_insert(reason);
            }
            Ok(TaskOutcome::Failed(err)) => {
                failure.get_or_insert(err);
            }
            Err(err) => {
                failure.get_or_insert(format!("download task join failure: {err}"));
            }
        }

        // Early completion: if all bytes downloaded, abort remaining tasks
        if range_mode {
            let downloaded = total_downloaded.load(Ordering::Relaxed);
            if downloaded >= probe.content_length {
                logger.log(
                    &app,
                    "[+] All data received — aborting remaining circuits".to_string(),
                );
                tasks.abort_all();
                break;
            }
        }
    }

    run_flag.store(false, Ordering::Relaxed);

    // Poison pill to shut down the lock-free background writer
    let mut eof = WriteMsg {
        filepath: String::new(),
        offset: 0,
        data: bytes::Bytes::new(),
        mmap_written: false,
        close_file: true,
        chunk_id: usize::MAX,
        piece_end: usize::MAX,
    };
    while let Err(err) = tx.push(eof) {
        eof = err;
        tokio::task::yield_now().await;
    }
    let _ = speed_handle.await;
    if let Some(telemetry) = telemetry_handle(&app) {
        telemetry.set_active_circuits(0);
    }

    let writer_hash = match writer_handle.await {
        Ok(Ok(h)) => h,
        Ok(Err(err)) => {
            failure.get_or_insert(err.to_string());
            None
        }
        Err(err) => {
            failure.get_or_insert(format!("writer task join failure: {err}"));
            None
        }
    };

    let _ = app.emit(
        "tor_status",
        TorStatusEvent {
            state: "stopped".to_string(),
            message: "Tor daemons shutting down.".to_string(),
            daemon_count,
        },
    );

    if let Some(reason) = interruption {
        if reason == "Stopped" {
            let _ = fs::remove_file(&state_file_path);
        }

        let _ = app.emit(
            "log",
            format!("[*] Download {} for {}", reason.to_lowercase(), entry.path),
        );

        let _ = app.emit(
            "download_interrupted",
            DownloadInterruptedEvent {
                url: entry.url.clone(),
                path: entry.path.clone(),
                reason: reason.to_string(),
            },
        );
        return Ok(());
    }

    if let Some(err) = failure {
        // Only treat as real failure if we didn't get all the data.
        // Individual circuit task failures are normal (stalls, timeouts, etc.)
        // — the overall download succeeds if all bytes were received.
        let downloaded = total_downloaded.load(Ordering::Relaxed);
        if !range_mode || downloaded < probe.content_length {
            logger.log(
                &app,
                format!(
                    "[!] Download failed (got {} / {} bytes): {}",
                    downloaded, probe.content_length, err
                ),
            );
            return Err(anyhow!(err));
        }
        // All data received despite task errors — proceed to SHA verification
        logger.log(
            &app,
            format!(
                "[*] Ignoring {} task errors — all data received successfully",
                err
            ),
        );
    }

    let download_elapsed = start_time.elapsed().as_secs_f64();

    logger.log(
        &app,
        "[+] Download complete. Verifying SHA256...".to_string(),
    );
    let _ = app.emit(
        "download_status",
        serde_json::json!({
            "phase": "sha256_started",
            "message": "Download complete — SHA256 verification in progress...",
            "download_time_secs": download_elapsed,
        }),
    );
    binary_telemetry::emit_frame(
        EventKind::DownloadStatus,
        DownloadStatusFrame {
            phase: "sha256_started".to_string(),
            message: "Download complete — SHA256 verification in progress...".to_string(),
            download_time_secs: Some(download_elapsed),
            percent: None,
        },
    );

    let sha_start = Instant::now();
    let output_target_clone = entry.path.clone();
    let content_length = probe.content_length;

    // Channel for SHA progress reporting
    let (sha_tx, mut sha_rx) = tokio::sync::mpsc::channel::<u64>(64);
    let sha_app = app.clone();

    // SHA progress watcher — emits updates every 300ms
    let sha_watcher = tokio::spawn(async move {
        let mut hashed_bytes: u64 = 0;
        loop {
            match tokio::time::timeout(Duration::from_millis(300), sha_rx.recv()).await {
                Ok(Some(bytes)) => {
                    hashed_bytes = bytes;
                }
                Ok(None) => break, // Channel closed, SHA done
                Err(_) => {}       // Timeout, emit current progress
            }

            // Drain any buffered updates to get latest
            while let Ok(bytes) = sha_rx.try_recv() {
                hashed_bytes = bytes;
            }

            if content_length > 0 && hashed_bytes > 0 {
                let pct = (hashed_bytes as f64 / content_length as f64 * 100.0).min(100.0);
                let elapsed = sha_start.elapsed().as_secs_f64();
                let speed = if elapsed > 0.0 {
                    hashed_bytes as f64 / elapsed
                } else {
                    0.0
                };
                let eta = if speed > 0.0 {
                    (content_length.saturating_sub(hashed_bytes) as f64 / speed).max(0.0)
                } else {
                    -1.0
                };

                let hashed_mb = hashed_bytes as f64 / 1_048_576.0;
                let total_mb = content_length as f64 / 1_048_576.0;
                let eta_str = if (0.0..3600.0).contains(&eta) {
                    format!("ETA {:.0}s", eta)
                } else {
                    "calculating...".to_string()
                };

                let msg = if total_mb >= 1024.0 {
                    format!(
                        "SHA256: {:.0}% ({:.1} GB / {:.1} GB) — {}",
                        pct,
                        hashed_mb / 1024.0,
                        total_mb / 1024.0,
                        eta_str
                    )
                } else {
                    format!(
                        "SHA256: {:.0}% ({:.0} MB / {:.0} MB) — {}",
                        pct, hashed_mb, total_mb, eta_str
                    )
                };

                let _ = sha_app.emit(
                    "download_status",
                    serde_json::json!({
                        "phase": "sha256_progress",
                        "message": msg,
                        "pct": pct,
                        "eta_secs": eta,
                    }),
                );
                binary_telemetry::emit_frame(
                    EventKind::DownloadStatus,
                    DownloadStatusFrame {
                        phase: "sha256_progress".to_string(),
                        message: msg,
                        download_time_secs: None,
                        percent: Some(pct),
                    },
                );
            }
        }
    });

    let hash = if let Some(h) = writer_hash {
        let _ = app.emit(
            "download_status",
            serde_json::json!({
                "phase": "sha256_progress",
                "message": "SHA256: 100% (In-Flight Accelerated)",
                "pct": 100.0,
                "eta_secs": 0.0,
            }),
        );
        h
    } else {
        tokio::task::spawn_blocking(move || -> Result<String> {
            let mut file = File::open(&output_target_clone)?;
            let mut hasher = Sha256::new();
            // Massively accelerate SHA256 disk I/O with 4MB pipelined heap buffers
            let mut buffer = vec![0u8; 4 * 1024 * 1024];
            let mut total_hashed: u64 = 0;
            let mut last_report: u64 = 0;
            loop {
                let bytes = file.read(&mut buffer)?;
                if bytes == 0 {
                    break;
                }
                hasher.update(&buffer[..bytes]);
                total_hashed += bytes as u64;
                // Report every ~5MB to avoid flooding the channel
                if total_hashed - last_report >= 5_242_880 {
                    let _ = sha_tx.blocking_send(total_hashed);
                    last_report = total_hashed;
                }
            }
            let _ = sha_tx.blocking_send(total_hashed); // Final report
            drop(sha_tx); // Close channel to signal watcher
            Ok(hex::encode(hasher.finalize()))
        })
        .await??
    };

    sha_watcher.abort(); // Stop watcher

    let sha_elapsed = sha_start.elapsed().as_secs_f64();
    let _ = app.emit(
        "download_status",
        serde_json::json!({
            "phase": "sha256_complete",
            "message": format!("SHA256 verified in {:.1}s", sha_elapsed),
            "hash": hash,
            "sha_time_secs": sha_elapsed,
        }),
    );
    binary_telemetry::emit_frame(
        EventKind::DownloadStatus,
        DownloadStatusFrame {
            phase: "sha256_complete".to_string(),
            message: format!("SHA256 verified in {:.1}s", sha_elapsed),
            download_time_secs: Some(sha_elapsed),
            percent: Some(100.0),
        },
    );
    logger.log(
        &app,
        format!("[+] SHA256 verified in {:.1}s: {}", sha_elapsed, hash),
    );
    // Phase 135: No rename needed — file was downloaded directly to final path.
    logger.log(&app, format!("[+] Download verified at final path: {}", entry.path));

    let _ = app.emit(
        "complete",
        DownloadCompleteEvent {
            url: entry.url.clone(),
            path: entry.path.clone(),
            hash,
            time_taken_secs: start_time.elapsed().as_secs_f64(),
        },
    );

    // Log human-readable time taken
    let total_secs = start_time.elapsed().as_secs();
    let hours = total_secs / 3600;
    let mins = (total_secs % 3600) / 60;
    let secs = total_secs % 60;
    let time_str = if hours > 0 {
        format!("{}h {}m {}s", hours, mins, secs)
    } else if mins > 0 {
        format!("{}m {}s", mins, secs)
    } else {
        format!("{}s", secs)
    };
    logger.log(
        &app,
        format!("[✓] Total time: {} | File: {}", time_str, entry.path),
    );

    let _ = fs::remove_file(state_file_path);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        build_piece_spans, current_epoch_ms, download_host_capabilities, estimate_downloaded_bytes,
        host_capability_key, normalize_download_state, piece_len_for_index, record_host_success,
        DownloadState, HostCapabilityState, PieceSpan,
    };

    fn clear_host_capability(url: &str, is_onion: bool) {
        let Some(key) = host_capability_key(url, is_onion) else {
            return;
        };
        if let Ok(mut guard) = download_host_capabilities().write() {
            guard.remove(&key);
        }
    }

    #[test]
    fn piece_spans_coalesce_contiguous_missing_runs() {
        let spans = build_piece_spans(&[true, false, false, false, true, false, false], 10, 70, 2);
        assert_eq!(
            spans,
            vec![
                PieceSpan {
                    start_piece: 1,
                    end_piece: 2,
                    start_byte: 10,
                    end_byte: 29,
                },
                PieceSpan {
                    start_piece: 3,
                    end_piece: 3,
                    start_byte: 30,
                    end_byte: 39,
                },
                PieceSpan {
                    start_piece: 5,
                    end_piece: 6,
                    start_byte: 50,
                    end_byte: 69,
                },
            ]
        );
    }

    #[test]
    fn piece_spans_clamp_final_piece_to_content_length() {
        let spans = build_piece_spans(&[false, false, false], 10, 25, 4);
        assert_eq!(
            spans,
            vec![PieceSpan {
                start_piece: 0,
                end_piece: 2,
                start_byte: 0,
                end_byte: 24,
            }]
        );
    }

    #[test]
    fn piece_mode_resume_accounting_tracks_partial_progress_beyond_first_wave() {
        let mut state = DownloadState {
            completed_chunks: vec![false; 4],
            current_offsets: vec![0; 4],
            num_circuits: 4,
            chunk_size: 0,
            content_length: 100,
            piece_mode: true,
            completed_pieces: vec![false; 10],
            total_pieces: 10,
            etag: None,
            last_modified: None,
        };
        normalize_download_state(&mut state, 4, 10, 10);
        state.completed_pieces[0] = true;
        state.completed_pieces[1] = true;
        state.current_offsets[4] = 6;
        state.current_offsets[9] = 5;

        assert_eq!(estimate_downloaded_bytes(&state, 4), 31);
    }

    #[test]
    fn normalize_download_state_expands_piece_offsets_to_total_piece_count() {
        let mut state = DownloadState {
            completed_chunks: vec![false; 4],
            current_offsets: vec![0; 4],
            num_circuits: 4,
            chunk_size: 0,
            content_length: 95,
            piece_mode: true,
            completed_pieces: vec![false; 8],
            total_pieces: 8,
            etag: None,
            last_modified: None,
        };

        normalize_download_state(&mut state, 4, 10, 10);

        assert_eq!(state.current_offsets.len(), 10);
        assert_eq!(state.completed_pieces.len(), 10);
        assert_eq!(state.chunk_size, 10);
        assert_eq!(piece_len_for_index(95, 10, 9), 5);
    }

    use super::{
        batch_entry_host, batch_large_threshold_bytes, batch_micro_threshold_bytes,
        batch_no_byte_requeue_limit, batch_swarm_first_byte_timeout, diversify_first_wave_by_host,
        handshake_keep_count, ordered_probe_candidates, plan_batch_lanes,
        probe_timeout_for_attempt, record_host_failure, remap_queued_file_to_next_alternate,
        reseed_probe_alternates, schedule_srpt_with_starvation, BatchFileEntry, HostFailureKind,
        QueuedBatchFile, ScheduledBatchFile, DEFAULT_BATCH_LARGE_THRESHOLD_CLEARNET,
        DEFAULT_BATCH_LARGE_THRESHOLD_ONION_HEAVY, DEFAULT_BATCH_MICRO_THRESHOLD,
    };
    use crate::resource_governor::{DownloadBudget, GovernorPressure};

    #[test]
    fn test_srpt_scheduling_order() {
        let files = vec![
            ScheduledBatchFile {
                entry: BatchFileEntry {
                    url: "a".to_string(),
                    path: "a".to_string(),
                    size_hint: Some(100),
                    jwt_exp: None,
                    alternate_urls: Vec::new(),
                },
                estimated_size: 100,
                enqueue_order: 0,
                prefetched_probe: None,
            },
            ScheduledBatchFile {
                entry: BatchFileEntry {
                    url: "b".to_string(),
                    path: "b".to_string(),
                    size_hint: Some(10),
                    jwt_exp: None,
                    alternate_urls: Vec::new(),
                },
                estimated_size: 10,
                enqueue_order: 1,
                prefetched_probe: None,
            },
            ScheduledBatchFile {
                entry: BatchFileEntry {
                    url: "c".to_string(),
                    path: "c".to_string(),
                    size_hint: Some(50),
                    jwt_exp: None,
                    alternate_urls: Vec::new(),
                },
                estimated_size: 50,
                enqueue_order: 2,
                prefetched_probe: None,
            },
        ];

        std::env::set_var("CRAWLI_BATCH_SRPT", "1");
        let scheduled = schedule_srpt_with_starvation(files);

        assert_eq!(scheduled.len(), 3);
        assert_eq!(scheduled[0].entry.url, "b");
        assert_eq!(scheduled[1].entry.url, "c");
        assert_eq!(scheduled[2].entry.url, "a");
    }

    #[test]
    fn onion_batch_promotes_mid_size_files_to_large_pipeline() {
        std::env::remove_var("CRAWLI_BATCH_MICRO_THRESHOLD_MIB");
        std::env::remove_var("CRAWLI_BATCH_LARGE_THRESHOLD_MIB");
        assert_eq!(batch_micro_threshold_bytes(), DEFAULT_BATCH_MICRO_THRESHOLD);
        assert_eq!(
            batch_large_threshold_bytes(true, 2_533, 120),
            DEFAULT_BATCH_LARGE_THRESHOLD_ONION_HEAVY
        );
        assert_eq!(
            batch_large_threshold_bytes(false, 2_533, 120),
            DEFAULT_BATCH_LARGE_THRESHOLD_CLEARNET
        );
    }

    #[test]
    fn onion_batch_overlap_reserves_large_lane_inside_stable_cap() {
        let budget = DownloadBudget {
            circuit_cap: 16,
            small_file_parallelism: 8,
            initial_active_cap: 10,
            tournament_cap: 24,
            micro_swarm_circuits: 8,
            pressure: GovernorPressure::default(),
        };
        let large_files = vec![
            BatchFileEntry {
                url: "a".to_string(),
                path: "a".to_string(),
                size_hint: Some(400 * 1_048_576),
                jwt_exp: None,
                alternate_urls: Vec::new(),
            },
            BatchFileEntry {
                url: "b".to_string(),
                path: "b".to_string(),
                size_hint: Some(300 * 1_048_576),
                jwt_exp: None,
                alternate_urls: Vec::new(),
            },
        ];

        let plan = plan_batch_lanes(&budget, true, 2_394, &large_files);

        assert!(plan.overlap_large_phase);
        assert_eq!(plan.large_pipeline_circuits, 4);
        assert_eq!(plan.micro_parallelism, 6);
        assert_eq!(plan.small_parallelism, 6);
        assert_eq!(
            plan.micro_parallelism + plan.small_parallelism + plan.large_pipeline_circuits,
            budget.circuit_cap
        );
    }

    #[test]
    fn small_or_clearnet_batches_keep_serial_large_phase() {
        let budget = DownloadBudget {
            circuit_cap: 16,
            small_file_parallelism: 8,
            initial_active_cap: 10,
            tournament_cap: 24,
            micro_swarm_circuits: 8,
            pressure: GovernorPressure::default(),
        };
        let one_large = vec![BatchFileEntry {
            url: "a".to_string(),
            path: "a".to_string(),
            size_hint: Some(32 * 1_048_576),
            jwt_exp: None,
            alternate_urls: Vec::new(),
        }];

        let onion_plan = plan_batch_lanes(&budget, true, 32, &one_large);
        let clearnet_plan = plan_batch_lanes(&budget, false, 4_240, &one_large);

        assert!(!onion_plan.overlap_large_phase);
        assert!(!clearnet_plan.overlap_large_phase);
        assert_eq!(onion_plan.large_pipeline_circuits, budget.circuit_cap);
        assert_eq!(clearnet_plan.large_pipeline_circuits, budget.circuit_cap);
    }

    #[test]
    fn first_wave_is_diversified_by_host() {
        let files = vec![
            ScheduledBatchFile {
                entry: BatchFileEntry {
                    url: "http://a.onion/file1".to_string(),
                    path: "1".to_string(),
                    size_hint: Some(1),
                    jwt_exp: None,
                    alternate_urls: Vec::new(),
                },
                estimated_size: 1,
                enqueue_order: 0,
                prefetched_probe: None,
            },
            ScheduledBatchFile {
                entry: BatchFileEntry {
                    url: "http://a.onion/file2".to_string(),
                    path: "2".to_string(),
                    size_hint: Some(2),
                    jwt_exp: None,
                    alternate_urls: Vec::new(),
                },
                estimated_size: 2,
                enqueue_order: 1,
                prefetched_probe: None,
            },
            ScheduledBatchFile {
                entry: BatchFileEntry {
                    url: "http://b.onion/file3".to_string(),
                    path: "3".to_string(),
                    size_hint: Some(3),
                    jwt_exp: None,
                    alternate_urls: Vec::new(),
                },
                estimated_size: 3,
                enqueue_order: 2,
                prefetched_probe: None,
            },
            ScheduledBatchFile {
                entry: BatchFileEntry {
                    url: "http://c.onion/file4".to_string(),
                    path: "4".to_string(),
                    size_hint: Some(4),
                    jwt_exp: None,
                    alternate_urls: Vec::new(),
                },
                estimated_size: 4,
                enqueue_order: 3,
                prefetched_probe: None,
            },
        ];

        let diversified = diversify_first_wave_by_host(files, 3);
        let first_hosts = diversified
            .iter()
            .take(3)
            .map(|file| batch_entry_host(&file.entry.url).unwrap())
            .collect::<Vec<_>>();

        assert_eq!(first_hosts, vec!["a.onion", "b.onion", "c.onion"]);
    }

    #[test]
    fn batch_swarm_defaults_enable_no_byte_escape_for_onion() {
        std::env::remove_var("CRAWLI_BATCH_SWARM_FIRST_BYTE_TIMEOUT_SECS");
        std::env::remove_var("CRAWLI_BATCH_NO_BYTE_REQUEUE_LIMIT");

        assert_eq!(
            batch_swarm_first_byte_timeout(true),
            std::time::Duration::from_secs(18)
        );
        assert_eq!(batch_no_byte_requeue_limit(), 2);
    }

    #[test]
    fn queued_file_remap_rotates_to_next_alternate_host() {
        let mut queued = QueuedBatchFile {
            entry: BatchFileEntry {
                url: "http://hosta.onion/root/file.pdf".to_string(),
                path: "file.pdf".to_string(),
                size_hint: Some(10),
                jwt_exp: None,
                alternate_urls: vec![
                    "http://hostb.onion/root/file.pdf".to_string(),
                    "http://hostc.onion/root/file.pdf".to_string(),
                ],
            },
            estimated_size: 10,
            requeue_count: 0,
            alternate_url_cursor: 0,
            prefetched_probe: None,
        };

        let remap = remap_queued_file_to_next_alternate(&mut queued).expect("should remap");

        assert_eq!(remap.0, "hosta.onion");
        assert_eq!(remap.1, "hostb.onion");
        assert_eq!(queued.entry.url, "http://hostb.onion/root/file.pdf");
        assert_eq!(queued.alternate_url_cursor, 1);
    }

    #[test]
    fn probe_candidate_order_demotes_quarantined_primary_host() {
        let primary = "http://quarantine-a.onion/root/file.pdf".to_string();
        clear_host_capability(&primary, true);
        record_host_failure(&primary, true, HostFailureKind::Timeout);

        let entry = BatchFileEntry {
            url: primary.clone(),
            path: "kent/root/file.pdf".to_string(),
            size_hint: Some(10),
            jwt_exp: None,
            alternate_urls: vec![
                "http://quarantine-b.onion/root/file.pdf".to_string(),
                "http://quarantine-c.onion/root/file.pdf".to_string(),
            ],
        };

        let ordered = ordered_probe_candidates(&entry, true);

        assert_eq!(ordered.len(), 3);
        assert_ne!(ordered[0].url, primary);
        assert_eq!(ordered.last().unwrap().url, primary);
    }

    #[test]
    fn repeated_onion_connect_failures_extend_quarantine_window() {
        let url = "http://cooldown-host.onion/root/file.pdf";
        clear_host_capability(url, true);
        record_host_success(url, true, 64 * 1024, Some(120));
        record_host_failure(url, true, HostFailureKind::Connect);
        let first_quarantine = super::host_capability_snapshot(url, true)
            .expect("host snapshot after first failure")
            .quarantine_until_epoch_ms;

        record_host_failure(url, true, HostFailureKind::Connect);
        let snapshot =
            super::host_capability_snapshot(url, true).expect("host snapshot after second failure");

        assert!(snapshot.quarantine_until_epoch_ms > first_quarantine);
        assert_eq!(snapshot.last_productive_epoch_ms, 0);
        clear_host_capability(url, true);
    }

    #[test]
    fn stale_failed_primary_host_loses_productive_priority() {
        let primary = "http://stale-primary.onion/root/file.pdf";
        let alternate = "http://fresh-alternate.onion/root/file.pdf";
        clear_host_capability(primary, true);
        clear_host_capability(alternate, true);

        let now_ms = current_epoch_ms();
        let primary_key = host_capability_key(primary, true).expect("primary key");
        let alternate_key = host_capability_key(alternate, true).expect("alternate key");
        if let Ok(mut guard) = download_host_capabilities().write() {
            guard.insert(
                primary_key,
                HostCapabilityState {
                    recent_successes: 3,
                    recent_failures: 4,
                    consecutive_connect_failures: 2,
                    last_productive_epoch_ms: now_ms.saturating_sub(10 * 60 * 1_000),
                    ..Default::default()
                },
            );
            guard.insert(
                alternate_key,
                HostCapabilityState {
                    recent_successes: 1,
                    last_productive_epoch_ms: now_ms,
                    ..Default::default()
                },
            );
        }

        let entry = BatchFileEntry {
            url: primary.to_string(),
            path: "kent/stale/file.pdf".to_string(),
            size_hint: Some(10),
            jwt_exp: None,
            alternate_urls: vec![alternate.to_string()],
        };

        let ordered = ordered_probe_candidates(&entry, true);

        assert_eq!(ordered.len(), 2);
        assert_eq!(
            batch_entry_host(&ordered[0].url).as_deref(),
            Some("fresh-alternate.onion")
        );
        assert_eq!(
            batch_entry_host(&ordered[1].url).as_deref(),
            Some("stale-primary.onion")
        );

        clear_host_capability(primary, true);
        clear_host_capability(alternate, true);
    }

    #[test]
    fn reseed_probe_alternates_uses_remaining_probe_order() {
        let mut entry = BatchFileEntry {
            url: "http://seed-a.onion/root/file.pdf".to_string(),
            path: "kent/seed/file.pdf".to_string(),
            size_hint: Some(10),
            jwt_exp: None,
            alternate_urls: vec![
                "http://seed-b.onion/root/file.pdf".to_string(),
                "http://seed-c.onion/root/file.pdf".to_string(),
            ],
        };
        let ordered = ordered_probe_candidates(&entry, true);
        let selected = ordered[1].url.clone();

        reseed_probe_alternates(&mut entry, &ordered, &selected);

        assert_eq!(entry.alternate_urls.len(), 2);
        assert!(entry.alternate_urls.iter().all(|url| url != &selected));
        assert_ne!(entry.alternate_urls[0], entry.alternate_urls[1]);
    }

    #[test]
    fn clearnet_handshake_filter_keeps_target_worker_set() {
        assert_eq!(handshake_keep_count(37, 32, false), 32);
        assert_eq!(handshake_keep_count(24, 16, false), 16);
    }

    #[test]
    fn onion_handshake_filter_still_culls_bottom_half() {
        assert_eq!(handshake_keep_count(24, 16, true), 12);
        assert_eq!(handshake_keep_count(1, 1, true), 1);
    }

    #[test]
    fn onion_probe_timeout_grades_up_for_alternates() {
        assert_eq!(
            probe_timeout_for_attempt("http://hosta.onion/file", 0, true),
            std::time::Duration::from_secs(8)
        );
        assert_eq!(
            probe_timeout_for_attempt("http://hostb.onion/file", 1, true),
            std::time::Duration::from_secs(12)
        );
        assert_eq!(
            probe_timeout_for_attempt("http://hostc.onion/file", 3, true),
            std::time::Duration::from_secs(20)
        );
    }

    #[test]
    fn onion_probe_timeout_without_alternates_is_conservative() {
        assert_eq!(
            probe_timeout_for_attempt("http://hosta.onion/file", 0, false),
            std::time::Duration::from_secs(10)
        );
    }
}
