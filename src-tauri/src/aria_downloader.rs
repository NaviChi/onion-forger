use anyhow::{anyhow, Result};
use reqwest::header::{ACCEPT_RANGES, CONTENT_RANGE, RANGE};
use reqwest::{Client, Proxy, StatusCode};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::mpsc;
use tokio::task::JoinSet;

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
        let safe_name = filename_hint
            .replace(['/', '\\', ':', '?', '*', '"', '<', '>', '|'], "_");
        let log_path = format!(
            "{}/ariaforge_{}_{}.log",
            output_dir.trim_end_matches('/'),
            safe_name,
            ts
        );

        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .ok();

        if file.is_some() {
            eprintln!("[DownloadLogger] Writing to {}", log_path);
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

pub fn get_tor_path(app: &AppHandle) -> Result<PathBuf> {
    fn append_tor_relative_path(path: &mut PathBuf) {
        #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
        {
            path.push("win_x64");
            path.push("tor");
            path.push("tor.exe");
        }

        #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
        {
            path.push("mac_x64");
            path.push("tor");
            path.push("tor");
        }

        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        {
            path.push("mac_aarch64");
            path.push("tor");
            path.push("tor");
        }

        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            path.push("linux_x64");
            path.push("tor");
            path.push("tor");
        }

        #[cfg(not(any(
            all(target_os = "windows", target_arch = "x86_64"),
            all(target_os = "macos", target_arch = "x86_64"),
            all(target_os = "macos", target_arch = "aarch64"),
            all(target_os = "linux", target_arch = "x86_64")
        )))]
        {
            path.push("unsupported_platform");
        }
    }

    let mut candidates: Vec<PathBuf> = Vec::new();

    if let Ok(resource_dir) = app.path().resource_dir() {
        let mut resource_path = resource_dir;
        resource_path.push("bin");
        append_tor_relative_path(&mut resource_path);
        candidates.push(resource_path);
    }

    if let Ok(current_exe) = std::env::current_exe() {
        if let Some(exe_dir) = current_exe.parent() {
            let mut sibling_bin = exe_dir.to_path_buf();
            sibling_bin.push("bin");
            append_tor_relative_path(&mut sibling_bin);
            candidates.push(sibling_bin);

            let mut mac_bundle_resources = exe_dir.to_path_buf();
            mac_bundle_resources.push("..");
            mac_bundle_resources.push("Resources");
            mac_bundle_resources.push("bin");
            append_tor_relative_path(&mut mac_bundle_resources);
            candidates.push(mac_bundle_resources);
        }
    }

    if let Some(found) = candidates.iter().find(|path| path.exists()) {
        return Ok(found.clone());
    }

    let searched = candidates
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    Err(anyhow!("Tor executable not found. Searched paths: {searched}"))
}

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

const TOR_DATA_DIR_PREFIX: &str = "loki_tor_";
const TOR_PID_FILE: &str = "loki_tor.pid";
const STREAM_TIMEOUT_SECS: u64 = 15;
const MAX_STALL_RETRIES: usize = 30;
const PROBE_SIZE: u64 = 102_400;  // 100KB micro-probe (80% signal in 10% of time)
const HANDSHAKE_CULL_RATIO: f64 = 0.50; // Kill bottom 50% by handshake latency

// Phase 4.1: Adaptive piece sizing bounds
const MIN_PIECE_SIZE: u64 = 5_242_880;   // 5MB minimum (Allows Tor TCP Window to reach maximum speed)
const MAX_PIECE_SIZE: u64 = 52_428_800;  // 50MB maximum

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

// Health monitoring: kill circuits below this fraction of median speed
const MIN_SPEED_RATIO: f64 = 0.20; // 20% of median = too slow
const HEALTH_CHECK_INTERVAL_SECS: u64 = 15;

// Phase 3: UCB1 Multi-Armed Bandit tuning
const UCB1_EXPLORATION_C: f64 = 1.5; // Exploration vs exploitation balance
const UNCHOKE_INTERVAL_SECS: u64 = 30; // Test a fresh circuit every 30s

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
    latency_baseline: Vec<AtomicU64>,  // Baseline from first 3 pieces (ms)
    latency_samples: Vec<AtomicU64>,   // Number of samples recorded
}

#[allow(dead_code)]
impl CircuitScorer {
    fn new(num_circuits: usize) -> Self {
        let mut kalmans = Vec::with_capacity(num_circuits);
        for _ in 0..num_circuits {
            // q = 10.0 (process noise), r = 100.0 (measurement noise for volatile Tor relays)
            kalmans.push(std::sync::Mutex::new(crate::kalman::KalmanFilter::new(10.0, 100.0, 0.0)));
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
        if cid >= self.capacity { return; }
        let samples = self.latency_samples[cid].fetch_add(1, Ordering::Relaxed);
        
        let mut kf = self.latency_kalman[cid].lock().unwrap();

        if samples < 3 {
            // Build baseline from first 3 pieces
            let old = self.latency_baseline[cid].load(Ordering::Relaxed);
            let new_baseline = if old == 0 { elapsed_ms } else { (old + elapsed_ms) / 2 };
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
        if cid >= self.capacity { return false; }
        let samples = self.latency_samples[cid].load(Ordering::Relaxed);
        if samples < 5 { return false; } // Need enough data
        
        let baseline = self.latency_baseline[cid].load(Ordering::Relaxed) as f64;
        if baseline == 0.0 { return false; }

        let kf = self.latency_kalman[cid].lock().unwrap();
        let prediction = kf.predict();

        // If predicted latency + uncertainty deviation > 2.5x baseline, it is stalling!
        let deviation = kf.p.sqrt();
        (prediction + (deviation * 1.5)) > (baseline * 2.5)
    }

    /// Compute UCB1 score for a circuit (higher = should get more pieces)
    fn ucb1_score(&self, cid: usize) -> f64 {
        if cid >= self.capacity { return 0.0; }
        let n = self.pieces_completed[cid].load(Ordering::Relaxed);
        if n == 0 {
            return f64::MAX; // Untested = infinite score (explore first)
        }
        let total_b = self.total_bytes[cid].load(Ordering::Relaxed) as f64;
        let total_ms = self.total_elapsed_ms[cid].load(Ordering::Relaxed).max(1) as f64;
        let avg_speed = total_b / total_ms; // bytes per ms

        let global = self.global_pieces.load(Ordering::Relaxed).max(1) as f64;
        let exploration = UCB1_EXPLORATION_C * (global.ln() / n as f64).sqrt();

        avg_speed + exploration
    }

    /// Compute average speed in MB/s for a circuit
    fn avg_speed_mbps(&self, cid: usize) -> f64 {
        if cid >= self.capacity { return 0.0; }
        let total_b = self.total_bytes[cid].load(Ordering::Relaxed) as f64;
        let total_ms = self.total_elapsed_ms[cid].load(Ordering::Relaxed).max(1) as f64;
        (total_b / total_ms) * 1000.0 / 1_048_576.0 // Convert bytes/ms to MB/s
    }

    /// How long a circuit should wait before claiming the next piece.
    /// Fast circuits: 0ms. Slow circuits: up to 1000ms.
    /// This naturally gives more work to faster circuits.
    fn yield_delay(&self, cid: usize) -> Duration {
        if cid >= self.capacity { return Duration::ZERO; }
        let my_score = self.ucb1_score(cid);
        if my_score == f64::MAX { return Duration::ZERO; } // Untested, no delay

        // Collect scores of all active circuits
        let mut scores: Vec<f64> = (0..self.capacity)
            .filter(|&i| self.pieces_completed[i].load(Ordering::Relaxed) > 0)
            .map(|i| self.ucb1_score(i))
            .collect();
        if scores.is_empty() { return Duration::ZERO; }

        scores.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
        let best = scores.first().copied().unwrap_or(1.0);
        if best <= 0.0 { return Duration::ZERO; }

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

/// Phase 4.4: AIMD (Additive Increase, Multiplicative Decrease) concurrency controller.
/// Dynamically adjusts active circuit count based on server response.
#[allow(dead_code)]
struct AimdController {
    active: AtomicUsize,
    max: usize,
    min: usize,
    consec_success: AtomicUsize,
}

#[allow(dead_code)]
impl AimdController {
    fn new(initial: usize, max: usize) -> Self {
        AimdController {
            active: AtomicUsize::new(initial),
            max,
            min: 1,
            consec_success: AtomicUsize::new(0),
        }
    }

    /// Call on successful piece download
    fn on_success(&self) {
        let consec = self.consec_success.fetch_add(1, Ordering::Relaxed);
        // Additive increase: +1 circuit every 20 consecutive successes
        if consec > 0 && consec % 20 == 0 {
            let current = self.active.load(Ordering::Relaxed);
            if current < self.max {
                self.active.store(current + 1, Ordering::Relaxed);
            }
        }
    }

    /// Call on server rejection (429, 503, connection refused)
    fn on_reject(&self) {
        self.consec_success.store(0, Ordering::Relaxed);
        // Multiplicative decrease: halve active circuits
        let current = self.active.load(Ordering::Relaxed);
        let new_val = (current / 2).max(self.min);
        self.active.store(new_val, Ordering::Relaxed);
    }

    /// Call on timeout (milder decrease)
    fn on_timeout(&self) {
        self.consec_success.store(0, Ordering::Relaxed);
        let current = self.active.load(Ordering::Relaxed);
        let new_val = (current * 3 / 4).max(self.min);
        self.active.store(new_val, Ordering::Relaxed);
    }

    /// Check if this circuit should be active
    fn should_be_active(&self, circuit_rank: usize) -> bool {
        circuit_rank < self.active.load(Ordering::Relaxed)
    }
}

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
}

pub struct WriteMsg {
    pub filepath: String,
    pub offset: u64,
    pub data: bytes::Bytes,
    pub close_file: bool,
    pub chunk_id: usize,
}

#[derive(Clone, Serialize)]
pub struct ProgressEvent {
    pub id: usize,
    pub downloaded: u64,
    pub total: u64,
    pub main_speed_mbps: f64,
    pub status: String,
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
pub struct SpeedEvent {
    pub speed_mbps: f64,
    pub elapsed_secs: f64,
    pub eta_secs: f64,  // -1 = unknown
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
}

fn parse_content_range_total(header_value: &str) -> Option<u64> {
    header_value
        .split('/')
        .next_back()
        .and_then(|value| value.parse::<u64>().ok())
}

fn terminate_pid(pid: u32) {
    #[cfg(target_os = "windows")]
    {
        let _ = Command::new("taskkill")
            .arg("/F")
            .arg("/PID")
            .arg(pid.to_string())
            .status();
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = Command::new("kill")
            .arg("-TERM")
            .arg(pid.to_string())
            .status();
        let _ = Command::new("kill")
            .arg("-KILL")
            .arg(pid.to_string())
            .status();
    }
}

fn cleanup_tor_data_dir(data_dir: &Path) {
    let pid_file = data_dir.join(TOR_PID_FILE);
    if let Ok(pid_value) = fs::read_to_string(&pid_file) {
        if let Ok(pid) = pid_value.trim().parse::<u32>() {
            terminate_pid(pid);
        }
    }
    let _ = fs::remove_file(pid_file);
    let _ = fs::remove_dir_all(data_dir);
}

pub fn cleanup_stale_tor_daemons() {
    let tmp_root = Path::new("/tmp");
    let entries = match fs::read_dir(tmp_root) {
        Ok(entries) => entries,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };

        if name.starts_with(TOR_DATA_DIR_PREFIX) {
            cleanup_tor_data_dir(&path);
        }
    }
}

fn is_port_available(port: u16) -> bool {
    std::net::TcpListener::bind(("127.0.0.1", port)).is_ok()
}


async fn probe_target(client: &Client, url: &str, app: &AppHandle) -> Result<ProbeResult> {
    let mut content_length = 0u64;
    let mut supports_ranges = false;

    match client.head(url).send().await {
        Ok(resp) => {
            content_length = resp.content_length().unwrap_or(0);
            supports_ranges = resp
                .headers()
                .get(ACCEPT_RANGES)
                .and_then(|value| value.to_str().ok())
                .map(|value| value.to_ascii_lowercase().contains("bytes"))
                .unwrap_or(false);
        }
        Err(err) => {
            let _ = app.emit("log", format!("[!] HEAD probe failed: {err}"));
        }
    }

    if content_length == 0 || !supports_ranges {
        let _ = app.emit(
            "log",
            "[*] HEAD probe insufficient. Attempting GET range probe...".to_string(),
        );

        if let Ok(resp) = client.get(url).header(RANGE, "bytes=0-1").send().await {
            if resp.status() == StatusCode::PARTIAL_CONTENT {
                supports_ranges = true;
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
    }

    Ok(ProbeResult {
        content_length,
        supports_ranges: supports_ranges && content_length > 0,
    })
}

fn range_download_client(is_onion: bool, daemon_port: usize, circuit_id: usize) -> Result<Client> {
    if is_onion {
        let proxy_url = format!("socks5h://u{circuit_id}:p{circuit_id}@127.0.0.1:{daemon_port}");
        let proxy = Proxy::all(&proxy_url)?;
        Ok(Client::builder()
            .proxy(proxy)
            .danger_accept_invalid_certs(true)
            .pool_max_idle_per_host(1) // Keep-alive: reuse TCP connection through Tor circuit
            .tcp_nodelay(true)
            .build()?)
    } else {
        Ok(Client::builder()
            .danger_accept_invalid_certs(true)
            .pool_max_idle_per_host(1)
            .tcp_nodelay(true)
            .build()?)
    }
}

fn stream_download_client(is_onion: bool, port: u16) -> Result<Client> {
    if is_onion {
        let proxy = Proxy::all(format!("socks5h://127.0.0.1:{}", port))?;
        Ok(Client::builder()
            .proxy(proxy)
            .danger_accept_invalid_certs(true)
            .pool_max_idle_per_host(0)
            .tcp_nodelay(true)
            .build()?)
    } else {
        Ok(Client::builder()
            .danger_accept_invalid_certs(true)
            .pool_max_idle_per_host(0)
            .tcp_nodelay(true)
            .build()?)
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
}

#[derive(Clone, Serialize)]
pub struct BatchProgressEvent {
    pub completed: usize,
    pub total: usize,
    pub current_file: String,
    pub speed_mbps: f64,
}

/// Size threshold: files above this use the full work queue + steal mode (existing start_download).
/// Files below this are downloaded as whole files, one per circuit, concurrently.
const BATCH_LARGE_THRESHOLD: u64 = 100 * 1_048_576; // 100MB

pub async fn start_batch_download(
    app: AppHandle,
    files: Vec<BatchFileEntry>,
    num_circuits: usize,
    force_tor: bool,
    control: DownloadControl,
) -> Result<()> {
    let requested_circuits = num_circuits.max(1);
    let is_onion = files.first().map(|f| f.url.contains(".onion")).unwrap_or(false) || force_tor;
    // Dynamically detect active Tor daemon ports
    let mut active_ports: Vec<u16> = Vec::new();
    let daemon_count;
    if is_onion {
        for port in 9051..=9054 {
            if !is_port_available(port) {
                active_ports.push(port);
            }
        }
        daemon_count = active_ports.len().max(1);
    } else {
        daemon_count = 1;
    }
    if active_ports.is_empty() {
        active_ports.push(9051);
    }

    // -- Probe all files and sort into small vs large --
    let sniff_client = stream_download_client(is_onion, active_ports[0])?;
    let mut small_files: Vec<BatchFileEntry> = Vec::new();
    let mut large_files: Vec<BatchFileEntry> = Vec::new();

    let _ = app.emit("log", format!("[*] Batch: probing {} files...", files.len()));

    for file in &files {
        if control.interruption_reason().is_some() { return Ok(()); }
        match probe_target(&sniff_client, &file.url, &app).await {
            Ok(probe) => {
                if probe.content_length <= BATCH_LARGE_THRESHOLD {
                    small_files.push(file.clone());
                } else {
                    large_files.push(file.clone());
                }
            }
            Err(_) => small_files.push(file.clone()),
        }
    }

    let _ = app.emit("log", format!(
        "[+] Batch routing: {} small (concurrent) + {} large (full pipeline)",
        small_files.len(), large_files.len()
    ));

    // -- Phase 1: Download small files concurrently (one file per circuit) --
    if !small_files.is_empty() {
        let total_small = small_files.len();
        let next_file = Arc::new(AtomicUsize::new(0));
        let files_completed = Arc::new(AtomicUsize::new(0));
        let total_bytes = Arc::new(AtomicU64::new(0));

        let _ = app.emit("log", format!(
            "[*] Phase 1: {} small files across {} circuits",
            total_small, requested_circuits
        ));

        let mut tasks = JoinSet::new();
        for circuit_id in 0..requested_circuits {
            let daemon_port = active_ports[circuit_id % daemon_count.max(1)] as usize;
            let client = match range_download_client(is_onion, daemon_port, circuit_id) {
                Ok(c) => c,
                Err(_) => continue,
            };

            let task_files = small_files.clone();
            let task_next = Arc::clone(&next_file);
            let task_done = Arc::clone(&files_completed);
            let task_bytes = Arc::clone(&total_bytes);
            let task_app = app.clone();
            let task_control = control.clone();

            tasks.spawn(async move {
                loop {
                    if task_control.interruption_reason().is_some() { break; }

                    let file_idx = task_next.fetch_add(1, Ordering::Relaxed);
                    if file_idx >= task_files.len() { break; }

                    let entry = &task_files[file_idx];

                    if let Some(dir) = Path::new(&entry.path).parent() {
                        let _ = fs::create_dir_all(dir);
                    }

                    // Download entire file with retries
                    let mut retries = 0;
                    let mut success = false;
                    while retries < 5 && !success {
                        let resp = match tokio::time::timeout(
                            Duration::from_secs(120),
                            client.get(&entry.url).header("Connection", "close").send()
                        ).await {
                            Ok(Ok(r)) if r.status().is_success() => r,
                            _ => { retries += 1; tokio::time::sleep(backoff_duration(retries)).await; continue; }
                        };

                        match tokio::time::timeout(Duration::from_secs(300), resp.bytes()).await {
                            Ok(Ok(bytes)) => {
                                let len = bytes.len() as u64;
                                if fs::write(&entry.path, &bytes).is_ok() {
                                    task_bytes.fetch_add(len, Ordering::Relaxed);
                                    success = true;
                                }
                            }
                            _ => { retries += 1; tokio::time::sleep(backoff_duration(retries)).await; }
                        }
                    }

                    let completed = task_done.fetch_add(1, Ordering::Relaxed) + 1;
                    let _ = task_app.emit("batch_progress", BatchProgressEvent {
                        completed,
                        total: task_files.len(),
                        current_file: entry.path.clone(),
                        speed_mbps: 0.0,
                    });
                }
            });
        }

        while tasks.join_next().await.is_some() {}

        let done = files_completed.load(Ordering::Relaxed);
        let bytes = total_bytes.load(Ordering::Relaxed);
        let _ = app.emit("log", format!(
            "[+] Phase 1 complete: {}/{} small files ({:.2} GB)",
            done, total_small, bytes as f64 / 1_073_741_824.0
        ));
    }

    // -- Phase 2: Download large files with full pipeline (tournament + steal) --
    for (i, file) in large_files.iter().enumerate() {
        if control.interruption_reason().is_some() { break; }

        let _ = app.emit("log", format!(
            "[*] Phase 2: Large file {}/{}: {}",
            i + 1, large_files.len(), file.path
        ));

        let inner_control = DownloadControl::new();
        let _ = start_download(
            app.clone(), file.url.clone(), file.path.clone(),
            num_circuits, force_tor, inner_control,
        ).await;
    }

    let _ = app.emit("log", format!("[✓] Batch complete: {} files processed", files.len()));
    Ok(())
}

pub async fn start_download(
    app: AppHandle,
    url: String,
    output_target: String,
    num_circuits: usize,
    force_tor: bool,
    control: DownloadControl,
) -> Result<()> {
    let requested_circuits = num_circuits.max(1);
    let is_onion = url.contains(".onion") || force_tor;
    let state_file_path = format!("{}.ariaforge_state", output_target);
    // Download to a temp file with .ariaforge extension, rename on completion
    let temp_target = format!("{}.ariaforge", output_target);

    // Create log file in the output directory
    let output_dir = Path::new(&output_target)
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| ".".to_string());
    let filename_hint = Path::new(&output_target)
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_else(|| "download".to_string());
    let logger = DownloadLogger::new(&output_dir, &filename_hint);
    logger.log(&app, "[*] Aria Forge Download Session".to_string());
    logger.log(&app, format!("[*] URL: {}", url));
    logger.log(&app, format!("[*] Output: {}", output_target));
    logger.log(&app, format!("[*] Circuits: {} | Tor: {}", requested_circuits, is_onion));

    // Detect or bootstrap Tor daemons dynamically
    let mut daemon_count = 0usize;
    let mut active_ports: Vec<u16> = Vec::new();
    let mut _tor_guard: Option<crate::tor::TorProcessGuard> = None;

    if is_onion {
        // Phase 1: Probe for already-running Crawli daemons on ports 9051-9054
        let candidate_ports: Vec<u16> = (9051..=9054).collect();
        for &port in &candidate_ports {
            if !is_port_available(port) {
                // Port is in use — likely a running Tor daemon
                active_ports.push(port);
            }
        }

        if active_ports.is_empty() {
            // No running daemons found — bootstrap our own cluster
            logger.log(&app, "[*] No active Tor daemons detected. Bootstrapping fresh Aria Forge cluster...".to_string());
            
            match crate::tor::bootstrap_tor_cluster(app.clone(), 4).await {
                Ok((guard, ports)) => {
                    _tor_guard = Some(guard);
                    active_ports = ports.clone();
                    daemon_count = ports.len();
                    logger.log(&app, format!("[✓] Aria Forge Tor cluster ready: {} daemons on {:?}", daemon_count, active_ports));
                },
                Err(e) => {
                    return Err(anyhow!("Failed to bootstrap Aria Forge Tor cluster: {}", e));
                }
            }
        } else {
            daemon_count = active_ports.len();
            logger.log(&app, format!("[✓] Reusing {} active Crawli Tor daemons on {:?}", daemon_count, active_ports));
        }

        let _ = app.emit(
            "tor_status",
            TorStatusEvent {
                state: "ready".to_string(),
                message: format!("Aria Forge: {} Tor daemons active on {:?}", daemon_count, active_ports),
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
        active_ports.push(9051); // Dummy port for clearnet (unused but prevents index panic)
        daemon_count = 1;
    }

    let primary_port = active_ports.first().copied().unwrap_or(9051);
    let sniff_client = stream_download_client(is_onion, primary_port)?;
    let probe = probe_target(&sniff_client, &url, &app).await?;
    let range_mode = probe.supports_ranges;

    let effective_circuits = if range_mode {
        requested_circuits
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
    };

    let mut is_resuming = false;
    let mut starting_total_downloaded = 0u64;
    if range_mode && Path::new(&state_file_path).exists() {
        if let Ok(content) = fs::read_to_string(&state_file_path) {
            if let Ok(mut parsed) = serde_json::from_str::<DownloadState>(&content) {
                if parsed.num_circuits == effective_circuits
                    && parsed.content_length == state.content_length
                    && parsed.completed_chunks.len() == effective_circuits
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
                }
            }
        }
    }

    if let Some(parent_dir) = Path::new(&output_target).parent() {
        fs::create_dir_all(parent_dir)?;
    }

    if !is_resuming {
        let mut opts = OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        crate::io_vanguard::apply_direct_io(&mut opts);
        let file = opts.open(&temp_target)?;
        crate::io_vanguard::post_open_config(&file);
        // Pre-allocate full file size to prevent fragmentation
        if range_mode && state.content_length > 0 {
            file.set_len(state.content_length)?;
            let _ = app.emit("log", format!(
                "[+] Pre-allocated {:.2} GB on disk",
                state.content_length as f64 / 1_073_741_824.0
            ));
            logger.log(&app, format!(
                "[+] Pre-allocated {:.2} GB on disk",
                state.content_length as f64 / 1_073_741_824.0
            ));
        }
    }

    if range_mode {
        fs::write(&state_file_path, serde_json::to_string(&state)?)?;
    } else {
        let _ = fs::remove_file(&state_file_path);
    }

    let (tx, mut rx) = mpsc::channel::<WriteMsg>(10_000);  // 10K buffer prevents circuit back-pressure
    let state_for_writer = if range_mode {
        Some((state.clone(), state_file_path.clone()))
    } else {
        None
    };

    let writer_handle = tokio::task::spawn_blocking(move || -> Result<()> {
        let mut active_filepath = String::new();
        let mut active_file: Option<File> = None;
        let mut local_state = state_for_writer;
        let mut last_flush = Instant::now();
        let mut pieces_since_flush = 0u32; // Throttle state saves
        let mut last_write_end: u64 = u64::MAX; // Phase 4.5: track for write coalescing

        while let Some(msg) = rx.blocking_recv() {
            let mut should_flush = false;

            if !msg.data.is_empty() {
                if active_filepath != msg.filepath || active_file.is_none() {
                    if let Some(dir) = Path::new(&msg.filepath).parent() {
                        fs::create_dir_all(dir)?;
                    }
                    let mut opts = OpenOptions::new();
                    opts.write(true).create(true).truncate(false);
                    crate::io_vanguard::apply_direct_io(&mut opts);
                    let file = opts.open(&msg.filepath)?;
                    crate::io_vanguard::post_open_config(&file);
                    active_filepath = msg.filepath.clone();
                    active_file = Some(file);
                    last_write_end = u64::MAX; // Reset on new file
                }

                if let Some(file) = active_file.as_mut() {
                    // Phase 4.5: Write coalescing — skip seek if writes are sequential
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
            }

            if msg.close_file {
                if let Some((state, _)) = local_state.as_mut() {
                    if msg.chunk_id < state.completed_chunks.len() {
                        state.completed_chunks[msg.chunk_id] = true;
                    }
                    if state.piece_mode && msg.chunk_id < state.completed_pieces.len() {
                        state.completed_pieces[msg.chunk_id] = true;
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
                    let _ = fs::write(path, serde_json::to_string(state).unwrap_or_default());
                }
            }
        }

        if let Some((state, path)) = local_state.as_mut() {
            let _ = fs::write(path, serde_json::to_string(state).unwrap_or_default());
        }

        Ok(())
    });

    let total_downloaded = Arc::new(AtomicU64::new(starting_total_downloaded));
    let run_flag = Arc::new(AtomicBool::new(true));
    let start_time = Instant::now();

    let watcher_total = Arc::clone(&total_downloaded);
    let active_circuits = Arc::new(AtomicUsize::new(0));
    let watcher_active = Arc::clone(&active_circuits);
    #[derive(Clone, serde::Serialize)]
    struct DownloadProgressEvent {
        path: String,
        bytes_downloaded: u64,
        total_bytes: Option<u64>,
        speed_bps: u64,
        active_circuits: usize,
    }

    let watcher_running = Arc::clone(&run_flag);
    let watcher_app = app.clone();
    let watcher_content_length = probe.content_length;
    let watcher_path = output_target.clone();
    let speed_handle = tokio::spawn(async move {
        while watcher_running.load(Ordering::Relaxed) {
            tokio::time::sleep(Duration::from_millis(500)).await;
            let downloaded = watcher_total.load(Ordering::Relaxed);
            let elapsed = start_time.elapsed().as_secs_f64();
            let speed_mbps = if elapsed > 0.0 {
                (downloaded as f64 / elapsed) / 1048576.0
            } else {
                0.0
            };
            let bytes_per_sec = if elapsed > 0.0 { downloaded as f64 / elapsed } else { 0.0 };
            let eta_secs = if bytes_per_sec > 0.0 && watcher_content_length > 0 {
                let remaining = watcher_content_length.saturating_sub(downloaded) as f64;
                remaining / bytes_per_sec
            } else {
                -1.0
            };
            
            let _ = watcher_app.emit("download_progress_update", DownloadProgressEvent {
                path: watcher_path.clone(),
                bytes_downloaded: downloaded,
                total_bytes: if watcher_content_length > 0 { Some(watcher_content_length) } else { None },
                speed_bps: bytes_per_sec as u64,
                active_circuits: watcher_active.load(Ordering::Relaxed),
            });

            let _ = watcher_app.emit("speed", SpeedEvent {
                speed_mbps,
                elapsed_secs: elapsed,
                eta_secs,
            });
        }
        let _ = watcher_app.emit("speed", SpeedEvent {
            speed_mbps: 0.0,
            elapsed_secs: start_time.elapsed().as_secs_f64(),
            eta_secs: 0.0,
        });
    });

    let mut tasks = JoinSet::new();

    if range_mode {
        let content_length = state.content_length;

        // Phase 4.1: Adaptive piece sizing
        let piece_size = compute_piece_size(content_length, effective_circuits);
        let total_pieces = content_length.div_ceil(piece_size) as usize;

        logger.log(&app, format!(
            "[*] Phase 4.1: Adaptive piece size: {:.1} MB ({} pieces)",
            piece_size as f64 / 1_048_576.0, total_pieces
        ));

        // Build or restore the piece completion tracker
        let piece_completed: Vec<bool> = if state.piece_mode && state.completed_pieces.len() == total_pieces {
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
                format!("[+] Resuming: {}/{} pieces already complete.", pieces_done_count, total_pieces),
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
        let scaled_circuits = total_pieces.clamp(1, effective_circuits);

        // Tournament: only justify 2x pool if enough pieces for circuits to compete
        // Need at least 2 pieces per circuit for tournament to be meaningful
        let tournament_pool = if total_pieces >= scaled_circuits * 3 {
            (scaled_circuits * 2).min(effective_circuits * 2) // Full tournament
        } else {
            scaled_circuits  // Not enough work — skip tournament, direct assign
        };
        let max_promoted = scaled_circuits;
        let skip_tournament = tournament_pool <= scaled_circuits;

        // Label the tier for UI logging
        let tier = if total_pieces <= 1 {
            "tiny"
        } else if scaled_circuits <= 10 {
            "small"
        } else if scaled_circuits <= 50 {
            "medium"
        } else {
            "large"
        };

        // Shared state: atomic next-piece index and piece completion array
        let next_piece = Arc::new(AtomicUsize::new(0));
        let piece_flags: Arc<Vec<AtomicBool>> = Arc::new(
            piece_completed.iter().map(|&done| AtomicBool::new(done)).collect()
        );
        // Track which circuit owns each in-progress piece (for kill-after-steal)
        let piece_owner: Arc<Vec<AtomicUsize>> = Arc::new(
            (0..total_pieces).map(|_| AtomicUsize::new(usize::MAX)).collect()
        );
        // Kill flags: when a circuit gets stolen from, it's marked for death
        let circuit_killed: Arc<Vec<AtomicBool>> = Arc::new(
            (0..tournament_pool).map(|_| AtomicBool::new(false)).collect()
        );
        // Per-circuit byte counters for health monitoring
        let circuit_bytes: Arc<Vec<AtomicU64>> = Arc::new(
            (0..tournament_pool).map(|_| AtomicU64::new(0)).collect()
        );
        // Global server health: rises when many circuits fail, triggers coordinated pause
        let server_fail_count = Arc::new(AtomicUsize::new(0));

        // Phase 3: UCB1 Multi-Armed Bandit scorer
        let circuit_scorer = Arc::new(CircuitScorer::new(tournament_pool));

        // Phase 4.4: AIMD concurrency controller
        let aimd = Arc::new(AimdController::new(scaled_circuits, effective_circuits));

        let _ = app.emit(
            "log",
            if skip_tournament {
                format!("[+] {} file ({} pieces) → {} circuits (no tournament)", tier, total_pieces, scaled_circuits)
            } else {
                format!("[+] {} file ({} pieces) → {} circuits | Tournament: {} racing for {} slots", tier, total_pieces, scaled_circuits, tournament_pool, max_promoted)
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
                    if !mon_running.load(Ordering::Relaxed) { break; }

                    // Compute per-circuit speed since last check
                    let mut speeds: Vec<(usize, f64)> = Vec::new();
                    for cid in 0..mon_pool_size {
                        if mon_killed[cid].load(Ordering::Relaxed) { continue; }
                        let current = mon_bytes[cid].load(Ordering::Relaxed);
                        let delta = current.saturating_sub(prev_bytes[cid]);
                        prev_bytes[cid] = current;
                        if current > 0 {
                            // Only track circuits that have downloaded something
                            let speed = delta as f64 / HEALTH_CHECK_INTERVAL_SECS as f64;
                            speeds.push((cid, speed));
                        }
                    }

                    if speeds.len() < 3 { continue; } // Need enough data points

                    // Compute median speed
                    let mut sorted_speeds: Vec<f64> = speeds.iter().map(|(_, s)| *s).collect();
                    sorted_speeds.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
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
            let unchoke_url = url.clone();
            let unchoke_is_onion = is_onion;
            let unchoke_content_length = content_length;
            let unchoke_active_ports = active_ports.clone();

            tokio::spawn(async move {
                // Wait for initial circuits to warm up
                tokio::time::sleep(Duration::from_secs(60)).await;
                let mut unchoke_id = 9000usize; // High IDs to avoid collision

                while unchoke_running.load(Ordering::Relaxed) {
                    tokio::time::sleep(Duration::from_secs(UNCHOKE_INTERVAL_SECS)).await;
                    if !unchoke_running.load(Ordering::Relaxed) { break; }

                    unchoke_id += 1;
                    let port = if unchoke_is_onion {
                        unchoke_active_ports[0] as usize // Use first daemon from active_ports
                    } else {
                        9051 // Fallback for non-onion, though unchoke_is_onion should handle this
                    };

                    // Build a fresh circuit
                    let client = match range_download_client(unchoke_is_onion, port, unchoke_id) {
                        Ok(c) => c,
                        Err(_) => continue,
                    };

                    // Download a 100KB probe to measure speed
                    let probe_start = 0u64;
                    let probe_end = (PROBE_SIZE - 1).min(unchoke_content_length.saturating_sub(1));
                    let probe_timer = Instant::now();

                    let probe_ok = match tokio::time::timeout(
                        Duration::from_secs(15),
                        client.get(&unchoke_url)
                            .header(RANGE, format!("bytes={probe_start}-{probe_end}"))
                            .send()
                    ).await {
                        Ok(Ok(resp)) if resp.status() == StatusCode::PARTIAL_CONTENT || resp.status() == StatusCode::OK => {
                            use futures::StreamExt;
                            let mut stream = resp.bytes_stream();
                            let mut bytes = 0u64;
                            loop {
                                match tokio::time::timeout(Duration::from_secs(7), stream.next()).await {
                                    Ok(Some(Ok(chunk))) => {
                                        bytes += chunk.len() as u64;
                                        if bytes >= PROBE_SIZE { break; }
                                    }
                                    _ => break,
                                }
                            }
                            bytes > 0
                        }
                        _ => false,
                    };

                    if !probe_ok { continue; }

                    let probe_ms = probe_timer.elapsed().as_millis() as u64;
                    let unchoke_speed = PROBE_SIZE as f64 / probe_ms.max(1) as f64; // bytes/ms

                    // Compare to slowest active circuit
                    if let Some(slowest_cid) = unchoke_scorer.slowest_circuit() {
                        let slowest_speed = {
                            let total_b = unchoke_scorer.total_bytes[slowest_cid].load(Ordering::Relaxed) as f64;
                            let total_ms = unchoke_scorer.total_elapsed_ms[slowest_cid].load(Ordering::Relaxed).max(1) as f64;
                            total_b / total_ms
                        };

                        if unchoke_speed > slowest_speed * 1.3 { // 30% faster threshold
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
        let mut circuit_candidates: Vec<(usize, reqwest::Client, usize)> = Vec::new();
        {
            logger.log(&app, format!(
                "[*] Phase 1: Handshake pre-filter — racing {} circuits...", tournament_pool
            ));

            let mut handshake_tasks = JoinSet::new();
            for cid in 0..tournament_pool {
                let probe_url = url.clone();
                let c = cid;
                let is_onion_clone = is_onion;
                let active_ports_clone = active_ports.clone();
                handshake_tasks.spawn(async move {
                    let port = active_ports_clone[c % daemon_count.max(1)] as usize;
                    let start = Instant::now();
                    let client = match range_download_client(is_onion_clone, port, c) {
                        Ok(c) => c,
                        Err(_) => return (c, port, None, u128::MAX),
                    };
                    // HEAD request to force the SOCKS5 handshake through Tor
                    let latency = match tokio::time::timeout(
                        Duration::from_secs(15),
                        client.head(&probe_url).send()
                    ).await {
                        Ok(Ok(_)) => start.elapsed().as_millis(),
                        _ => u128::MAX, // Failed — assign worst latency
                    };
                    (cid, port, Some(client), latency)
                });
            }

            // Collect all results
            let mut results: Vec<(usize, usize, Option<reqwest::Client>, u128)> = Vec::new();
            while let Some(Ok(result)) = handshake_tasks.join_next().await {
                results.push(result);
            }

            // Sort by latency (fastest first)
            results.sort_by_key(|r| r.3);

            // Keep top circuits (cull bottom HANDSHAKE_CULL_RATIO)
            let keep_count = ((results.len() as f64 * (1.0 - HANDSHAKE_CULL_RATIO)) as usize).max(1);
            let cutoff_latency = results.get(keep_count).map(|r| r.3).unwrap_or(u128::MAX);

            for (i, (cid, port, client_opt, _latency)) in results.into_iter().enumerate() {
                if i < keep_count {
                    if let Some(client) = client_opt {
                        circuit_candidates.push((cid, client, port));
                    }
                }
            }

            logger.log(&app, format!(
                "[+] Handshake filter: {} survived / {} culled (cutoff: {}ms)",
                circuit_candidates.len(),
                tournament_pool - circuit_candidates.len(),
                if cutoff_latency < u128::MAX { cutoff_latency.to_string() } else { "∞".to_string() }
            ));
        }

        for (circuit_id, circuit_client, daemon_port) in circuit_candidates {

            let task_tx = tx.clone();
            let task_app = app.clone();
            let task_url = url.clone();
            let task_path = temp_target.clone();
            let task_control = control.clone();
            let task_running = Arc::clone(&run_flag);
            let task_total = Arc::clone(&total_downloaded);
            let task_next_piece = Arc::clone(&next_piece);
            let task_piece_flags = Arc::clone(&piece_flags);
            let task_piece_owner = Arc::clone(&piece_owner);
            let task_circuit_killed = Arc::clone(&circuit_killed);
            let task_circuit_bytes = Arc::clone(&circuit_bytes);
            let task_server_fails = Arc::clone(&server_fail_count);
            let task_total_pieces = total_pieces;
            let task_content_length = content_length;
            let task_effective_circuits = scaled_circuits;
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

            tasks.spawn(async move {
                use futures::StreamExt;
                let mut circuit_client = circuit_client; // Mutable for recycling

                // === TOURNAMENT PROBE PHASE ===
                if !task_skip_tournament {
                    // Phase 1.2: 100KB micro-probe (instead of 1MB)
                    // TCP slow-start stabilizes at ~50KB through Tor, so 100KB
                    // captures 80% of the throughput signal in 10% of the time.
                    let probe_start = (circuit_id as u64 % task_total_pieces as u64) * task_piece_size;
                    let probe_end = (probe_start + PROBE_SIZE - 1).min(task_content_length.saturating_sub(1));

                    let probe_result = async {
                        let resp = tokio::time::timeout(
                            Duration::from_secs(30),
                            circuit_client
                                .get(&task_url)
                                .header(RANGE, format!("bytes={probe_start}-{probe_end}"))
                                .header("Connection", "close")
                                .send()
                        ).await;

                        match resp {
                            Ok(Ok(r)) if r.status() == StatusCode::PARTIAL_CONTENT || r.status() == StatusCode::OK => {
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
                let circuit_start = Instant::now();
                let mut pieces_completed = 0usize;
                let mut stalls = 0usize;
                let mut last_emit = Instant::now();
                let mut stealing = false;
                let mut recycle_count = 0usize;

                loop {
                    if !task_running.load(Ordering::Relaxed) {
                        break;
                    }
                    // Check if this circuit was killed → RECYCLE with fresh identity
                    if circuit_id < task_circuit_killed.len() && task_circuit_killed[circuit_id].load(Ordering::Relaxed) {
                        recycle_count += 1;
                        let new_socks_id = circuit_id + recycle_count * task_tournament_pool;
                        match range_download_client(task_is_onion, task_daemon_port, new_socks_id) {
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

                    // Grab next piece — either from queue or by stealing/hedging
                    let piece_idx = if !stealing {
                        // Normal mode: claim from atomic counter
                        let idx = task_next_piece.fetch_add(1, Ordering::Relaxed);
                        if idx >= task_total_pieces {
                            stealing = true;

                            // Phase 2.2: HEDGED REQUESTS for last 10%
                            // Check how many pieces remain — if <10%, skip stagger (aggressive hedge)
                            let done_count = task_piece_flags.iter()
                                .filter(|f| f.load(Ordering::Relaxed))
                                .count();
                            let remaining_pct = 100.0 * (1.0 - done_count as f64 / task_total_pieces as f64);

                            if remaining_pct > 10.0 {
                                // Normal steal entry: stagger to prevent thundering herd
                                let delay_ms = (circuit_id % 20) as u64 * 100; // 0-2s spread
                                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                            }
                            // When ≤10% remain: NO delay — all circuits race immediately (hedged)
                            continue; // Re-enter loop in steal mode
                        }
                        if task_piece_flags[idx].load(Ordering::Relaxed) {
                            continue; // Already done (from resume), skip
                        }
                        idx
                    } else {
                        // Steal/Hedge mode: scan from a random offset so circuits spread across pieces
                        let scan_start = circuit_id % task_total_pieces;
                        let found = (0..task_total_pieces)
                            .map(|i| (scan_start + i) % task_total_pieces)
                            .find(|&i| !task_piece_flags[i].load(Ordering::Relaxed));
                        match found {
                            Some(idx) => idx,
                            None => return TaskOutcome::Completed, // ALL pieces done!
                        }
                    };

                    // Register as owner of this piece
                    task_piece_owner[piece_idx].store(circuit_id, Ordering::Relaxed);

                    let piece_start = piece_idx as u64 * task_piece_size;
                    let piece_end = ((piece_idx as u64 + 1) * task_piece_size - 1).min(task_content_length.saturating_sub(1));
                    let mut current_offset = piece_start;
                    let piece_timer = Instant::now(); // UCB1: track piece download time

                    while current_offset <= piece_end && task_running.load(Ordering::Relaxed) {
                        // In steal mode, check if original owner finished this piece
                        if stealing && task_piece_flags[piece_idx].load(Ordering::Relaxed) {
                            break; // Original owner won the race — move to next
                        }

                        if let Some(reason) = task_control.interruption_reason() {
                            task_running.store(false, Ordering::Relaxed);
                            return TaskOutcome::Interrupted(reason);
                        }

                        let response_future = circuit_client
                            .get(&task_url)
                            .header(RANGE, format!("bytes={current_offset}-{piece_end}"))
                            .send();

                        let response = match tokio::time::timeout(Duration::from_secs(45), response_future).await {
                            Ok(Ok(resp)) => {
                                // Reset global fail counter on success
                                task_server_fails.store(0, Ordering::Relaxed);
                                task_aimd.on_success(); // Phase 4.4
                                resp
                            }
                            Ok(Err(_err)) => {
                                stalls += 1;
                                task_aimd.on_reject(); // Phase 4.4
                                let fails = task_server_fails.fetch_add(1, Ordering::Relaxed);
                                if stalls > MAX_STALL_RETRIES {
                                    let _ = task_app.emit("log", format!("[↻] Supervisor self-healing: Circuit {} rejected on piece {}. Rebuilding identity...", circuit_id, piece_idx));
                                    circuit_client = range_download_client(task_is_onion, task_daemon_port, circuit_id + 10000 + stalls).unwrap_or(circuit_client.clone());
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
                                    circuit_client = range_download_client(task_is_onion, task_daemon_port, circuit_id + 10000 + stalls).unwrap_or(circuit_client.clone());
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

                        if response.status() != StatusCode::PARTIAL_CONTENT
                            && response.status() != StatusCode::OK
                        {
                            stalls += 1;
                            task_server_fails.fetch_add(1, Ordering::Relaxed);
                            task_aimd.on_reject(); // Phase 4.4: bad status = server pushback
                            if stalls > MAX_STALL_RETRIES {
                                let _ = task_app.emit("log", format!("[↻] Supervisor self-healing: Circuit {} bad status on piece {}. Rebuilding identity...", circuit_id, piece_idx));
                                circuit_client = range_download_client(task_is_onion, task_daemon_port, circuit_id + 10000 + stalls).unwrap_or(circuit_client.clone());
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
                                    if task_tx
                                        .send(WriteMsg {
                                            filepath: task_path.clone(),
                                            offset: current_offset,
                                            data: chunk,
                                            close_file: false,
                                            chunk_id: piece_idx,
                                        })
                                        .await
                                        .is_err()
                                    {
                                        return TaskOutcome::Failed(
                                            "writer channel closed unexpectedly".to_string(),
                                        );
                                    }

                                    current_offset = current_offset.saturating_add(len);
                                    task_total.fetch_add(len, Ordering::Relaxed);
                                    // Track per-circuit bytes for health monitor
                                    if circuit_id < task_circuit_bytes.len() {
                                        task_circuit_bytes[circuit_id].fetch_add(len, Ordering::Relaxed);
                                    }

                                    if current_offset > piece_end {
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
                                    let _ = task_app.emit(
                                        "log",
                                        format!("[*] Circuit {} stream dropped on piece {}. Re-establishing...", circuit_id, piece_idx),
                                    );
                                    drop(stream);
                                    tokio::time::sleep(Duration::from_millis(500)).await;
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
                                circuit_client = range_download_client(task_is_onion, task_daemon_port, circuit_id + 10000 + stalls).unwrap_or(circuit_client.clone());
                                stalls = 0;
                                continue;
                            }
                            tokio::time::sleep(backoff_duration(stalls)).await;
                        }
                    }

                    // Piece completed — but only mark if we actually finished it (not stolen from under us)
                    if current_offset > piece_end && !task_piece_flags[piece_idx].load(Ordering::Relaxed) {
                        task_piece_flags[piece_idx].store(true, Ordering::Relaxed);
                        pieces_completed += 1;

                        // Phase 3: Record piece stats in UCB1 scorer
                        let piece_bytes = piece_end.saturating_sub(piece_start) + 1;
                        let piece_ms = piece_timer.elapsed().as_millis() as u64;
                        task_scorer.record_piece(circuit_id, piece_bytes, piece_ms);

                        // Phase 4.2: Predictive pre-warming — kill degrading circuits early
                        if task_scorer.is_degrading(circuit_id) && circuit_id < task_circuit_killed.len() {
                            task_circuit_killed[circuit_id].store(true, Ordering::Relaxed);
                            let _ = task_app.emit("log", format!(
                                "[⚡] Phase 4.2: Circuit {} degrading (latency 2.5× baseline) → pre-emptive recycle",
                                circuit_id
                            ));
                        }

                        if task_tx
                            .send(WriteMsg {
                                filepath: task_path.clone(),
                                offset: 0,
                                data: bytes::Bytes::new(),
                                close_file: true,
                                chunk_id: piece_idx,
                            })
                            .await
                            .is_err()
                        {
                            return TaskOutcome::Failed(
                                "writer channel closed unexpectedly".to_string(),
                            );
                        }

                        if stealing {
                            // Kill the original slow circuit
                            let original_owner = task_piece_owner[piece_idx].load(Ordering::Relaxed);
                            if original_owner != usize::MAX && original_owner != circuit_id && original_owner < task_circuit_killed.len() {
                                task_circuit_killed[original_owner].store(true, Ordering::Relaxed);
                                let _ = task_app.emit(
                                    "log",
                                    format!("[+] Circuit {} STOLE piece {} → killed slow circuit {}", circuit_id, piece_idx, original_owner),
                                );
                            } else {
                                let _ = task_app.emit(
                                    "log",
                                    format!("[+] Circuit {} STOLE piece {}", circuit_id, piece_idx),
                                );
                            }
                        }
                    }

                    // Emit progress
                    if last_emit.elapsed() >= Duration::from_millis(250) {
                        let elapsed = circuit_start.elapsed().as_secs_f64();
                        let speed = if elapsed > 0.0 {
                            ((pieces_completed as f64 * task_piece_size as f64) / elapsed) / 1048576.0
                        } else {
                            0.0
                        };

                        let _ = task_app.emit(
                            "progress",
                            ProgressEvent {
                                id: circuit_id,
                                downloaded: pieces_completed as u64 * task_piece_size,
                                total: task_content_length / task_effective_circuits as u64,
                                main_speed_mbps: speed,
                                status: if stealing { "Stealing".to_string() } else { "Active".to_string() },
                            },
                        );
                        last_emit = Instant::now();
                    }
                }

                TaskOutcome::Completed
            });
        }
    } else {
        let stream_client = stream_download_client(is_onion, active_ports.first().copied().unwrap_or(9051))?;
        let task_tx = tx.clone();
        let task_app = app.clone();
        let task_url = url.clone();
        let task_path = temp_target.clone();
        let task_control = control.clone();
        let task_running = Arc::clone(&run_flag);
        let task_total = Arc::clone(&total_downloaded);
        let total_hint = probe.content_length;

        tasks.spawn(async move {
            use futures::StreamExt;

            let mut current_offset = 0u64;
            let circuit_start = Instant::now();
            let mut retries = 0usize;

            while task_running.load(Ordering::Relaxed) {
                if let Some(reason) = task_control.interruption_reason() {
                    task_running.store(false, Ordering::Relaxed);
                    return TaskOutcome::Interrupted(reason);
                }

                let response_future = stream_client
                    .get(&task_url)
                    .send();

                let response = match tokio::time::timeout(Duration::from_secs(45), response_future).await {
                    Ok(Ok(resp)) => resp,
                    Ok(Err(err)) => {
                        retries += 1;
                        if retries > MAX_STALL_RETRIES {
                            return TaskOutcome::Failed(format!("stream request failed repeatedly: {err}"));
                        }
                        tokio::time::sleep(backoff_duration(retries)).await;
                        continue;
                    }
                    Err(_) => {
                        retries += 1;
                        if retries > MAX_STALL_RETRIES {
                            return TaskOutcome::Failed("stream request timeout (header wait)".to_string());
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
                let mut last_emit = Instant::now();

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
                            if task_tx
                                .send(WriteMsg {
                                    filepath: task_path.clone(),
                                    offset: current_offset,
                                    data: chunk,
                                    close_file: false,
                                    chunk_id: 0,
                                })
                                .await
                                .is_err()
                            {
                                return TaskOutcome::Failed(
                                    "writer channel closed unexpectedly".to_string(),
                                );
                            }

                            current_offset = current_offset.saturating_add(len);
                            task_total.fetch_add(len, Ordering::Relaxed);

                            let elapsed = circuit_start.elapsed().as_secs_f64();
                            let speed = if elapsed > 0.0 {
                                (current_offset as f64 / elapsed) / 1048576.0
                            } else {
                                0.0
                            };

                            if last_emit.elapsed() >= Duration::from_millis(150) {
                                let _ = task_app.emit(
                                    "progress",
                                    ProgressEvent {
                                        id: 0,
                                        downloaded: current_offset,
                                        total: total_hint.max(current_offset),
                                        main_speed_mbps: speed,
                                        status: "Active".to_string(),
                                    },
                                );
                                last_emit = Instant::now();
                            }
                        }
                        Ok(Some(Err(err))) => {
                            let _ = task_app.emit("log", format!("[*] Stream transient error: {err}. Re-establishing..."));
                            drop(stream);
                            tokio::time::sleep(Duration::from_millis(500)).await;
                            break;
                        }
                        Ok(None) => {
                            if current_offset >= total_hint && total_hint > 0 {
                                if task_tx
                                    .send(WriteMsg {
                                        filepath: task_path.clone(),
                                        offset: 0,
                                        data: bytes::Bytes::new(),
                                        close_file: true,
                                        chunk_id: 0,
                                    })
                                    .await
                                    .is_err()
                                {
                                    return TaskOutcome::Failed(
                                        "writer channel closed unexpectedly".to_string(),
                                    );
                                }

                                let elapsed = circuit_start.elapsed().as_secs_f64();
                                let speed = if elapsed > 0.0 {
                                    (current_offset as f64 / elapsed) / 1048576.0
                                } else {
                                    0.0
                                };

                                let _ = task_app.emit(
                                    "progress",
                                    ProgressEvent {
                                        id: 0,
                                        downloaded: current_offset,
                                        total: total_hint.max(current_offset),
                                        main_speed_mbps: speed,
                                        status: "Done".to_string(),
                                    },
                                );

                                return TaskOutcome::Completed;
                            } else {
                                let _ = task_app.emit("log", "[*] Stream dropped prematurely. Reconnecting...".to_string());
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

    drop(tx);

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
                logger.log(&app, "[+] All data received — aborting remaining circuits".to_string());
                tasks.abort_all();
                break;
            }
        }
    }

    run_flag.store(false, Ordering::Relaxed);
    let _ = speed_handle.await;

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
            format!(
                "[*] Download {} for {}",
                reason.to_lowercase(),
                output_target
            ),
        );

        let _ = app.emit(
            "download_interrupted",
            DownloadInterruptedEvent {
                url,
                path: output_target,
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
        logger.log(&app, format!(
            "[!] Download failed (got {} / {} bytes): {}", downloaded, probe.content_length, err
        ));
            return Err(anyhow!(err));
        }
        // All data received despite task errors — proceed to SHA verification
        logger.log(&app, format!(
            "[*] Ignoring {} task errors — all data received successfully", err
        ));
    }

    let download_elapsed = start_time.elapsed().as_secs_f64();

    logger.log(&app, "[+] Download complete. Verifying SHA256...".to_string());
    let _ = app.emit(
        "download_status",
        serde_json::json!({
            "phase": "sha256_started",
            "message": "Download complete — SHA256 verification in progress...",
            "download_time_secs": download_elapsed,
        }),
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
                Err(_) => {} // Timeout, emit current progress
            }

            // Drain any buffered updates to get latest
            while let Ok(bytes) = sha_rx.try_recv() {
                hashed_bytes = bytes;
            }

            if content_length > 0 && hashed_bytes > 0 {
                let pct = (hashed_bytes as f64 / content_length as f64 * 100.0).min(100.0);
                let elapsed = sha_start.elapsed().as_secs_f64();
                let speed = if elapsed > 0.0 { hashed_bytes as f64 / elapsed } else { 0.0 };
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
                        pct, hashed_mb / 1024.0, total_mb / 1024.0, eta_str
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
    logger.log(&app, format!("[+] SHA256 verified in {:.1}s: {}", sha_elapsed, hash));
    // Rename from .ariaforge temp file to final name
    if let Err(e) = fs::rename(&temp_target, &output_target) {
        logger.log(&app, format!("[!] Rename failed: {} — file is at {}", e, temp_target));
    } else {
        logger.log(&app, format!("[+] Renamed to final: {}", output_target));
    }

    let _ = app.emit(
        "complete",
        DownloadCompleteEvent {
            url,
            path: output_target.clone(),
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
    logger.log(&app, format!("[✓] Total time: {} | File: {}", time_str, output_target));

    let _ = fs::remove_file(state_file_path);
    Ok(())
}
