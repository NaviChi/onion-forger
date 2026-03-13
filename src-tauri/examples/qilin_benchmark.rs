use anyhow::{Context, Result};
use crawli_lib::adapters::qilin::QilinAdapter;
use crawli_lib::adapters::{CrawlerAdapter, EntryType};
use crawli_lib::frontier::{CrawlOptions, CrawlerFrontier};
use serde::Serialize;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tauri::Manager;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;

const MAX_DEPTH: usize = 4;
const DIRS_PER_LEVEL: usize = 4;
const FILES_PER_DIR: usize = 12;
const CIRCUIT_MATRIX: &[usize] = &[12, 24, 36, 64];

#[derive(Clone, Copy)]
struct BenchmarkProfile {
    name: &'static str,
    base_delay_ms: u64,
    slow_every: Option<u64>,
    slow_extra_delay_ms: u64,
    throttle_every: Option<u64>,
}

const BENCHMARK_PROFILES: &[BenchmarkProfile] = &[
    BenchmarkProfile {
        name: "clean",
        base_delay_ms: 4,
        slow_every: None,
        slow_extra_delay_ms: 0,
        throttle_every: None,
    },
    BenchmarkProfile {
        name: "hostile",
        base_delay_ms: 12,
        slow_every: Some(5),
        slow_extra_delay_ms: 60,
        throttle_every: Some(7),
    },
];

#[derive(Default)]
struct ServerStats {
    requests: AtomicUsize,
    throttles: AtomicUsize,
    slow_responses: AtomicUsize,
}

struct ServerState {
    profile: BenchmarkProfile,
    request_counts: Mutex<HashMap<String, usize>>,
    stats: ServerStats,
}

impl ServerState {
    fn new(profile: BenchmarkProfile) -> Self {
        Self {
            profile,
            request_counts: Mutex::new(HashMap::new()),
            stats: ServerStats::default(),
        }
    }
}

struct MockQilinServer {
    base_url: String,
    state: Arc<ServerState>,
    shutdown_tx: Option<oneshot::Sender<()>>,
    handle: tokio::task::JoinHandle<()>,
}

impl MockQilinServer {
    async fn spawn(profile: BenchmarkProfile) -> Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .context("bind mock Qilin benchmark listener")?;
        let addr = listener
            .local_addr()
            .context("read local benchmark address")?;
        let base_url = format!("http://127.0.0.1:{}/bench/", addr.port());
        let state = Arc::new(ServerState::new(profile));
        let (shutdown_tx, mut shutdown_rx) = oneshot::channel::<()>();
        let task_state = Arc::clone(&state);

        let handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => {
                        break;
                    }
                    accept = listener.accept() => {
                        match accept {
                            Ok((stream, _)) => {
                                let connection_state = Arc::clone(&task_state);
                                tokio::spawn(async move {
                                    let _ = handle_connection(stream, connection_state).await;
                                });
                            }
                            Err(_) => break,
                        }
                    }
                }
            }
        });

        Ok(Self {
            base_url,
            state,
            shutdown_tx: Some(shutdown_tx),
            handle,
        })
    }

    async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        let _ = self.handle.await;
    }

    fn snapshot(&self) -> ServerStatsSnapshot {
        ServerStatsSnapshot {
            requests: self.state.stats.requests.load(Ordering::Relaxed),
            throttles: self.state.stats.throttles.load(Ordering::Relaxed),
            slow_responses: self.state.stats.slow_responses.load(Ordering::Relaxed),
        }
    }
}

#[derive(Debug, Serialize)]
struct ServerStatsSnapshot {
    requests: usize,
    throttles: usize,
    slow_responses: usize,
}

#[derive(Debug, Serialize)]
struct BenchmarkRun {
    profile: String,
    circuits: usize,
    discovered_entries: usize,
    expected_entries: usize,
    file_entries: usize,
    folder_entries: usize,
    elapsed_secs: f64,
    entries_per_sec: f64,
    requests: usize,
    throttles: usize,
    slow_responses: usize,
    complete: bool,
}

#[derive(Debug, Serialize)]
struct BenchmarkReport {
    max_depth: usize,
    dirs_per_level: usize,
    files_per_dir: usize,
    expected_entries: usize,
    runs: Vec<BenchmarkRun>,
}

enum MockResponse {
    Directory(String),
    File(Vec<u8>),
    Throttle(String),
    NotFound(String),
}

#[tokio::main(flavor = "multi_thread", worker_threads = 8)]
async fn main() -> Result<()> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let app = tauri::Builder::default()
        .manage(crawli_lib::AppState::default())
        .build(tauri::generate_context!())
        .context("build tauri app")?;

    let vfs_path =
        std::env::temp_dir().join(format!("crawli_qilin_benchmark_vfs_{}", std::process::id()));
    let state = app.handle().state::<crawli_lib::AppState>();
    state
        .vfs
        .initialize(&vfs_path.to_string_lossy())
        .await
        .context("initialize benchmark VFS")?;

    let expected_entries = expected_entry_count(MAX_DEPTH, DIRS_PER_LEVEL, FILES_PER_DIR);
    let mut runs = Vec::new();

    println!("=======================================================");
    println!("QILIN SYNTHETIC BENCHMARK");
    println!("=======================================================");
    println!(
        "Tree: depth={} dirs/level={} files/dir={} expected_entries={}",
        MAX_DEPTH, DIRS_PER_LEVEL, FILES_PER_DIR, expected_entries
    );
    println!("This benchmark does not touch any live hidden service.");
    println!();

    for profile in BENCHMARK_PROFILES {
        for &circuits in CIRCUIT_MATRIX {
            let run = run_case(app.handle().clone(), *profile, circuits, expected_entries).await?;
            println!(
                "[{} | {:>2} circuits] entries={} files={} dirs={} elapsed={:.2}s rate={:.1}/s requests={} throttles={} complete={}",
                run.profile,
                run.circuits,
                run.discovered_entries,
                run.file_entries,
                run.folder_entries,
                run.elapsed_secs,
                run.entries_per_sec,
                run.requests,
                run.throttles,
                run.complete
            );
            runs.push(run);
        }
    }

    let report = BenchmarkReport {
        max_depth: MAX_DEPTH,
        dirs_per_level: DIRS_PER_LEVEL,
        files_per_dir: FILES_PER_DIR,
        expected_entries,
        runs,
    };

    let report_path =
        "/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/tmp/qilin_benchmark_latest.json";
    if let Some(parent) = std::path::Path::new(report_path).parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(report_path, serde_json::to_string_pretty(&report)?)?;
    println!();
    println!("Report written to {}", report_path);

    Ok(())
}

async fn run_case(
    app: tauri::AppHandle,
    profile: BenchmarkProfile,
    circuits: usize,
    expected_entries: usize,
) -> Result<BenchmarkRun> {
    let state_handle = app.clone();
    let state = state_handle.state::<crawli_lib::AppState>();
    state.vfs.clear().await.context("clear benchmark VFS")?;

    let server = MockQilinServer::spawn(profile).await?;
    let target = server.base_url.clone();

    let options = CrawlOptions {
        listing: true,
        sizes: true,
        download: false,
        circuits: Some(circuits),
        agnostic_state: false,
        resume: false,
        resume_index: None,
        mega_password: None,
        stealth_ramp: true, parallel_download: false,
            download_mode: crawli_lib::frontier::DownloadMode::Medium,
            force_clearnet: false,
    };

    let frontier = Arc::new(CrawlerFrontier::new(
        Some(app.clone()),
        target.clone(),
        circuits,
        false,
        vec![0; circuits.max(1)],
        Vec::new(),
        options,
        None,
    ));

    let adapter = QilinAdapter;
    let started = Instant::now();
    let entries = adapter
        .crawl(&target, frontier, app)
        .await
        .context("run synthetic Qilin benchmark crawl")?;
    let summary = state
        .vfs
        .summarize_entries()
        .await
        .context("summarize benchmark VFS")?;
    let elapsed = started.elapsed().as_secs_f64();
    let stats = server.snapshot();
    server.shutdown().await;

    let (discovered_entries, file_entries, folder_entries) = if summary.discovered_count > 0 {
        (
            summary.discovered_count,
            summary.file_count,
            summary.folder_count,
        )
    } else {
        let file_entries = entries
            .iter()
            .filter(|entry| matches!(entry.entry_type, EntryType::File))
            .count();
        let folder_entries = entries
            .iter()
            .filter(|entry| matches!(entry.entry_type, EntryType::Folder))
            .count();
        (entries.len(), file_entries, folder_entries)
    };

    Ok(BenchmarkRun {
        profile: profile.name.to_string(),
        circuits,
        discovered_entries,
        expected_entries,
        file_entries,
        folder_entries,
        elapsed_secs: elapsed,
        entries_per_sec: discovered_entries as f64 / elapsed.max(0.001),
        requests: stats.requests,
        throttles: stats.throttles,
        slow_responses: stats.slow_responses,
        complete: discovered_entries == expected_entries,
    })
}

async fn handle_connection(stream: TcpStream, state: Arc<ServerState>) -> Result<()> {
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);

    let mut request_line = String::new();
    if reader.read_line(&mut request_line).await? == 0 {
        return Ok(());
    }

    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).await? == 0 || line == "\r\n" {
            break;
        }
    }

    let path = request_line
        .split_whitespace()
        .nth(1)
        .unwrap_or("/")
        .split('?')
        .next()
        .unwrap_or("/")
        .to_string();

    state.stats.requests.fetch_add(1, Ordering::Relaxed);
    let response = respond_for_path(&path, &state).await;

    match response {
        MockResponse::Directory(body) => {
            write_http_response(
                &mut write_half,
                "200 OK",
                "text/html; charset=utf-8",
                body.as_bytes(),
            )
            .await?;
        }
        MockResponse::File(body) => {
            write_http_response(&mut write_half, "200 OK", "application/octet-stream", &body)
                .await?;
        }
        MockResponse::Throttle(body) => {
            write_http_response(
                &mut write_half,
                "429 Too Many Requests",
                "text/plain; charset=utf-8",
                body.as_bytes(),
            )
            .await?;
        }
        MockResponse::NotFound(body) => {
            write_http_response(
                &mut write_half,
                "404 Not Found",
                "text/plain; charset=utf-8",
                body.as_bytes(),
            )
            .await?;
        }
    }

    Ok(())
}

async fn respond_for_path(path: &str, state: &ServerState) -> MockResponse {
    let request_count = {
        let mut guard = state.request_counts.lock().unwrap();
        let entry = guard.entry(path.to_string()).or_insert(0);
        *entry += 1;
        *entry
    };

    if let Some(delay) = delay_for_path(path, state.profile) {
        if delay > state.profile.base_delay_ms {
            state.stats.slow_responses.fetch_add(1, Ordering::Relaxed);
        }
        tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
    }

    if should_throttle(path, request_count, state.profile) {
        state.stats.throttles.fetch_add(1, Ordering::Relaxed);
        return MockResponse::Throttle("synthetic throttle".to_string());
    }

    if let Some(dir_depth) = directory_depth(path) {
        return MockResponse::Directory(render_directory(dir_depth));
    }

    if file_bytes(path).is_some() {
        return MockResponse::File(vec![b'X'; 128]);
    }

    MockResponse::NotFound("synthetic not found".to_string())
}

async fn write_http_response(
    write_half: &mut tokio::net::tcp::OwnedWriteHalf,
    status: &str,
    content_type: &str,
    body: &[u8],
) -> Result<()> {
    let headers = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    write_half.write_all(headers.as_bytes()).await?;
    write_half.write_all(body).await?;
    write_half.shutdown().await?;
    Ok(())
}

fn should_throttle(path: &str, request_count: usize, profile: BenchmarkProfile) -> bool {
    if request_count > 1 {
        return false;
    }
    match profile.throttle_every {
        Some(modulus) if modulus > 0 => stable_hash(path).is_multiple_of(modulus),
        _ => false,
    }
}

fn delay_for_path(path: &str, profile: BenchmarkProfile) -> Option<u64> {
    let mut delay = profile.base_delay_ms;
    if let Some(modulus) = profile.slow_every {
        if modulus > 0 && stable_hash(path).is_multiple_of(modulus) {
            delay = delay.saturating_add(profile.slow_extra_delay_ms);
        }
    }
    Some(delay)
}

fn stable_hash(value: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

fn directory_depth(path: &str) -> Option<usize> {
    let normalized = normalize_path(path)?;
    if normalized.is_empty() {
        return Some(0);
    }
    if normalized.last()?.contains('.') {
        return None;
    }
    Some(normalized.len())
}

fn file_bytes(path: &str) -> Option<u64> {
    let normalized = normalize_path(path)?;
    let last = normalized.last()?;
    if !last.contains('.') {
        return None;
    }
    Some(128)
}

fn normalize_path(path: &str) -> Option<Vec<String>> {
    if !path.starts_with("/bench/") {
        return None;
    }
    let trimmed = path.trim_start_matches("/bench/").trim_end_matches('/');
    if trimmed.is_empty() {
        return Some(Vec::new());
    }
    Some(
        trimmed
            .split('/')
            .map(|segment| segment.to_string())
            .collect(),
    )
}

fn render_directory(depth: usize) -> String {
    let mut rows = Vec::new();

    if depth < MAX_DEPTH {
        for idx in 0..DIRS_PER_LEVEL {
            let dir_name = format!("dir_{depth}_{idx:03}/");
            rows.push(format!(
                r#"<tr><td class="link"><a href="{dir_name}">{dir_name}</a></td><td class="size">-</td></tr>"#
            ));
        }
    }

    for idx in 0..FILES_PER_DIR {
        let file_name = format!("file_{depth}_{idx:03}.bin");
        let size_mb = 5 + depth as u64 + idx as u64;
        rows.push(format!(
            r#"<tr><td class="link"><a href="{file_name}">{file_name}</a></td><td class="size">{size_mb}.00 MB</td></tr>"#
        ));
    }

    format!(
        r#"<!doctype html>
<html>
  <head><title>QData Synthetic Benchmark</title></head>
  <body>
    <div class="page-header-title">QData</div>
    <div>Data browser</div>
    <table id="list">
      {}
    </table>
  </body>
</html>"#,
        rows.join("\n")
    )
}

fn expected_entry_count(max_depth: usize, dirs_per_level: usize, files_per_dir: usize) -> usize {
    let mut total_directories_including_root = 0usize;
    let mut level_dirs = 1usize;

    for _depth in 0..=max_depth {
        total_directories_including_root =
            total_directories_including_root.saturating_add(level_dirs);
        level_dirs = level_dirs.saturating_mul(dirs_per_level);
    }

    let non_root_directories = total_directories_including_root.saturating_sub(1);
    let file_entries = total_directories_including_root.saturating_mul(files_per_dir);
    non_root_directories.saturating_add(file_entries)
}
