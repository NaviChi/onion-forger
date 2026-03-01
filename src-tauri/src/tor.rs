use anyhow::{anyhow, Result};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use std::fs;

// The prefix for temporary Tor data directories
const TOR_DATA_DIR_PREFIX: &str = "crawli_tor_";
const TOR_PID_FILE: &str = "pid";

/// Ports reserved for Tor Browser — NEVER use, kill, or bind to these.
/// 9150 = Tor Browser SOCKS proxy, 9151 = Tor Browser control port
const RESERVED_PORTS: &[u16] = &[9150, 9151];

fn is_reserved(port: u16) -> bool {
    RESERVED_PORTS.contains(&port)
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
            all(target_os = "linux", target_arch = "x86_64"),
            all(target_os = "linux", target_arch = "aarch64")
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

    for path in &candidates {
        if path.exists() {
            return Ok(path.clone());
        }
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
        let _ = Command::new("taskkill").arg("/F").arg("/PID").arg(pid.to_string()).status();
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = Command::new("kill").arg("-TERM").arg(pid.to_string()).status();
        let _ = Command::new("kill").arg("-KILL").arg(pid.to_string()).status();
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
    // Phase 1: PID-file based cleanup (data dirs we created)
    let tmp_root = std::env::temp_dir();
    let entries = match fs::read_dir(&tmp_root) {
        Ok(entries) => entries,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() { continue; }

        let Some(name) = path.file_name().and_then(|value| value.to_str()) else { continue; };

        if name.starts_with(TOR_DATA_DIR_PREFIX) || name.starts_with("crawli_tor_") || name.starts_with("crawli_test_tor_") || name.starts_with("crawli_aria_tor_") {
            cleanup_tor_data_dir(&path);
        }
    }

    // Phase 2: Aggressive process kill — any tor with our IsolateSOCKSAuth marker
    #[cfg(not(target_os = "windows"))]
    {
        let _ = Command::new("pkill").args(["-9", "-f", "tor.*IsolateSOCKSAuth"]).status();
    }
    #[cfg(target_os = "windows")]
    {
        // On Windows, use taskkill with a filter
        let _ = Command::new("taskkill").args(["/F", "/IM", "tor.exe"]).status();
    }
}

// Slays any rogue processes clinging to a specific port
fn kill_port_process(port: u16) {
    if is_reserved(port) { return; } // Never kill Tor Browser
    #[cfg(not(target_os = "windows"))]
    {
        if let Ok(output) = Command::new("lsof").arg("-t").arg("-n").arg("-P").arg(format!("-i:{}", port)).output() {
            let pids = String::from_utf8_lossy(&output.stdout);
            for pid_str in pids.lines() {
                if let Ok(pid) = pid_str.trim().parse::<u32>() {
                    terminate_pid(pid);
                }
            }
        }
    }
    #[cfg(target_os = "windows")]
    {
        // Windows: use netstat to find PID bound to port, then taskkill
        if let Ok(output) = Command::new("netstat").args(["-ano", "-p", "TCP"]).output() {
            let text = String::from_utf8_lossy(&output.stdout);
            for line in text.lines() {
                if line.contains(&format!(":{} ", port)) || line.contains(&format!(":{}	", port)) {
                    if let Some(pid_str) = line.split_whitespace().last() {
                        if let Ok(pid) = pid_str.trim().parse::<u32>() {
                            terminate_pid(pid);
                        }
                    }
                }
            }
        }
    }
}

fn establish_free_port(preferred: u16, active_ports: &[u16]) -> Result<u16> {
    // 1. Try our preferred port (skip reserved Tor Browser ports)
    if !is_reserved(preferred) && !active_ports.contains(&preferred) {
        if std::net::TcpListener::bind(format!("127.0.0.1:{}", preferred)).is_ok() {
            return Ok(preferred);
        }
        
        // 2. Port is locked! Slay the Zombie and try again.
        kill_port_process(preferred);
        std::thread::sleep(Duration::from_millis(500));
        if std::net::TcpListener::bind(format!("127.0.0.1:{}", preferred)).is_ok() {
            return Ok(preferred);
        }
    }

    // 3. Fallback: Search cleanly in range 9051..=9068 (excluding reserved)
    for port in 9051..=9068 {
        if is_reserved(port) || active_ports.contains(&port) { continue; }
        if std::net::TcpListener::bind(format!("127.0.0.1:{}", port)).is_ok() {
            return Ok(port);
        }
    }

    // 4. Aggressive Fallback: Kill zombies across the entire range (excluding reserved)
    for port in 9051..=9068 {
        if is_reserved(port) || active_ports.contains(&port) { continue; }
        kill_port_process(port);
        std::thread::sleep(Duration::from_millis(200));
        if std::net::TcpListener::bind(format!("127.0.0.1:{}", port)).is_ok() {
            return Ok(port);
        }
    }

    Err(anyhow!("No available Tor ports in range 9051-9068"))
}

/// Spawns `daemon_count` number of Tor instances. 
/// Each instance listens on SOCKS5 port `9150 + index`.
pub async fn bootstrap_tor_cluster(app: AppHandle, daemon_count: usize) -> Result<(TorProcessGuard, Vec<u16>)> {
    let mut tor_guard = TorProcessGuard::new();
    let bootstrapped_daemons = Arc::new(AtomicUsize::new(0));

    let mut active_ports = Vec::new();

    let _ = app.emit("tor_status", TorStatusEvent {
        state: "starting".to_string(),
        message: format!("Bootstrapping {} Tor daemon(s)...", daemon_count),
        daemon_count,
        ports: vec![],
    });

    for daemon_index in 0..daemon_count {
        let target_port = 9051 + daemon_index as u16;
        let final_port = establish_free_port(target_port, &active_ports)?;
        active_ports.push(final_port);

        let data_dir = std::env::temp_dir().join(format!("{}{}", TOR_DATA_DIR_PREFIX, final_port));
        
        cleanup_tor_data_dir(&data_dir);
        fs::create_dir_all(&data_dir)?;

        let tor_path = get_tor_path(&app)?;
        let tor_dir = tor_path.parent().unwrap();
        
        let mut cmd = Command::new(&tor_path);

        #[cfg(target_os = "linux")]
        cmd.env("LD_LIBRARY_PATH", tor_dir);

        #[cfg(target_os = "macos")]
        cmd.env("DYLD_LIBRARY_PATH", tor_dir);

        let mut child = cmd
            .arg("--SocksPort")
            // Use IsolateSOCKSAuth to ensure every unique auth uses a fresh circuit
            .arg(format!("{} IsolateSOCKSAuth", final_port)) 
            .arg("--DataDirectory")
            .arg(&data_dir)
            .arg("--Log")
            .arg("notice stdout")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|err| anyhow!("failed to launch tor daemon on port {}: {}", final_port, err))?;

        // Shared flag to prevent double-counting from stdout + stderr
        let daemon_ready = Arc::new(std::sync::atomic::AtomicBool::new(false));

        // Read from stdout
        if let Some(stdout) = child.stdout.take() {
            let ready_counter = Arc::clone(&bootstrapped_daemons);
            let app_clone = app.clone();
            let ready_flag = Arc::clone(&daemon_ready);
            tokio::task::spawn_blocking(move || {
                use std::io::{BufRead, BufReader};
                let reader = BufReader::new(stdout);
                for line in reader.lines().map_while(Result::ok) {
                    if line.contains("Bootstrapped") || line.contains("WARN") || line.contains("ERR") || line.contains("NOTICE") {
                        let _ = app_clone.emit("tor_status", TorStatusEvent {
                            state: "bootstrapping".to_string(),
                            message: format!("[Daemon {}] {}", daemon_index, line),
                            daemon_count,
                            ports: vec![],
                        });
                    }
                    if line.contains("Bootstrapped 100%") {
                        if !ready_flag.swap(true, Ordering::Relaxed) {
                            ready_counter.fetch_add(1, Ordering::Relaxed);
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
            tokio::task::spawn_blocking(move || {
                use std::io::{BufRead, BufReader};
                let reader = BufReader::new(stderr);
                for line in reader.lines().map_while(Result::ok) {
                    if line.contains("Bootstrapped") || line.contains("WARN") || line.contains("ERR") {
                        let _ = app_clone_err.emit("tor_status", TorStatusEvent {
                            state: "bootstrapping".to_string(),
                            message: format!("[Daemon {} stderr] {}", daemon_index, line),
                            daemon_count,
                            ports: vec![],
                        });
                    }
                    if line.contains("Bootstrapped 100%") {
                        if !ready_flag_err.swap(true, Ordering::Relaxed) {
                            ready_counter_err.fetch_add(1, Ordering::Relaxed);
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

    let _ = app.emit("tor_status", TorStatusEvent {
        state: "consensus".to_string(),
        message: "Waiting for Tor consensus bootstrap...".to_string(),
        daemon_count,
        ports: active_ports.clone(),
    });

    let mut elapsed = Duration::from_millis(0);
    // 120 seconds timeout for fully bootstrapping N proxies
    let timeout = Duration::from_secs(120); 
    
    while bootstrapped_daemons.load(Ordering::Relaxed) < daemon_count && elapsed < timeout {
        tokio::time::sleep(Duration::from_millis(500)).await;
        elapsed += Duration::from_millis(500);
        
        // Emit progress every 5 seconds
        if elapsed.as_millis() % 5000 == 0 {
            let ready = bootstrapped_daemons.load(Ordering::Relaxed);
            let _ = app.emit("tor_status", TorStatusEvent {
                state: "consensus".to_string(),
                message: format!("Bootstrap progress: {}/{} daemons ready ({:.0}s elapsed)", ready, daemon_count, elapsed.as_secs_f64()),
                daemon_count,
                ports: active_ports.clone(),
            });
        }
    }

    let ready_count = bootstrapped_daemons.load(Ordering::Relaxed);
    
    if ready_count == 0 {
        return Err(anyhow!("Tor cluster failed to bootstrap — no daemons became ready in {}s.", timeout.as_secs()));
    }
    
    if ready_count < daemon_count {
        let _ = app.emit("crawl_log", format!("[TOR] ⚠ Partial bootstrap: {}/{} daemons ready. Proceeding with available circuits.", ready_count, daemon_count));
    }

    let _ = app.emit("tor_status", TorStatusEvent {
        state: "ready".to_string(),
        message: format!("Proxy Swarm ready. {}/{} daemons active.", ready_count, daemon_count),
        daemon_count: ready_count,
        ports: active_ports.clone(),
    });

    Ok((tor_guard, active_ports))
}
