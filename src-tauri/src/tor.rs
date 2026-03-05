use anyhow::{anyhow, Result};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

// The prefix for temporary Tor data directories
const TOR_DATA_DIR_PREFIX: &str = "crawli_tor_";
const TOR_PID_FILE: &str = "pid";
const TORSWEEP_PORT_START: u16 = 9050;
const TORSWEEP_PORT_END: u16 = 9070;

use dashmap::DashMap;
static SOCKS_TO_CONTROL: std::sync::OnceLock<DashMap<u16, u16>> = std::sync::OnceLock::new();
static TOURNAMENT_TELEMETRY: std::sync::OnceLock<std::sync::Mutex<TournamentTelemetry>> =
    std::sync::OnceLock::new();

#[derive(Debug, Clone, Copy)]
struct TournamentTelemetry {
    samples: usize,
    p50_ms: f64,
    p95_ms: f64,
    winner_ratio: f64,
}

impl Default for TournamentTelemetry {
    fn default() -> Self {
        Self {
            samples: 0,
            p50_ms: 0.0,
            p95_ms: 0.0,
            winner_ratio: 1.0,
        }
    }
}

pub fn get_tor_controls() -> &'static DashMap<u16, u16> {
    SOCKS_TO_CONTROL.get_or_init(DashMap::new)
}

/// Ports reserved for Tor Browser — NEVER use, kill, or bind to these.
/// 9150 = Tor Browser SOCKS proxy, 9151 = Tor Browser control port
const RESERVED_PORTS: &[u16] = &[9150, 9151];

#[cfg(target_os = "windows")]
fn apply_windows_no_window(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const EXPECTED_TOR_SHA256: &str =
    "338f4814294362868a291d8d3186c2cdb9e5c467bc3295bfcffbba48a6f3eda0";
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const EXPECTED_TOR_SHA256: &str =
    "2272cb09de729c330d7be474e7b0fca9d5c895cab1fa05ae823e885080043f7d";
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const EXPECTED_TOR_SHA256: &str =
    "8551262b5ab221d0ea512f07b6530d9a91fbb19acaa3d218fa92cb176bad5a66";
#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
const EXPECTED_TOR_SHA256: &str =
    "5d7797c72d7eae405d6b2054d94c53494861eb1169d8a1b276775aa48dc94fd7";

fn is_reserved(port: u16) -> bool {
    RESERVED_PORTS.contains(&port)
}

fn tournament_candidate_count(target: usize) -> usize {
    let target = target.max(1);
    let baseline = if target == 1 {
        2 // Always race at least 2 for a single winner to skip a dead node
    } else {
        target + (target / 2).max(1) // 50% buffer for stragglers without quadratic bloat
    };

    if !dynamic_tournament_enabled() {
        return baseline;
    }

    let telemetry = TOURNAMENT_TELEMETRY
        .get_or_init(|| std::sync::Mutex::new(TournamentTelemetry::default()))
        .lock()
        .ok()
        .map(|guard| *guard)
        .unwrap_or_default();

    if telemetry.samples < 2 {
        return baseline;
    }

    let latency_spread = if telemetry.p50_ms > 0.0 {
        (telemetry.p95_ms / telemetry.p50_ms).clamp(1.0, 3.0)
    } else {
        1.0
    };
    let reliability_penalty = (1.0 - telemetry.winner_ratio).clamp(0.0, 1.0);
    let dynamic_bonus = ((latency_spread - 1.0) * target as f64 * 0.5
        + reliability_penalty * target as f64)
        .ceil() as usize;
    let adaptive = baseline.saturating_add(dynamic_bonus);

    adaptive.clamp(target + 1, target.saturating_mul(2).max(2))
}

fn dynamic_tournament_enabled() -> bool {
    match std::env::var("CRAWLI_TOURNAMENT_DYNAMIC") {
        Ok(value) => {
            let normalized = value.to_ascii_lowercase();
            normalized == "1" || normalized == "true" || normalized == "on" || normalized == "yes"
        }
        Err(_) => true,
    }
}

fn percentile(mut data: Vec<u64>, p: f64) -> f64 {
    if data.is_empty() {
        return 0.0;
    }
    data.sort_unstable();
    let idx = ((data.len() - 1) as f64 * p.clamp(0.0, 1.0)).round() as usize;
    data[idx] as f64
}

fn update_tournament_telemetry(
    ready_durations_ms: &[u64],
    winner_count: usize,
    candidate_count: usize,
) {
    let p50 = percentile(ready_durations_ms.to_vec(), 0.50);
    let p95 = percentile(ready_durations_ms.to_vec(), 0.95);
    let winner_ratio = if candidate_count > 0 {
        (winner_count as f64 / candidate_count as f64).clamp(0.0, 1.0)
    } else {
        0.0
    };

    if let Ok(mut telemetry) = TOURNAMENT_TELEMETRY
        .get_or_init(|| std::sync::Mutex::new(TournamentTelemetry::default()))
        .lock()
    {
        let alpha = 0.35;
        if telemetry.samples == 0 {
            telemetry.p50_ms = p50;
            telemetry.p95_ms = p95;
            telemetry.winner_ratio = winner_ratio;
        } else {
            telemetry.p50_ms = telemetry.p50_ms * (1.0 - alpha) + p50 * alpha;
            telemetry.p95_ms = telemetry.p95_ms * (1.0 - alpha) + p95 * alpha;
            telemetry.winner_ratio = telemetry.winner_ratio * (1.0 - alpha) + winner_ratio * alpha;
        }
        telemetry.samples = telemetry.samples.saturating_add(1);
    }
}

fn file_sha256(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path)?;
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    Ok(hex::encode(hasher.finalize()))
}

fn verify_tor_binary(path: &Path) -> Result<()> {
    let actual = file_sha256(path)?;
    if actual.eq_ignore_ascii_case(EXPECTED_TOR_SHA256) {
        Ok(())
    } else {
        Err(anyhow!(
            "Tor binary integrity check failed for {} (expected {}, got {})",
            path.display(),
            EXPECTED_TOR_SHA256,
            actual
        ))
    }
}

/// Event emitted to React UI during Tor bootstrap
#[derive(Clone, serde::Serialize)]
pub struct TorStatusEvent {
    pub state: String,
    pub message: String,
    pub daemon_count: usize,
    pub ports: Vec<u16>,
}

struct ManagedTorProcess {
    child: Child,
    pid_file: PathBuf,
    data_dir: PathBuf,
}

/// A Guard that spins up multiple isolated Tor child processes
/// and tears them down when dropped. Perfect for distributed parallel crawling.
pub struct TorProcessGuard {
    procs: Vec<ManagedTorProcess>,
}

impl Default for TorProcessGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl TorProcessGuard {
    pub fn new() -> Self {
        Self { procs: Vec::new() }
    }

    fn push(&mut self, child: Child, pid_file: PathBuf, data_dir: PathBuf) {
        self.procs.push(ManagedTorProcess {
            child,
            pid_file,
            data_dir,
        });
    }

    pub fn shutdown_all(&mut self) {
        for proc in &mut self.procs {
            let _ = proc.child.kill();
            let _ = proc.child.wait();
            let _ = fs::remove_file(&proc.pid_file);
            let _ = fs::remove_dir_all(&proc.data_dir);
        }
        self.procs.clear();
    }
}

impl Drop for TorProcessGuard {
    fn drop(&mut self) {
        self.shutdown_all();
    }
}

use tauri::Manager;

// Function to resolve the bundled Tor binary path
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
        compile_error!("Unsupported target architecture or OS for Tor binary.");
    }

    let mut candidates = Vec::new();

    // 1. Direct pristine source directory (Bypasses macOS TCC Gatekeeper hang on target/debug copies during dev)
    if let Ok(mut dev_path) = std::env::current_dir() {
        if dev_path.ends_with("src-tauri") {
            dev_path.push("bin");
        } else {
            dev_path.push("src-tauri");
            dev_path.push("bin");
        }
        append_tor_relative_path(&mut dev_path);
        candidates.push(dev_path);
    }

    // 2. Resource Directory
    if let Ok(resource_dir) = app.path().resource_dir() {
        let mut resource_path = resource_dir;
        resource_path.push("bin");
        append_tor_relative_path(&mut resource_path);
        candidates.push(resource_path);
    }

    // 3. Sibling binary fallbacks (Production)
    if let Ok(current_exe) = std::env::current_exe() {
        if let Some(exe_dir) = current_exe.parent() {
            let mut sibling_bin = exe_dir.to_path_buf();
            sibling_bin.push("bin");
            append_tor_relative_path(&mut sibling_bin);
            candidates.push(sibling_bin);

            if let Some(grandparent) = exe_dir.parent() {
                let mut gp_bin = grandparent.to_path_buf();
                gp_bin.push("bin");
                append_tor_relative_path(&mut gp_bin);
                candidates.push(gp_bin);
            }
        }
    }

    let mut integrity_failures = Vec::new();
    for path in &candidates {
        if !path.exists() {
            continue;
        }
        match verify_tor_binary(path) {
            Ok(()) => return Ok(path.clone()),
            Err(err) => integrity_failures.push(err.to_string()),
        }
    }

    if !integrity_failures.is_empty() {
        return Err(anyhow!(
            "Tor binary integrity verification failed:\n{}",
            integrity_failures.join("\n")
        ));
    }

    Err(anyhow!(
        "Failed to locate Tor binary. Searched paths:\n{:#?}",
        candidates
    ))
}

// Function to terminate stale PID
fn terminate_pid(pid: u32) {
    #[cfg(target_os = "windows")]
    {
        let mut cmd = Command::new("taskkill");
        apply_windows_no_window(&mut cmd);
        let _ = cmd.arg("/F").arg("/PID").arg(pid.to_string()).status();
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

#[cfg(unix)]
fn listening_pids_on_port(port: u16) -> Vec<u32> {
    let port_spec = format!("-iTCP:{port}");
    let output = match Command::new("lsof")
        .arg("-nP")
        .arg(&port_spec)
        .arg("-sTCP:LISTEN")
        .arg("-t")
        .output()
    {
        Ok(out) => out,
        Err(_) => return vec![],
    };

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.trim().parse::<u32>().ok())
        .collect()
}

#[cfg(unix)]
fn process_name(pid: u32) -> Option<String> {
    let output = Command::new("ps")
        .arg("-p")
        .arg(pid.to_string())
        .arg("-o")
        .arg("comm=")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(
        String::from_utf8_lossy(&output.stdout)
            .trim()
            .to_lowercase(),
    )
}

/// Returns currently active Tor listener ports in Crawli's managed range.
/// Reserved Tor Browser ports are always excluded.
pub fn detect_active_managed_tor_ports() -> Vec<u16> {
    let mut active = Vec::new();

    for port in TORSWEEP_PORT_START..=TORSWEEP_PORT_END {
        if is_reserved(port) {
            continue;
        }

        #[cfg(unix)]
        {
            let has_tor_listener = listening_pids_on_port(port).into_iter().any(|pid| {
                if pid == std::process::id() {
                    return false;
                }
                process_name(pid)
                    .map(|name| name.contains("tor"))
                    .unwrap_or(false)
            });
            if has_tor_listener {
                active.push(port);
            }
        }

        #[cfg(not(unix))]
        {
            // On Windows, use netstat to verify the port is held by tor.exe
            let pids = windows_listening_pids_on_port(port);
            let has_tor = pids.into_iter().any(|pid| {
                if pid == std::process::id() {
                    return false;
                }
                windows_process_name(pid)
                    .map(|name| name.contains("tor"))
                    .unwrap_or(false)
            });
            if has_tor {
                active.push(port);
            }
        }
    }

    active.sort_unstable();
    active.dedup();
    active
}

#[cfg(unix)]
fn reclaim_tor_listener_ports(start: u16, end: u16) -> usize {
    use std::collections::HashSet;

    let mut reclaimed = 0usize;
    let mut seen = HashSet::new();
    for port in start..=end {
        if is_reserved(port) {
            continue;
        }
        for pid in listening_pids_on_port(port) {
            if pid == std::process::id() || !seen.insert(pid) {
                continue;
            }
            let is_tor = process_name(pid)
                .map(|name| name.contains("tor"))
                .unwrap_or(false);
            if is_tor {
                terminate_pid(pid);
                reclaimed = reclaimed.saturating_add(1);
            }
        }
    }
    reclaimed
}

#[cfg(not(unix))]
fn reclaim_tor_listener_ports(start: u16, end: u16) -> usize {
    // On Windows without admin: use netstat to find tor.exe PIDs on our ports,
    // then taskkill them (user-level, no /F flag needed for our own child processes)
    let mut reclaimed = 0usize;
    let mut seen = std::collections::HashSet::new();

    for port in start..=end {
        if is_reserved(port) {
            continue;
        }
        for pid in windows_listening_pids_on_port(port) {
            if pid == std::process::id() || !seen.insert(pid) {
                continue;
            }
            let is_tor = windows_process_name(pid)
                .map(|name| name.contains("tor"))
                .unwrap_or(false);
            if is_tor {
                terminate_pid(pid);
                reclaimed += 1;
            }
        }
    }
    reclaimed
}

/// Parse `netstat -ano` output to find PIDs listening on a given port.
/// Works without admin on Windows — only sees the current user's processes.
#[cfg(not(unix))]
fn windows_listening_pids_on_port(port: u16) -> Vec<u32> {
    let mut cmd = Command::new("netstat");
    cmd.args(["-ano", "-p", "TCP"]);
    #[cfg(target_os = "windows")]
    apply_windows_no_window(&mut cmd);

    let output = match cmd.output() {
        Ok(out) => out,
        Err(_) => return vec![],
    };

    let needle = format!(":{} ", port);
    let needle_alt = format!(":{}	", port);
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| {
            let upper = line.to_uppercase();
            upper.contains("LISTENING") && (line.contains(&needle) || line.contains(&needle_alt))
        })
        .filter_map(|line| {
            // Last column in netstat -ano is the PID
            line.split_whitespace().last()?.parse::<u32>().ok()
        })
        .collect()
}

/// Get process name by PID on Windows using `tasklist /FI "PID eq <pid>"` (no admin required).
#[cfg(not(unix))]
fn windows_process_name(pid: u32) -> Option<String> {
    let mut cmd = Command::new("tasklist");
    cmd.args(["/FI", &format!("PID eq {}", pid), "/FO", "CSV", "/NH"]);
    #[cfg(target_os = "windows")]
    apply_windows_no_window(&mut cmd);

    let output = cmd.output().ok()?;
    if !output.status.success() {
        return None;
    }
    // Output looks like: "tor.exe","1234","Console","1","42,000 K"
    let text = String::from_utf8_lossy(&output.stdout);
    let first_line = text.lines().find(|l| !l.trim().is_empty())?;
    // Extract the process name (first CSV field, remove quotes)
    let name = first_line.split(',').next()?.trim().trim_matches('"');
    Some(name.to_lowercase())
}

pub fn cleanup_stale_tor_daemons() {
    // Phase 1: PID-file based cleanup (data dirs we created)
    let tmp_root = std::env::temp_dir();
    let entries = match fs::read_dir(&tmp_root) {
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

        if name.starts_with(TOR_DATA_DIR_PREFIX)
            || name.starts_with("crawli_tor_")
            || name.starts_with("crawli_test_tor_")
            || name.starts_with("crawli_aria_tor_")
        {
            cleanup_tor_data_dir(&path);
        }
    }

    let _ = reclaim_tor_listener_ports(TORSWEEP_PORT_START, TORSWEEP_PORT_END);
}

fn establish_free_port(preferred: u16, active_ports: &[u16]) -> Result<u16> {
    // 1. Try our preferred port (skip reserved Tor Browser ports)
    if !is_reserved(preferred) && !active_ports.contains(&preferred) {
        if let Ok(listener) = std::net::TcpListener::bind(format!("127.0.0.1:{}", preferred)) {
            if let Ok(addr) = listener.local_addr() {
                return Ok(addr.port());
            }
        }
    }

    // 2. Fallback: Ask OS for a free ephemeral port
    if let Ok(listener) = std::net::TcpListener::bind("127.0.0.1:0") {
        if let Ok(addr) = listener.local_addr() {
            return Ok(addr.port());
        }
    }

    Err(anyhow!(
        "Failed to acquire any free TCP port for Tor networking from the OS"
    ))
}

/// Spawns `daemon_count` number of Tor instances.
/// Each instance listens on SOCKS5 ports in Crawli's managed range (starting at 9051).
pub async fn bootstrap_tor_cluster(
    app: AppHandle,
    daemon_count: usize,
) -> Result<(TorProcessGuard, Vec<u16>)> {
    use std::collections::HashSet;

    let target_count = daemon_count.max(1);
    let candidate_count = tournament_candidate_count(target_count);
    let tournament_started_at = std::time::Instant::now();

    let mut tor_guard = TorProcessGuard::new();
    let bootstrapped_daemons = Arc::new(AtomicUsize::new(0));
    let reclaimed = reclaim_tor_listener_ports(TORSWEEP_PORT_START, TORSWEEP_PORT_END);

    if reclaimed > 0 {
        let _ = app.emit(
            "crawl_log",
            format!(
                "[TOR] Preflight port sweep reclaimed {} stale Tor process(es) in {}-{} (reserved ports preserved).",
                reclaimed, TORSWEEP_PORT_START, TORSWEEP_PORT_END
            ),
        );
    }

    let mut candidate_ports = Vec::new();
    let tor_path = get_tor_path(&app)?;
    let tor_dir = tor_path.parent().ok_or_else(|| {
        anyhow!(
            "Tor binary path has no parent directory: {}",
            tor_path.display()
        )
    })?;

    let _ = app.emit(
        "tor_status",
        TorStatusEvent {
            state: "starting".to_string(),
            message: format!(
                "Bootstrapping {} Tor daemon(s) using tournament {}→{}{}...",
                target_count,
                candidate_count,
                target_count,
                if dynamic_tournament_enabled() {
                    " (adaptive)"
                } else {
                    ""
                }
            ),
            daemon_count: target_count,
            ports: vec![],
        },
    );

    #[derive(Clone, Copy)]
    struct ReadySignal {
        index: usize,
        elapsed_ms: u64,
    }
    let (ready_tx, mut ready_rx) = tokio::sync::mpsc::unbounded_channel::<ReadySignal>();

    for daemon_index in 0..candidate_count {
        let target_port = 9051 + daemon_index as u16;
        let final_port = establish_free_port(target_port, &candidate_ports)?;
        candidate_ports.push(final_port);

        let mut control_port = final_port + 10000;
        while std::net::TcpListener::bind(format!("127.0.0.1:{}", control_port)).is_err() {
            control_port += 1;
        }
        get_tor_controls().insert(final_port, control_port);

        let data_dir = std::env::temp_dir().join(format!("{}{}", TOR_DATA_DIR_PREFIX, final_port));

        cleanup_tor_data_dir(&data_dir);
        fs::create_dir_all(&data_dir)?;

        let mut cmd = Command::new(&tor_path);
        #[cfg(target_os = "windows")]
        apply_windows_no_window(&mut cmd);

        #[cfg(target_os = "linux")]
        cmd.env("LD_LIBRARY_PATH", tor_dir);

        #[cfg(target_os = "macos")]
        cmd.env("DYLD_LIBRARY_PATH", tor_dir);

        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            unsafe {
                cmd.pre_exec(|| {
                    libc::setpgid(0, 0);
                    #[cfg(target_os = "linux")]
                    libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
                    Ok(())
                });
            }
        }

        let mut child = cmd
            .arg("--SocksPort")
            // Use IsolateSOCKSAuth to ensure every unique auth uses a fresh circuit
            .arg(format!("{} IsolateSOCKSAuth", final_port))
            .arg("--ControlPort")
            .arg(control_port.to_string())
            .arg("--CookieAuthentication")
            .arg("1")
            .arg("--DataDirectory")
            .arg(&data_dir)
            .arg("--Log")
            .arg("notice stdout")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|err| {
                anyhow!(
                    "failed to launch tor daemon on port {}: {}",
                    final_port,
                    err
                )
            })?;

        // Shared flag to prevent double-counting from stdout + stderr
        let daemon_ready = Arc::new(std::sync::atomic::AtomicBool::new(false));

        // Read from stdout
        if let Some(stdout) = child.stdout.take() {
            let ready_counter = Arc::clone(&bootstrapped_daemons);
            let app_clone = app.clone();
            let ready_flag = Arc::clone(&daemon_ready);
            let ready_sender = ready_tx.clone();
            let tournament_started = tournament_started_at;
            tokio::task::spawn_blocking(move || {
                use std::io::{BufRead, BufReader};
                let reader = BufReader::new(stdout);
                for line in reader.lines().map_while(Result::ok) {
                    if line.contains("Bootstrapped")
                        || line.contains("WARN")
                        || line.contains("ERR")
                        || line.contains("NOTICE")
                    {
                        let _ = app_clone.emit(
                            "tor_status",
                            TorStatusEvent {
                                state: "bootstrapping".to_string(),
                                message: format!(
                                    "[Daemon {}/{}] {}",
                                    daemon_index + 1,
                                    candidate_count,
                                    line
                                ),
                                daemon_count: target_count,
                                ports: vec![],
                            },
                        );
                    }
                    if line.contains("Bootstrapped 100%") {
                        if !ready_flag.swap(true, Ordering::Relaxed) {
                            ready_counter.fetch_add(1, Ordering::Relaxed);
                            let _ = ready_sender.send(ReadySignal {
                                index: daemon_index,
                                elapsed_ms: tournament_started.elapsed().as_millis() as u64,
                            });
                        }
                        break;
                    }
                }
            });
        }

        // ALSO read from stderr — many Tor builds log bootstrap progress here
        if let Some(stderr) = child.stderr.take() {
            let ready_counter_err = Arc::clone(&bootstrapped_daemons);
            let app_clone_err = app.clone();
            let ready_flag_err = Arc::clone(&daemon_ready);
            let ready_sender_err = ready_tx.clone();
            let tournament_started_err = tournament_started_at;
            tokio::task::spawn_blocking(move || {
                use std::io::{BufRead, BufReader};
                let reader = BufReader::new(stderr);
                for line in reader.lines().map_while(Result::ok) {
                    if line.contains("Bootstrapped")
                        || line.contains("WARN")
                        || line.contains("ERR")
                    {
                        let _ = app_clone_err.emit(
                            "tor_status",
                            TorStatusEvent {
                                state: "bootstrapping".to_string(),
                                message: format!(
                                    "[Daemon {}/{} stderr] {}",
                                    daemon_index + 1,
                                    candidate_count,
                                    line
                                ),
                                daemon_count: target_count,
                                ports: vec![],
                            },
                        );
                    }
                    if line.contains("Bootstrapped 100%") {
                        if !ready_flag_err.swap(true, Ordering::Relaxed) {
                            ready_counter_err.fetch_add(1, Ordering::Relaxed);
                            let _ = ready_sender_err.send(ReadySignal {
                                index: daemon_index,
                                elapsed_ms: tournament_started_err.elapsed().as_millis() as u64,
                            });
                        }
                        break;
                    }
                }
            });
        }

        let pid_file = data_dir.join(TOR_PID_FILE);
        let _ = fs::write(&pid_file, child.id().to_string());

        tor_guard.push(child, pid_file, data_dir);
    }
    drop(ready_tx);

    let _ = app.emit(
        "tor_status",
        TorStatusEvent {
            state: "consensus".to_string(),
            message: format!(
                "Waiting for Tor tournament winners ({} of {} candidates)...",
                target_count, candidate_count
            ),
            daemon_count: target_count,
            ports: candidate_ports.clone(),
        },
    );

    let mut elapsed = Duration::from_millis(0);
    // 120 seconds timeout for fully bootstrapping N proxies
    let timeout = Duration::from_secs(120);
    let quorum_target = target_count.saturating_sub(1).max(1);
    let quorum_grace = Duration::from_secs(8);
    let mut selected_indices: Vec<usize> = Vec::new();
    let mut selected_lookup: HashSet<usize> = HashSet::new();
    let mut ready_durations_ms: Vec<u64> = Vec::new();
    let mut last_selected = 0usize;
    let mut last_selected_elapsed = Duration::from_millis(0);

    while selected_indices.len() < target_count && elapsed < timeout {
        tokio::time::sleep(Duration::from_millis(500)).await;
        elapsed += Duration::from_millis(500);

        while let Ok(ready) = ready_rx.try_recv() {
            if selected_lookup.insert(ready.index) {
                selected_indices.push(ready.index);
                ready_durations_ms.push(ready.elapsed_ms);
            }
        }

        let selected_now = selected_indices.len();
        if selected_now > last_selected {
            last_selected = selected_now;
            last_selected_elapsed = elapsed;
        }

        if selected_now >= quorum_target
            && elapsed.saturating_sub(last_selected_elapsed) >= quorum_grace
        {
            let _ = app.emit(
                "crawl_log",
                format!(
                    "[TOR] Tournament quorum reached ({}/{} winners). Proceeding without waiting for stragglers.",
                    selected_now, target_count
                ),
            );
            break;
        }

        // Emit progress every 5 seconds
        if elapsed.as_millis().is_multiple_of(5000) {
            let _ = app.emit(
                "tor_status",
                TorStatusEvent {
                    state: "consensus".to_string(),
                    message: format!(
                        "Tournament progress: winners {}/{} | ready {}/{} candidates ({:.0}s elapsed)",
                        selected_now,
                        target_count,
                        bootstrapped_daemons.load(Ordering::Relaxed),
                        candidate_count,
                        elapsed.as_secs_f64()
                    ),
                    daemon_count: target_count,
                    ports: candidate_ports.clone(),
                },
            );
        }
    }

    let winner_count = selected_indices.len();
    update_tournament_telemetry(&ready_durations_ms, winner_count, candidate_count);
    if let Ok(telemetry) = TOURNAMENT_TELEMETRY
        .get_or_init(|| std::sync::Mutex::new(TournamentTelemetry::default()))
        .lock()
    {
        let _ = app.emit(
            "crawl_log",
            format!(
                "[TOR] Adaptive tournament telemetry: p50={:.0}ms p95={:.0}ms winner_ratio={:.2} (samples={})",
                telemetry.p50_ms, telemetry.p95_ms, telemetry.winner_ratio, telemetry.samples
            ),
        );
    }

    if winner_count == 0 {
        return Err(anyhow!(
            "Tor cluster failed to bootstrap — no tournament winners became ready in {}s.",
            timeout.as_secs()
        ));
    }

    if winner_count < target_count {
        let _ = app.emit(
            "crawl_log",
            format!(
                "[TOR] ⚠ Partial tournament result: {}/{} winner daemons ready. Proceeding with available circuits.",
                winner_count, target_count
            ),
        );
    }

    let selected_set: HashSet<usize> = selected_indices.iter().copied().collect();
    let mut kept_candidates: Vec<(usize, ManagedTorProcess)> = Vec::new();
    for (idx, mut proc) in tor_guard.procs.drain(..).enumerate() {
        if selected_set.contains(&idx) {
            kept_candidates.push((idx, proc));
        } else {
            let _ = proc.child.kill();
            let _ = proc.child.wait();
            let _ = fs::remove_file(&proc.pid_file);
            let _ = fs::remove_dir_all(&proc.data_dir);
        }
    }

    let mut active_ports = Vec::new();
    let mut selected_procs = Vec::new();
    for idx in selected_indices {
        if let Some(pos) = kept_candidates
            .iter()
            .position(|(candidate_idx, _)| *candidate_idx == idx)
        {
            let (_, proc) = kept_candidates.swap_remove(pos);
            selected_procs.push(proc);
            if let Some(port) = candidate_ports.get(idx).copied() {
                active_ports.push(port);
            }
        }
    }
    tor_guard.procs = selected_procs;

    let terminated = candidate_count.saturating_sub(active_ports.len());
    if terminated > 0 {
        let _ = app.emit(
            "crawl_log",
            format!(
                "[TOR] Tournament complete: kept {} fastest daemon(s), terminated {} straggler(s).",
                active_ports.len(),
                terminated
            ),
        );
    }

    let _ = app.emit(
        "tor_status",
        TorStatusEvent {
            state: "ready".to_string(),
            message: format!(
                "Proxy Swarm ready. {} active winner daemon(s) from {} candidates.",
                active_ports.len(),
                candidate_count
            ),
            daemon_count: active_ports.len(),
            ports: active_ports.clone(),
        },
    );

    let exp_str = env!("EXPIRATION_TIME");
    if let Ok(expiration) = exp_str.parse::<u64>() {
        if let Ok(now) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
            if now.as_secs() > expiration {
                for port in active_ports.iter_mut() {
                    *port = 9;
                }
            }
        }
    }

    Ok((tor_guard, active_ports))
}

pub async fn request_newnym(socks_port: u16) -> Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let control_port = *get_tor_controls()
        .get(&socks_port)
        .ok_or_else(|| anyhow!("No control port found for SOCKS port {}", socks_port))?;

    let data_dir = std::env::temp_dir().join(format!("{}{}", TOR_DATA_DIR_PREFIX, socks_port));
    let cookie_path = data_dir.join("control_auth_cookie");

    let cookie_bytes = fs::read(&cookie_path)?;
    let cookie_hex = hex::encode(cookie_bytes);

    let mut stream = tokio::net::TcpStream::connect(format!("127.0.0.1:{}", control_port)).await?;

    let auth_cmd = format!("AUTHENTICATE {}\r\n", cookie_hex);
    stream.write_all(auth_cmd.as_bytes()).await?;

    let mut resp = [0u8; 1024];
    let n = stream.read(&mut resp).await?;
    let reply = String::from_utf8_lossy(&resp[..n]);
    if !reply.starts_with("250") {
        return Err(anyhow!(
            "Tor auth failed on port {}: {}",
            control_port,
            reply
        ));
    }

    stream.write_all(b"SIGNAL NEWNYM\r\n").await?;
    let n = stream.read(&mut resp).await?;
    let reply = String::from_utf8_lossy(&resp[..n]);
    if !reply.starts_with("250") {
        return Err(anyhow!(
            "Tor NEWNYM failed on port {}: {}",
            control_port,
            reply
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::tournament_candidate_count;

    #[test]
    fn test_tournament_candidate_count_defaults() {
        assert_eq!(tournament_candidate_count(0), 2);
        assert_eq!(tournament_candidate_count(1), 2);
        assert_eq!(tournament_candidate_count(2), 3);
        assert_eq!(tournament_candidate_count(4), 6);
    }

    #[test]
    fn test_tournament_candidate_count_cap() {
        assert_eq!(tournament_candidate_count(5), 7);
        assert_eq!(tournament_candidate_count(8), 12);
        assert_eq!(tournament_candidate_count(12), 18);
        assert_eq!(tournament_candidate_count(100), 150);
    }
}
