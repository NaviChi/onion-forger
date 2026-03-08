use crate::adapters::CrawlerAdapter;
use crate::arti_client::ArtiClient;
use crate::binary_telemetry::{self, DownloadStatusFrame, EventKind};
use anyhow::{anyhow, Result};
use http::header::{ACCEPT_RANGES, CONTENT_RANGE};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};
use tauri::{AppHandle, Emitter, Manager};

use tokio::task::JoinSet;

#[cfg(target_os = "windows")]
use std::process::Command;

#[cfg(target_os = "windows")]
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
        // Build unique log filename: ariaforge_<filename>_<timestamp>.log
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default();
        let ts = now.as_secs();
        // Sanitize filename for log
        let safe_name = filename_hint.replace(['/', '\\', ':', '?', '*', '"', '<', '>', '|'], "_");
        let log_path = Path::new(output_dir).join(format!("ariaforge_{}_{}.log", safe_name, ts));

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
}
impl ActiveCircuitGuard {
    fn new(counter: Arc<AtomicUsize>) -> Self {
        counter.fetch_add(1, Ordering::Relaxed);
        Self { counter }
    }
}
impl Drop for ActiveCircuitGuard {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::Relaxed);
    }
}

const STREAM_TIMEOUT_SECS: u64 = 15;
const MAX_STALL_RETRIES: usize = 30;
const PROBE_SIZE: u64 = 102_400; // 100KB micro-probe (80% signal in 10% of time)
const HANDSHAKE_CULL_RATIO: f64 = 0.50; // Kill bottom 50% by handshake latency
const DEFAULT_DOWNLOAD_TOURNAMENT_CAP: usize = 48;
const DEFAULT_DOWNLOAD_INITIAL_ACTIVE_CAP_ONION: usize = 16;
const DEFAULT_DOWNLOAD_INITIAL_ACTIVE_CAP_CLEARNET: usize = 32;
const DEFAULT_DOWNLOAD_INITIAL_ACTIVE_MIN: usize = 4;
const DEFAULT_RESUME_COALESCE_PIECES: usize = 4;

// Phase 4.1: Adaptive piece sizing bounds
const MIN_PIECE_SIZE: u64 = 5_242_880; // 5MB minimum (Allows Tor TCP Window to reach maximum speed)
const MAX_PIECE_SIZE: u64 = 52_428_800; // 50MB maximum

/// Phase 4.1: Compute optimal piece size based on file size and circuit count.
/// Targets ~8 pieces per circuit to balance granularity vs overhead.
fn compute_piece_size(content_length: u64, circuits: usize) -> u64 {
    if content_length == 0 || circuits == 0 {
        return MIN_PIECE_SIZE;
    }
    let target_pieces_per_circuit = 8u64;
    let ideal = content_length / (circuits as u64 * target_pieces_per_circuit);
    ideal.clamp(MIN_PIECE_SIZE, MAX_PIECE_SIZE)
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

    /// How long a circuit should wait before claiming the next piece.
    /// Fast circuits: 0ms. Slow circuits: up to 1000ms.
    /// This naturally gives more work to faster circuits.
    fn yield_delay(&self, cid: usize) -> Duration {
        if cid >= self.capacity {
            return Duration::ZERO;
        }
        let my_score = self.thompson_score(cid);
        if my_score == f64::MAX {
            return Duration::ZERO;
        } // Untested, no delay

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

        // Map: top 50% → 0ms, bottom 50% → 0-1000ms proportional
        if ratio > 0.5 {
            Duration::ZERO
        } else {
            let delay_ms = ((0.5 - ratio) * 2000.0) as u64; // 0-1000ms
            Duration::from_millis(delay_ms.min(1000))
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
}

use crate::bbr::BbrController;

/// Exponential backoff: min(2^retries * 500ms, 30s)
fn backoff_duration(retries: usize) -> Duration {
    let base_ms = 500u64 * (1u64 << retries.min(6));
    Duration::from_millis(base_ms.min(30_000))
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

struct ProbeResult {
    content_length: u64,
    supports_ranges: bool,
    etag: Option<String>,
    last_modified: Option<String>,
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

async fn probe_target(
    client: &crate::arti_client::ArtiClient,
    url: &str,
    app: &AppHandle,
) -> Result<ProbeResult> {
    let mut content_length = 0u64;
    let mut supports_ranges = false;
    let mut etag = None;
    let mut last_modified = None;

    // Apply strict 8-second timeout to HEAD to prevent infinite proxy stalls
    match tokio::time::timeout(Duration::from_secs(8), client.head(url).send()).await {
        Ok(Ok(resp)) => {
            content_length = resp.content_length().unwrap_or(0);
            supports_ranges = resp
                .headers()
                .get(ACCEPT_RANGES)
                .and_then(|value| value.to_str().ok())
                .map(|value| value.to_ascii_lowercase().contains("bytes"))
                .unwrap_or(false);
            (etag, last_modified) = extract_resume_validators(&resp);
        }
        Ok(Err(err)) => {
            let _ = app.emit("log", format!("[!] HEAD probe failed: {err}"));
        }
        Err(_) => {
            let _ = app.emit(
                "log",
                "[!] HEAD probe timed out after 8s. Hostile proxy likely dropped TCP.".to_string(),
            );
        }
    }

    if content_length == 0 || !supports_ranges {
        let _ = app.emit(
            "log",
            "[*] HEAD probe insufficient. Attempting GET range probe...".to_string(),
        );

        match tokio::time::timeout(
            Duration::from_secs(8),
            client.get(url).header("Range", "bytes=0-1").send(),
        )
        .await
        {
            Ok(Ok(resp)) => {
                if resp.status() == StatusCode::PARTIAL_CONTENT {
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
            }
            Ok(Err(err)) => {
                let _ = app.emit("log", format!("[!] GET Range probe failed: {err}"));
                supports_ranges = false;
                content_length = 0;
            }
            Err(_) => {
                let _ = app.emit(
                    "log",
                    "[!] GET Range probe timed out after 8s. Forcing fallback stream mode..."
                        .to_string(),
                );
                supports_ranges = false;
                content_length = 0;
            }
        }
    }

    // Always succeed. If content_length remains 0, the caller falls back to 1-circuit stream mode.
    Ok(ProbeResult {
        content_length,
        supports_ranges: supports_ranges && content_length > 0,
        etag,
        last_modified,
    })
}

fn get_arti_client(is_onion: bool, circuit_id: usize) -> Result<ArtiClient> {
    if is_onion {
        let clients = crate::tor_native::active_tor_clients();
        if clients.is_empty() {
            return Err(anyhow::anyhow!("No active Tor clients available"));
        }
        let shared_client = &clients[circuit_id % clients.len()];
        let tor_client = if tokio::runtime::Handle::try_current().is_ok() {
            tokio::task::block_in_place(|| shared_client.blocking_read().clone())
        } else {
            shared_client.blocking_read().clone()
        };
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

/// Size threshold: files above this use the full work queue + steal mode (existing start_download).
/// Files below this are downloaded as whole files, one per circuit, concurrently.
const BATCH_LARGE_THRESHOLD: u64 = 100 * 1_048_576; // 100MB
const BATCH_MICRO_THRESHOLD: u64 = 5 * 1_048_576; // 5MB

#[derive(Clone)]
struct ScheduledBatchFile {
    entry: BatchFileEntry,
    estimated_size: u64,
    enqueue_order: usize,
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

fn schedule_srpt_with_starvation(mut files: Vec<ScheduledBatchFile>) -> Vec<BatchFileEntry> {
    if files.len() <= 1 || !srpt_scheduler_enabled() {
        files.sort_by_key(|file| file.enqueue_order);
        return files.into_iter().map(|file| file.entry).collect();
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
        dispatch_order.push(selected.entry);
        dispatched += 1;
    }

    dispatch_order
}

async fn process_swarm(
    phase_name: &str,
    app: AppHandle,
    files: Vec<BatchFileEntry>,
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
) {
    if files.is_empty() {
        return;
    }
    let total_sub = files.len();
    let next_file = Arc::new(AtomicUsize::new(0));
    let phase_completed = Arc::new(AtomicUsize::new(0));
    let phase_bytes = Arc::new(AtomicU64::new(0));
    let phase_bbr = Arc::new(BbrController::new(parallelism, parallelism));

    let _ = app.emit(
        "log",
        format!(
            "[*] {}: {} files across {} circuits",
            phase_name, total_sub, parallelism
        ),
    );

    let mut tasks = JoinSet::new();
    for circuit_id in 0..parallelism {
        let daemon_port = active_ports[circuit_id % daemon_count.max(1)] as usize;
        let client = match get_arti_client(is_onion, circuit_id) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let task_files = files.clone();
        let task_next = Arc::clone(&next_file);
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

        tasks.spawn(async move {
            loop {
                if task_control.interruption_reason().is_some() { break; }

                let file_idx = task_next.fetch_add(1, Ordering::Relaxed);
                if file_idx >= task_files.len() { break; }

                let entry = &task_files[file_idx];

                if let Some(dir) = Path::new(&entry.path).parent() {
                    let _ = fs::create_dir_all(dir);
                }

                let _active_guard = ActiveCircuitGuard::new(Arc::clone(&task_active_batch_circuits));
                let mut retries = 0;
                let mut success = false;
                let mut downloaded_len = 0u64;
                while retries < 5 && !success {
                    let resp = match tokio::time::timeout(
                        Duration::from_secs(120),
                        client.get(&entry.url).header("Connection", "close").send()
                    ).await {
                        Ok(Ok(r)) if r.status().is_success() => r,
                        Ok(Ok(r)) => {
                            let status = r.status();
                            if status == reqwest::StatusCode::TOO_MANY_REQUESTS
                                || status == reqwest::StatusCode::SERVICE_UNAVAILABLE
                            {
                                task_bbr.on_reject();
                                let _ = task_app.emit("log", format!("[🛡] Swarm Evasion: Small-file circuit {} hit HTTP {}. Rotating client slot {}...", circuit_id, status, daemon_port));
                                let slot = daemon_port;
                                tokio::spawn(async move { let _ = crate::tor::request_newnym_slot(slot).await; });
                            } else {
                                task_bbr.on_reject();
                            }
                            retries += 1;
                            let active = task_bbr.current_active();
                            let base = backoff_duration(retries);
                            let bbr_pause = if circuit_id >= active { Duration::from_millis(2000) } else { Duration::ZERO };
                            tokio::time::sleep(base + bbr_pause).await;
                            continue;
                        }
                        Ok(Err(err)) => {
                            task_bbr.on_reject();
                            if err.to_string().contains("connect") || err.to_string().contains("request") {
                                let _ = task_app.emit("log", format!("[🛡] Swarm Evasion: Small-file circuit {} connection reset. Rotating client slot {}...", circuit_id, daemon_port));
                                let slot = daemon_port;
                                tokio::spawn(async move { let _ = crate::tor::request_newnym_slot(slot).await; });
                            }
                            retries += 1;
                            let active = task_bbr.current_active();
                            let base = backoff_duration(retries);
                            let bbr_pause = if circuit_id >= active { Duration::from_millis(2000) } else { Duration::ZERO };
                            tokio::time::sleep(base + bbr_pause).await;
                            continue;
                        }
                        Err(_) => {
                            task_bbr.on_timeout();
                            retries += 1; tokio::time::sleep(backoff_duration(retries)).await; continue;
                        }
                    };

                    match tokio::time::timeout(Duration::from_secs(300), resp.bytes()).await {
                        Ok(Ok(bytes)) => {
                            let len = bytes.len() as u64;
                            if fs::write(&entry.path, &bytes).is_ok() {
                                downloaded_len = len;
                                success = true;
                                task_bbr.on_success(len, 1000);
                            }
                        }
                        _ => { retries += 1; tokio::time::sleep(backoff_duration(retries)).await; }
                    }
                }

                if success {
                    task_phase_completed.fetch_add(1, Ordering::Relaxed);
                    task_phase_bytes.fetch_add(downloaded_len, Ordering::Relaxed);
                    task_overall_bytes.fetch_add(downloaded_len, Ordering::Relaxed);
                    let completed = task_overall_completed.fetch_add(1, Ordering::Relaxed) + 1;
                    let failed = task_overall_failed.load(Ordering::Relaxed);
                    publish_batch_progress(
                        &task_app,
                        BatchProgressEvent {
                            completed,
                            failed,
                            total: total_files,
                            current_file: entry.path.clone(),
                            speed_mbps: 0.0,
                            downloaded_bytes: task_overall_bytes.load(Ordering::Relaxed),
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
                } else {
                    let failed = task_overall_failed.fetch_add(1, Ordering::Relaxed) + 1;
                    let completed = task_overall_completed.load(Ordering::Relaxed);
                    publish_batch_progress(
                        &task_app,
                        BatchProgressEvent {
                            completed,
                            failed,
                            total: total_files,
                            current_file: entry.path.clone(),
                            speed_mbps: 0.0,
                            downloaded_bytes: task_overall_bytes.load(Ordering::Relaxed),
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
        .map(|f| f.url.contains(".onion"))
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
    let mut _tor_guard: Option<crate::tor::TorProcessGuard> = None;
    let daemon_count = if is_onion {
        if crate::tor_native::active_tor_clients().is_empty() {
            match crate::tor::bootstrap_tor_cluster(app.clone(), batch_circuit_cap).await {
                Ok((guard, _ports)) => {
                    _tor_guard = Some(guard);
                }
                Err(err) => {
                    return Err(anyhow!(
                        "Failed to bootstrap Aria Forge Tor cluster for batch download: {}",
                        err
                    ));
                }
            }
        }
        let live_clients = crate::tor_native::active_tor_clients().len().max(1);
        active_ports = (0..live_clients).map(|idx| idx as u16).collect();
        live_clients
    } else {
        1
    };
    if active_ports.is_empty() {
        active_ports.push(0);
    }

    // -- Probe all files and sort into small vs large --
    let sniff_client = get_arti_client(is_onion, 0)?;
    let mut micro_candidates: Vec<ScheduledBatchFile> = Vec::new();
    let mut small_candidates: Vec<ScheduledBatchFile> = Vec::new();
    let mut large_candidates: Vec<ScheduledBatchFile> = Vec::new();
    let mut enqueue_order = 0usize;

    let _ = app.emit(
        "log",
        format!("[*] Batch: probing {} files...", files.len()),
    );

    for file in &files {
        if control.interruption_reason().is_some() {
            return Ok(());
        }

        // Smart Skip Idempotency (redundant fallback)
        if let Some(hint) = file.size_hint {
            if hint > 0 {
                if hint <= BATCH_MICRO_THRESHOLD {
                    micro_candidates.push(ScheduledBatchFile {
                        entry: file.clone(),
                        estimated_size: hint,
                        enqueue_order,
                    });
                } else if hint <= BATCH_LARGE_THRESHOLD {
                    small_candidates.push(ScheduledBatchFile {
                        entry: file.clone(),
                        estimated_size: hint,
                        enqueue_order,
                    });
                } else {
                    large_candidates.push(ScheduledBatchFile {
                        entry: file.clone(),
                        estimated_size: hint,
                        enqueue_order,
                    });
                }
                enqueue_order = enqueue_order.saturating_add(1);
                continue;
            }
        }

        match probe_target(&sniff_client, &file.url, &app).await {
            Ok(probe) => {
                let estimated_size = file.size_hint.unwrap_or(probe.content_length);
                if probe.content_length <= BATCH_MICRO_THRESHOLD {
                    micro_candidates.push(ScheduledBatchFile {
                        entry: file.clone(),
                        estimated_size,
                        enqueue_order,
                    });
                } else if probe.content_length <= BATCH_LARGE_THRESHOLD {
                    small_candidates.push(ScheduledBatchFile {
                        entry: file.clone(),
                        estimated_size,
                        enqueue_order,
                    });
                } else {
                    large_candidates.push(ScheduledBatchFile {
                        entry: file.clone(),
                        estimated_size,
                        enqueue_order,
                    });
                }
            }
            Err(_) => small_candidates.push(ScheduledBatchFile {
                entry: file.clone(),
                estimated_size: file
                    .size_hint
                    .unwrap_or(BATCH_LARGE_THRESHOLD.saturating_sub(1)),
                enqueue_order,
            }),
        }
        enqueue_order = enqueue_order.saturating_add(1);
    }

    let scheduler_enabled = srpt_scheduler_enabled();
    let starvation_interval = srpt_starvation_interval();
    let micro_files = schedule_srpt_with_starvation(micro_candidates);
    let small_files = schedule_srpt_with_starvation(small_candidates);
    let large_files = schedule_srpt_with_starvation(large_candidates);

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
            "[+] Batch routing: {} micro (bg) + {} small (concurrent) + {} large (pipeline)",
            micro_files.len(),
            small_files.len(),
            large_files.len()
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
        let micro_parallelism = batch_download_budget.micro_swarm_circuits;
        let total = total_files;
        let micro_jwt_cache = Arc::clone(&jwt_cache);

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
            )
            .await;
        })
    } else {
        tokio::spawn(async move {}) // No-op if empty
    };

    // -- Phase 1: Download small files concurrently (one file per circuit) --
    if !small_files.is_empty() {
        process_swarm(
            "Phase 1 (Small)",
            app.clone(),
            small_files,
            small_file_parallelism,
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
        )
        .await;
    }

    // -- Phase 2: Download large files with full pipeline (tournament + steal) --
    for (i, file) in large_files.iter().enumerate() {
        if control.interruption_reason().is_some() {
            break;
        }

        let _ = app.emit(
            "log",
            format!(
                "[*] Phase 2: Large file {}/{}: {}",
                i + 1,
                large_files.len(),
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
                speed_mbps: 0.0,
                downloaded_bytes: overall_downloaded_bytes.load(Ordering::Relaxed),
                active_circuits: None,
            },
        );

        let inner_control = DownloadControl::new();
        let result = start_download(
            app.clone(),
            file.clone(),
            batch_circuit_cap,
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
                publish_batch_progress(
                    &app,
                    BatchProgressEvent {
                        completed,
                        failed: overall_failed.load(Ordering::Relaxed),
                        total: total_files,
                        current_file: file.path.clone(),
                        speed_mbps: 0.0,
                        downloaded_bytes: overall_downloaded_bytes.load(Ordering::Relaxed),
                        active_circuits: None,
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
                        speed_mbps: 0.0,
                        downloaded_bytes: overall_downloaded_bytes.load(Ordering::Relaxed),
                        active_circuits: None,
                    },
                );
            }
        }
    }

    let completed = overall_completed.load(Ordering::Relaxed);
    let failed = overall_failed.load(Ordering::Relaxed);
    if let Some(telemetry) = &batch_telemetry {
        telemetry.set_active_circuits(0);
    }

    // Phase 0: Ensure micro background swarm has finished
    let _ = micro_swarm_handle.await;

    publish_batch_progress(
        &app,
        BatchProgressEvent {
            completed,
            failed,
            total: total_files,
            current_file: "Batch complete".to_string(),
            speed_mbps: 0.0,
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
    let is_onion = entry.url.contains(".onion") || force_tor;
    let download_telemetry = telemetry_handle(&app);
    let state_file_path = format!("{}.ariaforge_state", entry.path);
    // Download to a temp file with .ariaforge extension, rename on completion
    let temp_target = format!("{}.ariaforge", entry.path);

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
    let mut _tor_guard: Option<crate::tor::TorProcessGuard> = None;

    if is_onion {
        if crate::tor_native::active_tor_clients().is_empty() {
            // No running daemons found — bootstrap our own cluster
            logger.log(
                &app,
                "[*] No active TorForge client pool detected. Bootstrapping fresh Aria Forge cluster..."
                    .to_string(),
            );

            match crate::tor::bootstrap_tor_cluster(app.clone(), bootstrap_budget.circuit_cap).await
            {
                Ok((guard, ports)) => {
                    _tor_guard = Some(guard);
                    let _ = ports;
                    daemon_count = crate::tor_native::active_tor_clients().len();
                    active_ports = (0..daemon_count).map(|idx| idx as u16).collect();
                    logger.log(
                        &app,
                        format!(
                            "[✓] Aria Forge TorForge client pool ready: {} slots",
                            daemon_count
                        ),
                    );
                }
                Err(e) => {
                    return Err(anyhow!("Failed to bootstrap Aria Forge Tor cluster: {}", e));
                }
            }
        } else {
            daemon_count = crate::tor_native::active_tor_clients().len();
            active_ports = (0..daemon_count).map(|idx| idx as u16).collect();
            logger.log(
                &app,
                format!("[✓] Reusing {} active TorForge client slots", daemon_count),
            );
        }

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
    let sniff_client = get_arti_client(is_onion, 0)?;

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

    let probe = probe_target(&sniff_client, &entry.url, &app).await?;
    let range_mode = probe.supports_ranges;
    let download_budget = crate::resource_governor::recommend_download_budget(
        requested_circuits,
        Some(probe.content_length),
        is_onion,
        Some(Path::new(&entry.path)),
        download_telemetry.as_ref(),
    );
    logger.log(
        &app,
        format!(
            "[*] Download governor: range_mode={} content_length={} circuit_cap={} active_start={} tournament_cap={} pressure={:.2}",
            range_mode,
            probe.content_length,
            download_budget.circuit_cap,
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
                    if parsed.current_offsets.len() != effective_circuits {
                        parsed.current_offsets = vec![0; effective_circuits];
                    }
                    state = parsed;
                    is_resuming = true;
                    for (i, &done) in state.completed_chunks.iter().enumerate() {
                        if done {
                            let end_byte = if i == effective_circuits - 1 {
                                state.content_length.saturating_sub(1)
                            } else {
                                ((i as u64 + 1) * state.chunk_size).saturating_sub(1)
                            };
                            let start_byte = i as u64 * state.chunk_size;
                            starting_total_downloaded += end_byte.saturating_sub(start_byte) + 1;
                        } else {
                            starting_total_downloaded += state.current_offsets[i];
                        }
                    }
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
                    let _ = fs::remove_file(&temp_target);
                }
            }
        }
    }

    if let Some(parent_dir) = Path::new(&entry.path).parent() {
        fs::create_dir_all(parent_dir)?;
    }

    if range_mode {
        let piece_size = compute_piece_size(state.content_length, effective_circuits);
        let total_pieces = state.content_length.div_ceil(piece_size) as usize;
        if !state.piece_mode || state.total_pieces != total_pieces {
            state.piece_mode = true;
            state.total_pieces = total_pieces;
            if state.completed_pieces.len() != total_pieces {
                state.completed_pieces = vec![false; total_pieces];
            }
        }
    }

    let resume_if_range = preferred_if_range(&state.etag, &state.last_modified);
    if let Some(validator) = &resume_if_range {
        logger.log(&app, format!("[*] Resume validator active: {}", validator));
    }

    if !is_resuming {
        let file = open_file_with_adaptive_io(
            Path::new(&temp_target),
            false,
            true,
            true,
            true,
            &app,
            Some(&logger),
        )?;
        // Pre-allocate full file size to prevent fragmentation
        if range_mode && state.content_length > 0 {
            file.set_len(state.content_length)?;
            let _ = app.emit(
                "log",
                format!(
                    "[+] Pre-allocated {:.2} GB on disk",
                    state.content_length as f64 / 1_073_741_824.0
                ),
            );
            logger.log(
                &app,
                format!(
                    "[+] Pre-allocated {:.2} GB on disk",
                    state.content_length as f64 / 1_073_741_824.0
                ),
            );
        }
    }

    if range_mode {
        fs::write(&state_file_path, serde_json::to_string(&state)?)?;
    } else {
        let _ = fs::remove_file(&state_file_path);
    }

    let ring_buffer = Arc::new(crossbeam_queue::ArrayQueue::<WriteMsg>::new(10_000));
    let _ring_capacity: usize = 10_000; // Phase 49: backpressure threshold constant
    let tx = Arc::clone(&ring_buffer);
    let rx = Arc::clone(&ring_buffer);
    let state_for_writer = if range_mode {
        Some((state.clone(), state_file_path.clone()))
    } else {
        None
    };

    let writer_app = app.clone();
    let writer_logger = logger.clone();
    let writer_handle = tokio::task::spawn_blocking(move || -> Result<()> {
        let mut active_filepath = String::new();
        let mut active_file: Option<File> = None;
        let mut active_mmap: Option<memmap2::MmapMut> = None;
        let mut local_state = state_for_writer;
        let mut last_flush = Instant::now();
        let mut pieces_since_flush = 0u32; // Throttle state saves
        let mut last_write_end: u64 = u64::MAX; // Phase 4.5: track for write coalescing
        let mut idle_polls = 0u32;

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

            if !msg.data.is_empty() {
                if active_filepath != msg.filepath || active_file.is_none() {
                    if let Some(mmap) = active_mmap.as_mut() {
                        let _ = mmap.flush();
                    }
                    active_mmap = None;

                    if let Some(dir) = Path::new(&msg.filepath).parent() {
                        fs::create_dir_all(dir)?;
                    }
                    let file = open_file_with_adaptive_io(
                        Path::new(&msg.filepath),
                        true,
                        true,
                        true,
                        false,
                        &writer_app,
                        Some(&writer_logger),
                    )?;

                    if let Some((st, _)) = &local_state {
                        if st.content_length > 0 {
                            // Phase 7: HFT Memory-Mapped Virtual Disk (HDD compatibility)
                            let _ = file.set_len(st.content_length);
                            if let Ok(m) = unsafe { memmap2::MmapOptions::new().map_mut(&file) } {
                                active_mmap = Some(m);
                            }
                        }
                    }

                    active_filepath = msg.filepath.clone();
                    active_file = Some(file);
                    last_write_end = u64::MAX; // Reset on new file
                }

                if let Some(mmap) = active_mmap.as_mut() {
                    let start = msg.offset as usize;
                    let end = start + msg.data.len();
                    if end <= mmap.len() {
                        // Phase 7: Zero-Copy ram write!
                        mmap[start..end].copy_from_slice(&msg.data);
                    } else if let Some(file) = active_file.as_mut() {
                        // Fallback if out-of-bounds mapping
                        if msg.offset != last_write_end {
                            // On slower mechanical drives, out-of-order writes with many circuits
                            // can cause thrashing. The seek will succeed but kill IOPS.
                            file.seek(SeekFrom::Start(msg.offset))?;
                        }
                        file.write_all(&msg.data)?;
                        last_write_end = msg.offset + msg.data.len() as u64;
                    }
                } else if let Some(file) = active_file.as_mut() {
                    // Phase 35: Windows IO Mechanical HDD Optimization
                    // If mmap failed to allocate (e.g. low 4GB RAM or fragmented hard drive),
                    // fallback to standard file writes.
                    if msg.offset != last_write_end {
                        file.seek(SeekFrom::Start(msg.offset))?;
                    }
                    file.write_all(&msg.data)?;
                    last_write_end = msg.offset + msg.data.len() as u64;
                }

                if let Some((state, _)) = local_state.as_mut() {
                    if msg.chunk_id < state.current_offsets.len() {
                        let chunk_start = msg.chunk_id as u64 * state.chunk_size;
                        let written_global = msg.offset + msg.data.len() as u64;
                        let chunk_offset = written_global.saturating_sub(chunk_start);

                        if chunk_offset > state.current_offsets[msg.chunk_id] {
                            state.current_offsets[msg.chunk_id] = chunk_offset;
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

        Ok(())
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

                    // Build a fresh circuit
                    let client = match get_arti_client(unchoke_is_onion, unchoke_id) {
                        Ok(c) => c,
                        Err(_) => continue,
                    };

                    // Download a 100KB probe to measure speed
                    let probe_start = 0u64;
                    let probe_end = (PROBE_SIZE - 1).min(unchoke_content_length.saturating_sub(1));
                    let probe_timer = Instant::now();

                    let probe_ok = match tokio::time::timeout(Duration::from_secs(15), {
                        let mut req = client
                            .get(&unchoke_url)
                            .header("Range", &format!("bytes={probe_start}-{probe_end}"));
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
            for cid in 0..tournament_pool {
                let probe_url = entry.url.clone();
                let c = cid;
                let is_onion_clone = is_onion;
                let active_ports_clone = active_ports.clone();
                handshake_tasks.spawn(async move {
                    let port = active_ports_clone[c % daemon_count.max(1)] as usize;
                    let start = Instant::now();
                    let client = match get_arti_client(is_onion_clone, c) {
                        Ok(c) => c,
                        Err(_) => return (c, port, None, u128::MAX),
                    };
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
            while let Some(Ok(result)) = handshake_tasks.join_next().await {
                results.push(result);
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
            let keep_count =
                ((results.len() as f64 * (1.0 - HANDSHAKE_CULL_RATIO)) as usize).max(1);
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
            let task_url = entry.url.clone();
            let task_path = temp_target.clone();
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

            tasks.spawn(async move {
                use futures::StreamExt;
                let mut circuit_client = circuit_client; // Mutable for recycling
                let mut ddos_guard = crate::adapters::qilin_ddos_guard::DdosGuard::new();

                // === TOURNAMENT PROBE PHASE ===
                if !task_skip_tournament {
                    // Phase 1.2: 100KB micro-probe (instead of 1MB)
                    // TCP slow-start stabilizes at ~50KB through Tor, so 100KB
                    // captures 80% of the throughput signal in 10% of the time.
                    let probe_start = (circuit_id as u64 % task_total_pieces as u64) * task_piece_size;
                    let probe_end = (probe_start + PROBE_SIZE - 1).min(task_content_length.saturating_sub(1));

                    let probe_result = async {
                        let resp = tokio::time::timeout(Duration::from_secs(30), {
                            let mut req = circuit_client
                                .get(&task_url)
                                .header("Range", &format!("bytes={probe_start}-{probe_end}"))
                                .header("Connection", "close");
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
                        match get_arti_client(task_is_onion, new_socks_id) {
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
                            Some(idx) => idx,
                            None => return TaskOutcome::Completed,
                        };
                        stolen_from = task_piece_owner[piece_idx].load(Ordering::Relaxed);
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

                        let response_future = {
                            let mut req = circuit_client
                                .get(&task_url)
                                .header("Range", &format!("bytes={current_offset}-{current_chunk_end}"));
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
                                resp
                            }
                            Ok(Err(err)) => {
                                stalls += 1;
                                task_aimd.on_reject(); // Phase 4.4
                                let fails = task_server_fails.fetch_add(1, Ordering::Relaxed);

                                if err.to_string().contains("connect") || err.to_string().contains("request") {
                                    let _ = task_app.emit("log", format!("[🛡] Swarm Evasion: Circuit {} connection reset. Rotating client slot {}...", circuit_id, task_daemon_port));
                                    let slot = task_daemon_port;
                                    tokio::spawn(async move { let _ = crate::tor::request_newnym_slot(slot).await; });
                                }

                                if stalls > MAX_STALL_RETRIES {
                                    let _ = task_app.emit("log", format!("[↻] Supervisor self-healing: Circuit {} rejected on piece {}. Rebuilding identity...", circuit_id, piece_idx));
                                    circuit_client = circuit_client.new_isolated();
                                    stalls = 0;
                                    continue;
                                }
                                // Graceful degradation: if many circuits failing, pause longer
                                if fails > 50 {
                                    tokio::time::sleep(Duration::from_secs(10)).await;
                                }
                                tokio::time::sleep(backoff_duration(stalls)).await;
                                continue;
                            }
                            Err(_) => {
                                stalls += 1;
                                task_aimd.on_timeout(); // Phase 4.4
                                let fails = task_server_fails.fetch_add(1, Ordering::Relaxed);
                                if stalls > MAX_STALL_RETRIES {
                                    let _ = task_app.emit("log", format!("[↻] Supervisor self-healing: Circuit {} header timeout on piece {}. Rebuilding identity...", circuit_id, piece_idx));
                                    circuit_client = circuit_client.new_isolated();
                                    stalls = 0;
                                    continue;
                                }
                                if fails > 50 {
                                    tokio::time::sleep(Duration::from_secs(10)).await;
                                }
                                tokio::time::sleep(backoff_duration(stalls)).await;
                                continue;
                            }
                        };

                        if let Some(delay) = ddos_guard.record_response(response.status().as_u16()) {
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
                                let slot = task_daemon_port;
                                tokio::spawn(async move { let _ = crate::tor::request_newnym_slot(slot).await; });
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

                                    let len = chunk.len() as u64;
                                    let mut m = WriteMsg {
                                        filepath: task_path.clone(),
                                        offset: current_offset,
                                        data: chunk,
                                        close_file: false,
                                        chunk_id: piece_idx,
                                        piece_end: piece_end_idx,
                                    };
                                    while let Err(err) = task_tx.push(m) {
                                        m = err;
                                        // Phase 49: Disk backpressure — slow down when ring buffer >80% full
                                        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
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
                            close_file: true,
                            chunk_id: piece_idx,
                            piece_end: piece_end_idx,
                        };
                        while let Err(err) = task_tx.push(m) {
                            m = err;
                            // Phase 49: Disk backpressure — slow down when ring buffer is full
                            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
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
        let stream_client = get_arti_client(is_onion, 0)?;
        let task_tx = tx.clone();
        let task_app = app.clone();
        let task_url = entry.url.clone();
        let task_path = temp_target.clone();
        let task_control = control.clone();
        let task_running = Arc::clone(&run_flag);
        let task_total = Arc::clone(&total_downloaded);
        let total_hint = probe.content_length;

        tasks.spawn(async move {
            use futures::StreamExt;

            let mut current_offset = 0u64;
            let mut retries = 0usize;

            while task_running.load(Ordering::Relaxed) {
                if let Some(reason) = task_control.interruption_reason() {
                    task_running.store(false, Ordering::Relaxed);
                    return TaskOutcome::Interrupted(reason);
                }

                let response_future = stream_client.get(&task_url).send();

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

                            let len = chunk.len() as u64;
                            let mut m = WriteMsg {
                                filepath: task_path.clone(),
                                offset: current_offset,
                                data: chunk,
                                close_file: false,
                                chunk_id: 0,
                                piece_end: 0,
                            };
                            while let Err(err) = task_tx.push(m) {
                                m = err;
                                // Phase 49: Disk backpressure
                                tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
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
                                    close_file: true,
                                    chunk_id: 0,
                                    piece_end: 0,
                                };
                                while let Err(err) = task_tx.push(m) {
                                    m = err;
                                    // Phase 49: Disk backpressure
                                    tokio::time::sleep(tokio::time::Duration::from_millis(10))
                                        .await;
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

                if !progressed {
                    retries += 1;
                    if retries > MAX_STALL_RETRIES {
                        return TaskOutcome::Failed("stream stalled too many times".to_string());
                    }
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
        close_file: true,
        chunk_id: usize::MAX,
        piece_end: usize::MAX,
    };
    while let Err(err) = tx.push(eof) {
        eof = err;
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    let _ = speed_handle.await;
    if let Some(telemetry) = telemetry_handle(&app) {
        telemetry.set_active_circuits(0);
    }

    match writer_handle.await {
        Ok(Ok(())) => {}
        Ok(Err(err)) => {
            failure.get_or_insert(err.to_string());
        }
        Err(err) => {
            failure.get_or_insert(format!("writer task join failure: {err}"));
        }
    }

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
    let output_target_clone = temp_target.clone();
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

    let hash = tokio::task::spawn_blocking(move || -> Result<String> {
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
    .await??;

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
    // Rename from .ariaforge temp file to final name
    if let Err(e) = fs::rename(&temp_target, &entry.path) {
        logger.log(
            &app,
            format!("[!] Rename failed: {} — file is at {}", e, temp_target),
        );
    } else {
        logger.log(&app, format!("[+] Renamed to final: {}", entry.path));
    }

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
    use super::{build_piece_spans, PieceSpan};

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

    use super::{schedule_srpt_with_starvation, BatchFileEntry, ScheduledBatchFile};

    #[test]
    fn test_srpt_scheduling_order() {
        let files = vec![
            ScheduledBatchFile {
                entry: BatchFileEntry {
                    url: "a".to_string(),
                    path: "a".to_string(),
                    size_hint: Some(100),
                    jwt_exp: None,
                },
                estimated_size: 100,
                enqueue_order: 0,
            },
            ScheduledBatchFile {
                entry: BatchFileEntry {
                    url: "b".to_string(),
                    path: "b".to_string(),
                    size_hint: Some(10),
                    jwt_exp: None,
                },
                estimated_size: 10,
                enqueue_order: 1,
            },
            ScheduledBatchFile {
                entry: BatchFileEntry {
                    url: "c".to_string(),
                    path: "c".to_string(),
                    size_hint: Some(50),
                    jwt_exp: None,
                },
                estimated_size: 50,
                enqueue_order: 2,
            },
        ];

        std::env::set_var("CRAWLI_BATCH_SRPT", "1");
        let scheduled = schedule_srpt_with_starvation(files);

        assert_eq!(scheduled.len(), 3);
        assert_eq!(scheduled[0].url, "b");
        assert_eq!(scheduled[1].url, "c");
        assert_eq!(scheduled[2].url, "a");
    }
}
