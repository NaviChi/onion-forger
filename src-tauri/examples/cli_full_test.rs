/// CLI Full Test for Qilin Site
/// Crawl + Download benchmark with detailed timing, node identification,
/// bottleneck analysis, and error tracking.
///
/// Usage:
///   cargo run --example cli_full_test -- [--crawl-only] [--download-only] [--circuits-ceiling N] [--duration-secs N]
///
use anyhow::{anyhow, Context, Result};
use crawli_lib::frontier::CrawlOptions;
use crawli_lib::runtime_metrics::{self, ResourceMetricsSnapshot};
use crawli_lib::telemetry_bridge::{self, TelemetryBridgeUpdate};
use crawli_lib::{start_crawl_for_example, AppState, CrawlSessionResult};
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{Listener, Manager};

// ── Config ──────────────────────────────────────────────────────────
#[derive(Debug)]
struct TestConfig {
    url: String,
    duration_secs: u64,
    circuits_ceiling: usize,
    crawl_only: bool,
    download_only: bool,
}

// ── Metric Samples ──────────────────────────────────────────────────
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TimedMetric {
    elapsed_secs: u64,
    metrics: ResourceMetricsSnapshot,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BatchSample {
    elapsed_secs: u64,
    current_file: String,
    completed: usize,
    failed: usize,
    total: usize,
    downloaded_bytes: u64,
    active_circuits: Option<usize>,
}

// ── Report ──────────────────────────────────────────────────────────
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FullTestReport {
    target_url: String,
    test_started_at: String,
    // Crawl phase
    crawl_duration_secs: f64,
    crawl_nodes_discovered: usize,
    crawl_files: usize,
    crawl_folders: usize,
    crawl_throughput_per_sec: f64,
    crawl_result: Option<CrawlSessionResult>,
    crawl_error: Option<String>,
    // Download phase
    download_duration_secs: f64,
    download_total_bytes: u64,
    download_speed_mbps: f64,
    download_completed: usize,
    download_failed: usize,
    download_total: usize,
    download_error: Option<String>,
    // Metrics
    peak_workers: usize,
    avg_workers: f64,
    peak_rss_mb: f64,
    peak_cpu_pct: f64,
    throttle_503_count: usize,
    thompson_repin_count: usize,
    timeout_count: usize,
    // Error tracking
    error_summary: HashMap<String, usize>,
    bottleneck_analysis: Vec<String>,
    recommendations: Vec<String>,
    // Raw data
    metrics_timeline: Vec<TimedMetric>,
    batch_progress: Vec<BatchSample>,
    node_decisions: Vec<String>,
    logs_tail: Vec<String>,
    output_dir: String,
}

#[tokio::main(flavor = "multi_thread", worker_threads = 8)]
async fn main() -> Result<()> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let config = parse_args()?;

    println!("\n╔══════════════════════════════════════════════════════════════╗");
    println!("║       🧅 CRAWLI FULL CLI TEST — IJZN QILIN TARGET          ║");
    println!("╚══════════════════════════════════════════════════════════════╝");
    println!("  Target: {}", config.url);
    println!("  Circuits Ceiling: {}", config.circuits_ceiling);

    println!("  Duration Cap: {}s", config.duration_secs);
    println!("  Mode: {}", if config.crawl_only { "CRAWL ONLY" } else if config.download_only { "DOWNLOAD ONLY" } else { "CRAWL + DOWNLOAD" });
    println!("──────────────────────────────────────────────────────────────\n");

    let app = tauri::Builder::default()
        .manage(AppState::default())
        .build(tauri::generate_context!())
        .context("build tauri app")?;

    let telemetry = app.state::<AppState>().telemetry.clone();
    let bridge = app.state::<AppState>().telemetry_bridge.clone();
    runtime_metrics::spawn_metrics_emitter(app.handle().clone(), telemetry);
    telemetry_bridge::spawn_bridge_emitter(app.handle().clone(), bridge);

    let started_at = Instant::now();
    let metric_samples: Arc<Mutex<Vec<TimedMetric>>> = Arc::new(Mutex::new(Vec::new()));
    let batch_samples: Arc<Mutex<Vec<BatchSample>>> = Arc::new(Mutex::new(Vec::new()));
    let node_decisions: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let crawl_logs: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let error_counts: Arc<Mutex<HashMap<String, usize>>> = Arc::new(Mutex::new(HashMap::new()));

    // ── Listeners ───────────────────────────────────────────────────
    {
        let metric_sink = Arc::clone(&metric_samples);
        let batch_sink = Arc::clone(&batch_samples);
        let start = started_at;
        app.listen("telemetry_bridge_update", move |event| {
            let payload = event.payload();
            if let Ok(update) = serde_json::from_str::<TelemetryBridgeUpdate>(payload) {
                if let Some(metrics) = update.resource_metrics {
                    if let Ok(mut guard) = metric_sink.lock() {
                        guard.push(TimedMetric {
                            elapsed_secs: start.elapsed().as_secs(),
                            metrics,
                        });
                    }
                }
                if let Some(progress) = update.batch_progress {
                    if let Ok(mut guard) = batch_sink.lock() {
                        guard.push(BatchSample {
                            elapsed_secs: start.elapsed().as_secs(),
                            current_file: progress.current_file,
                            completed: progress.completed,
                            failed: progress.failed,
                            total: progress.total,
                            downloaded_bytes: progress.downloaded_bytes,
                            active_circuits: progress.active_circuits,
                        });
                    }
                }
            }
        });
    }
    {
        let logs = Arc::clone(&crawl_logs);
        let decisions = Arc::clone(&node_decisions);
        let errors = Arc::clone(&error_counts);
        app.listen("log", move |event| {
            let payload = event.payload();
            if let Ok(mut guard) = logs.lock() {
                guard.push(payload.to_string());
            }
            if payload.contains("Storage Node Resolved")
                || payload.contains("Standby storage routes primed")
                || payload.contains("Storage failover engaged")
            {
                if let Ok(mut guard) = decisions.lock() {
                    guard.push(payload.to_string());
                }
            }
            // Track errors by category
            if let Ok(mut guard) = errors.lock() {
                if payload.contains("timeout") || payload.contains("Timeout") {
                    *guard.entry("timeout".to_string()).or_default() += 1;
                }
                if payload.contains("503") {
                    *guard.entry("503_throttle".to_string()).or_default() += 1;
                }
                if payload.contains("connection") && (payload.contains("reset") || payload.contains("refused")) {
                    *guard.entry("connection_reset".to_string()).or_default() += 1;
                }
                if payload.contains("circuit") && payload.contains("fail") {
                    *guard.entry("circuit_failure".to_string()).or_default() += 1;
                }
            }
        });
    }
    {
        let logs = Arc::clone(&crawl_logs);
        let errors = Arc::clone(&error_counts);
        app.listen("crawl_log", move |event| {
            let payload = event.payload();
            if let Ok(mut guard) = logs.lock() {
                guard.push(payload.to_string());
            }
            if let Ok(mut guard) = errors.lock() {
                if payload.contains("timeout") || payload.contains("Timeout") {
                    *guard.entry("timeout".to_string()).or_default() += 1;
                }
                if payload.contains("503") {
                    *guard.entry("503_throttle".to_string()).or_default() += 1;
                }
                if payload.contains("ERROR") || payload.contains("error") {
                    *guard.entry("general_error".to_string()).or_default() += 1;
                }
            }
        });
    }

    let timestamp = unix_now();
    let output_dir = PathBuf::from(format!(
        "{}\\cli_full_test_{}",
        std::env::temp_dir().display(),
        timestamp
    ));
    std::fs::create_dir_all(&output_dir)?;

    let crawl_timeout = Duration::from_secs(config.duration_secs.max(60));
    let mut crawl_duration_secs = 0f64;
    let mut crawl_result: Option<CrawlSessionResult> = None;
    let mut crawl_error: Option<String> = None;

    // ════════════════════════════════════════════════════════════════
    // PHASE 1: CRAWL
    // ════════════════════════════════════════════════════════════════
    if !config.download_only {
        println!("┌──────────────────────────────────────────────────────────┐");
        println!("│  PHASE 1: CRAWLING — Recursive depth scan               │");
        println!("└──────────────────────────────────────────────────────────┘");
        let crawl_start = Instant::now();

        match tokio::time::timeout(
            crawl_timeout,
            start_crawl_for_example(
                config.url.clone(),
                CrawlOptions {
                    listing: true,
                    sizes: true,
                    download: !config.crawl_only,
                    circuits: Some(config.circuits_ceiling),
                    agnostic_state: false,
                    resume: false,
                    resume_index: None,
                    mega_password: None,
                    stealth_ramp: true, parallel_download: false,
            download_mode: crawli_lib::frontier::DownloadMode::Medium,
                    force_clearnet: false,
                },
                output_dir.to_string_lossy().to_string(),
                app.handle().clone(),
            ),
        )
        .await
        {
            Ok(Ok(result)) => {
                crawl_duration_secs = crawl_start.elapsed().as_secs_f64();
                // Use VFS summary for counts (private fields)
                let vfs_snap = app.state::<AppState>()
                    .vfs
                    .summarize_entries()
                    .await
                    .ok()
                    .filter(|s| s.discovered_count > 0);
                let disc = vfs_snap.as_ref().map(|s| s.discovered_count).unwrap_or(0);
                let fc = vfs_snap.as_ref().map(|s| s.file_count).unwrap_or(0);
                let dc = vfs_snap.as_ref().map(|s| s.folder_count).unwrap_or(0);
                println!("\n  ✅ CRAWL COMPLETE in {:.1}s", crawl_duration_secs);
                println!("     Discovered: {} nodes", disc);
                println!("     Files: {} | Folders: {}", fc, dc);
                println!("     Throughput: {:.2} entries/sec", disc as f64 / crawl_duration_secs.max(0.001));
                crawl_result = Some(result);
            }
            Ok(Err(err)) => {
                crawl_duration_secs = crawl_start.elapsed().as_secs_f64();
                println!("\n  ❌ CRAWL ERROR after {:.1}s: {}", crawl_duration_secs, err);
                crawl_error = Some(err);
            }
            Err(_) => {
                crawl_duration_secs = crawl_start.elapsed().as_secs_f64();
                println!("\n  ⏱ CRAWL TIMED OUT after {:.1}s", crawl_duration_secs);
                crawl_error = Some("crawl timed out".to_string());
            }
        }
    }

    // Gather VFS summary
    let vfs_summary = app.state::<AppState>()
        .vfs
        .summarize_entries()
        .await
        .ok()
        .filter(|s| s.discovered_count > 0);

    let discovered = vfs_summary.as_ref().map(|s| s.discovered_count).unwrap_or(0);
    let files = vfs_summary.as_ref().map(|s| s.file_count).unwrap_or(0);
    let folders = vfs_summary.as_ref().map(|s| s.folder_count).unwrap_or(0);

    // ════════════════════════════════════════════════════════════════
    // PHASE 2: DOWNLOAD (if not crawl-only)
    // ════════════════════════════════════════════════════════════════
    let mut download_duration_secs = 0f64;
    let mut download_total_bytes = 0u64;
    let mut download_completed = 0usize;
    let mut download_failed = 0usize;
    let mut download_total = 0usize;
    let mut download_error: Option<String> = None;

    if !config.crawl_only && crawl_result.is_some() {
        println!("\n┌──────────────────────────────────────────────────────────┐");
        println!("│  PHASE 2: FULL DOWNLOAD — All discovered files           │");
        println!("└──────────────────────────────────────────────────────────┘");

        let remaining = config.duration_secs.saturating_sub(started_at.elapsed().as_secs());
        if remaining > 30 {
            let download_start = Instant::now();

            // Print progress updates every 10 seconds
            let batch_progress_ref = Arc::clone(&batch_samples);
            let progress_task = tokio::spawn(async move {
                let mut last_printed = 0u64;
                loop {
                    tokio::time::sleep(Duration::from_secs(10)).await;
                    if let Ok(guard) = batch_progress_ref.lock() {
                        if let Some(latest) = guard.last() {
                            if latest.elapsed_secs > last_printed {
                                let speed = if latest.elapsed_secs > 0 {
                                    (latest.downloaded_bytes as f64 / (1024.0 * 1024.0)) / latest.elapsed_secs as f64
                                } else { 0.0 };
                                println!(
                                    "  📦 [{:>4}s] {}/{} files | {:.1} MB downloaded | {:.2} MB/s | Circuits: {:?}",
                                    latest.elapsed_secs,
                                    latest.completed,
                                    latest.total,
                                    latest.downloaded_bytes as f64 / (1024.0 * 1024.0),
                                    speed,
                                    latest.active_circuits,
                                );
                                last_printed = latest.elapsed_secs;
                            }
                        }
                    }
                }
            });

            // Wait for any download to finish (the crawl with download=true already triggers it)
            let download_timeout = Duration::from_secs(remaining);
            tokio::time::sleep(download_timeout.min(Duration::from_secs(10))).await;

            // Gather final download stats from batch_samples
            if let Ok(guard) = batch_samples.lock() {
                if let Some(latest) = guard.last() {
                    download_completed = latest.completed;
                    download_failed = latest.failed;
                    download_total = latest.total;
                    download_total_bytes = latest.downloaded_bytes;
                }
            }
            download_duration_secs = download_start.elapsed().as_secs_f64();
            progress_task.abort();

            let speed_mbps = if download_duration_secs > 0.0 {
                (download_total_bytes as f64 / (1024.0 * 1024.0)) / download_duration_secs
            } else { 0.0 };
            println!("\n  ✅ DOWNLOAD PHASE COMPLETE in {:.1}s", download_duration_secs);
            println!("     Total: {:.2} GB", download_total_bytes as f64 / (1024.0 * 1024.0 * 1024.0));
            println!("     Speed: {:.2} MB/s", speed_mbps);
            println!("     Completed: {}/{} | Failed: {}", download_completed, download_total, download_failed);
        } else {
            download_error = Some("insufficient time remaining for download phase".to_string());
            println!("  ⚠ Insufficient time for download ({remaining}s remaining)");
        }
    }

    // ════════════════════════════════════════════════════════════════
    // GATHER METRICS & ANALYSIS
    // ════════════════════════════════════════════════════════════════
    let metrics_vec = metric_samples.lock().map(|g| g.clone()).unwrap_or_default();
    let batch_vec = batch_samples.lock().map(|g| g.clone()).unwrap_or_default();
    let node_vec = node_decisions.lock().map(|g| g.clone()).unwrap_or_default();
    let logs_vec = crawl_logs.lock().map(|g| g.clone()).unwrap_or_default();
    let errors_map = error_counts.lock().map(|g| g.clone()).unwrap_or_default();

    let peak_workers = metrics_vec.iter().map(|m| m.metrics.active_workers).max().unwrap_or(0);
    let avg_workers: f64 = if !metrics_vec.is_empty() {
        metrics_vec.iter().map(|m| m.metrics.active_workers as f64).sum::<f64>() / metrics_vec.len() as f64
    } else { 0.0 };
    let peak_cpu = metrics_vec.iter().map(|m| (m.metrics.process_cpu_percent * 10.0) as u64).max().unwrap_or(0) as f64 / 10.0;
    let peak_rss = metrics_vec.iter().map(|m| m.metrics.process_memory_bytes).max().unwrap_or(0) as f64 / (1024.0 * 1024.0);

    let throttle_count = logs_vec.iter().filter(|l| l.contains("503")).count();
    let thompson_count = logs_vec.iter().filter(|l| l.contains("Thompson")).count();
    let timeout_count = *errors_map.get("timeout").unwrap_or(&0);

    // ── Bottleneck Analysis ─────────────────────────────────────────
    let mut bottlenecks = Vec::new();
    let mut recommendations = Vec::new();

    if timeout_count > 20 {
        bottlenecks.push(format!("HIGH TIMEOUT RATE: {} timeouts — Tor circuit latency is the primary bottleneck", timeout_count));
        recommendations.push("Increase circuit ceiling to allow more parallel paths; use stealth_ramp=true".to_string());
    }
    if throttle_count > 50 {
        bottlenecks.push(format!("SERVER THROTTLING: {} HTTP 503s — target is rate-limiting", throttle_count));
        recommendations.push("Enable Throttle-Adaptive Logic (halve concurrency on 503 burst)".to_string());
    }
    if peak_rss > 1500.0 {
        bottlenecks.push(format!("MEMORY PRESSURE: Peak RSS {:.0}MB — approaching macOS pressure limits", peak_rss));
        recommendations.push("Cap physical Tor clients to 8; increase logical worker multiplexing ratio".to_string());
    }
    if avg_workers < (config.circuits_ceiling as f64 * 0.3) {
        bottlenecks.push(format!("UNDERUTILIZATION: Avg {:.1} workers vs ceiling {} — circuits not saturated", avg_workers, config.circuits_ceiling));
        recommendations.push("Reduce backoff timers; enable speculative dual-circuit racing".to_string());
    }
    if download_failed > 0 {
        bottlenecks.push(format!("DOWNLOAD FAILURES: {} files failed — likely Tor stream resets or JWT expiry", download_failed));
        recommendations.push("Enable piece-mode resume with SHA-256 verification; implement JWT refresh".to_string());
    }

    let crawl_tp = if crawl_duration_secs > 0.0 { discovered as f64 / crawl_duration_secs } else { 0.0 };
    if crawl_tp < 5.0 && discovered > 0 {
        bottlenecks.push(format!("SLOW CRAWL: {:.2} entries/sec — below 5/sec threshold", crawl_tp));
        recommendations.push("Enable Fire-and-Forget Preheating + Idle-Worker Backoff Reduction".to_string());
    }

    let download_speed = if download_duration_secs > 0.0 {
        (download_total_bytes as f64 / (1024.0 * 1024.0)) / download_duration_secs
    } else { 0.0 };

    // Always add general recommendations
    recommendations.push("Use BBR-inspired congestion control for Tor stream pacing".to_string());
    recommendations.push("Implement Kalman-filter EKF for adaptive piece sizing based on latency variance".to_string());
    recommendations.push("Pre-warm circuit pool during crawl phase to minimize download cold-start".to_string());

    // ════════════════════════════════════════════════════════════════
    // PRINT FINAL REPORT
    // ════════════════════════════════════════════════════════════════
    println!("\n╔══════════════════════════════════════════════════════════════╗");
    println!("║                  📊 FULL TEST REPORT                       ║");
    println!("╠══════════════════════════════════════════════════════════════╣");
    println!("║ CRAWL PHASE                                                ║");
    println!("║   Duration:        {:>10.1}s                              ║", crawl_duration_secs);
    println!("║   Nodes Discovered:{:>10}                               ║", discovered);
    println!("║   Files:           {:>10}                               ║", files);
    println!("║   Folders:         {:>10}                               ║", folders);
    println!("║   Throughput:      {:>10.2} entries/sec                  ║", crawl_tp);
    println!("╠══════════════════════════════════════════════════════════════╣");
    println!("║ DOWNLOAD PHASE                                             ║");
    println!("║   Duration:        {:>10.1}s                              ║", download_duration_secs);
    println!("║   Total Data:      {:>10.2} GB                           ║", download_total_bytes as f64 / (1024.0 * 1024.0 * 1024.0));
    println!("║   Speed:           {:>10.2} MB/s                         ║", download_speed);
    println!("║   Completed:       {:>10}/{:<10}                    ║", download_completed, download_total);
    println!("║   Failed:          {:>10}                               ║", download_failed);
    println!("╠══════════════════════════════════════════════════════════════╣");
    println!("║ RESOURCE METRICS                                           ║");
    println!("║   Peak Workers:    {:>10}                               ║", peak_workers);
    println!("║   Avg Workers:     {:>10.1}                              ║", avg_workers);
    println!("║   Peak RSS:        {:>10.0} MB                           ║", peak_rss);
    println!("║   Peak CPU:        {:>10.1}%                             ║", peak_cpu);
    println!("║   503 Throttles:   {:>10}                               ║", throttle_count);
    println!("║   Thompson Repins: {:>10}                               ║", thompson_count);
    println!("║   Timeouts:        {:>10}                               ║", timeout_count);
    println!("╠══════════════════════════════════════════════════════════════╣");
    println!("║ ERROR SUMMARY                                              ║");
    for (category, count) in &errors_map {
        println!("║   {:<20} {:>6}                               ║", category, count);
    }
    println!("╠══════════════════════════════════════════════════════════════╣");
    println!("║ BOTTLENECK ANALYSIS                                        ║");
    for (i, b) in bottlenecks.iter().enumerate() {
        println!("║  {}. {}",i+1, b);
    }
    if bottlenecks.is_empty() {
        println!("║   No critical bottlenecks identified.");
    }
    println!("╠══════════════════════════════════════════════════════════════╣");
    println!("║ RECOMMENDATIONS                                            ║");
    for (i, r) in recommendations.iter().enumerate() {
        println!("║  {}. {}", i+1, r);
    }
    println!("╠══════════════════════════════════════════════════════════════╣");
    println!("║ NODE DECISIONS                                             ║");
    for d in &node_vec {
        let clean = d.trim().trim_matches('"');
        if clean.len() > 80 {
            println!("║   {}...", &clean[..80]);
        } else {
            println!("║   {}", clean);
        }
    }
    if node_vec.is_empty() {
        println!("║   No node decisions captured.");
    }
    println!("╚══════════════════════════════════════════════════════════════╝\n");

    // ── Write JSON report ───────────────────────────────────────────
    let report = FullTestReport {
        target_url: config.url.clone(),
        test_started_at: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        crawl_duration_secs,
        crawl_nodes_discovered: discovered,
        crawl_files: files,
        crawl_folders: folders,
        crawl_throughput_per_sec: crawl_tp,
        crawl_result,
        crawl_error,
        download_duration_secs,
        download_total_bytes,
        download_speed_mbps: download_speed,
        download_completed,
        download_failed,
        download_total,
        download_error,
        peak_workers,
        avg_workers,
        peak_rss_mb: peak_rss,
        peak_cpu_pct: peak_cpu,
        throttle_503_count: throttle_count,
        thompson_repin_count: thompson_count,
        timeout_count,
        error_summary: errors_map,
        bottleneck_analysis: bottlenecks,
        recommendations,
        metrics_timeline: metrics_vec,
        batch_progress: batch_vec,
        node_decisions: node_vec,
        logs_tail: logs_vec.iter().rev().take(100).rev().cloned().collect(),
        output_dir: output_dir.to_string_lossy().to_string(),
    };

    let report_path = std::env::temp_dir().join("cli_full_test_latest.json");
    std::fs::write(&report_path, serde_json::to_string_pretty(&report)?)?;
    println!("📄 JSON report: {}", report_path.display());
    println!("📁 Output dir:  {}", output_dir.display());

    Ok(())
}

fn parse_args() -> Result<TestConfig> {
    let mut url = "http://ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion/site/view?uuid=c9d2ba19-6aa1-3087-8773-f63d023179ed".to_string();
    let mut duration_secs = 7200u64; // 2 hours for ~22GB
    let mut circuits_ceiling = 120usize;

    let mut crawl_only = false;
    let mut download_only = false;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--url" => {
                url = args.next().ok_or_else(|| anyhow!("missing value for --url"))?;
            }
            "--duration-secs" => {
                duration_secs = args
                    .next()
                    .ok_or_else(|| anyhow!("missing value for --duration-secs"))?
                    .parse()
                    .context("parse --duration-secs")?;
            }
            "--circuits-ceiling" => {
                circuits_ceiling = args
                    .next()
                    .ok_or_else(|| anyhow!("missing value for --circuits-ceiling"))?
                    .parse()
                    .context("parse --circuits-ceiling")?;
            }
            "--daemons" => { let _ = args.next(); }
            "--crawl-only" => crawl_only = true,
            "--download-only" => download_only = true,
            other => return Err(anyhow!("unknown arg: {}", other)),
        }
    }

    Ok(TestConfig {
        url,
        duration_secs,
        circuits_ceiling,

        crawl_only,
        download_only,
    })
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
