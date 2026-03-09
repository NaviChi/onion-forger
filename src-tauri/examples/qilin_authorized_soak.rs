use anyhow::{anyhow, Context, Result};
use crawli_lib::adapters::FileEntry;
use crawli_lib::aria_downloader;
use crawli_lib::db::VfsSummary;
use crawli_lib::frontier::CrawlOptions;
use crawli_lib::runtime_metrics::{self, ResourceMetricsSnapshot};
use crawli_lib::telemetry_bridge::{self, TelemetryBridgeUpdate};
use crawli_lib::{path_utils, start_crawl_for_example, AppState, CrawlSessionResult};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{Listener, Manager};

#[derive(Debug)]
struct SoakConfig {
    url: String,
    duration_secs: u64,
    mode: String,
    circuits_ceiling: usize,
    daemons: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TimedMetricSample {
    elapsed_secs: u64,
    metrics: ResourceMetricsSnapshot,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BatchProgressSample {
    elapsed_secs: u64,
    current_file: String,
    completed: usize,
    failed: usize,
    total: usize,
    downloaded_bytes: u64,
    active_circuits: Option<usize>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SoakReport {
    target_url: String,
    runtime: String,
    duration_secs: u64,
    mode: String,
    circuits_ceiling: usize,
    daemons: usize,
    crawl_result: Option<CrawlSessionResult>,
    partial_vfs_summary: Option<VfsSummary>,
    crawl_error: Option<String>,
    selected_large_file: Option<String>,
    metrics_timeline: Vec<TimedMetricSample>,
    batch_progress: Vec<BatchProgressSample>,
    node_decisions: Vec<String>,
    crawl_logs: Vec<String>,
    output_dir: String,
    report_written_at_epoch: u64,
}

#[tokio::main(flavor = "multi_thread", worker_threads = 8)]
async fn main() -> Result<()> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let config = parse_args()?;
    let app = tauri::Builder::default()
        .manage(AppState::default())
        .build(tauri::generate_context!())
        .context("build tauri app")?;

    let telemetry = app.state::<AppState>().telemetry.clone();
    let bridge = app.state::<AppState>().telemetry_bridge.clone();
    runtime_metrics::spawn_metrics_emitter(app.handle().clone(), telemetry);
    telemetry_bridge::spawn_bridge_emitter(app.handle().clone(), bridge);

    let started_at = Instant::now();
    let metric_samples: Arc<Mutex<Vec<TimedMetricSample>>> = Arc::new(Mutex::new(Vec::new()));
    let batch_samples: Arc<Mutex<Vec<BatchProgressSample>>> = Arc::new(Mutex::new(Vec::new()));
    let node_decisions: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let crawl_logs: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

    {
        let metric_sink = Arc::clone(&metric_samples);
        let batch_sink = Arc::clone(&batch_samples);
        let start = started_at;
        app.listen("telemetry_bridge_update", move |event| {
            let payload = event.payload();
            if let Ok(update) = serde_json::from_str::<TelemetryBridgeUpdate>(payload) {
                if let Some(metrics) = update.resource_metrics {
                    if let Ok(mut guard) = metric_sink.lock() {
                        guard.push(TimedMetricSample {
                            elapsed_secs: start.elapsed().as_secs(),
                            metrics,
                        });
                    }
                }

                if let Some(progress) = update.batch_progress {
                    if let Ok(mut guard) = batch_sink.lock() {
                        guard.push(BatchProgressSample {
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
        });
    }

    {
        let logs = Arc::clone(&crawl_logs);
        app.listen("crawl_log", move |event| {
            let payload = event.payload();
            if let Ok(mut guard) = logs.lock() {
                guard.push(payload.to_string());
            }
        });
    }

    let timestamp = unix_now();
    let output_dir = PathBuf::from(format!(
        "/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/tmp/qilin_authorized_soak_{}",
        timestamp
    ));
    std::fs::create_dir_all(&output_dir)?;

    let crawl_timeout = Duration::from_secs(config.duration_secs.max(30));
    let report_path = PathBuf::from(
        "/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/tmp/qilin_authorized_soak_latest.json",
    );
    let partial_vfs_summary = || async {
        app.state::<AppState>()
            .vfs
            .summarize_entries()
            .await
            .ok()
            .filter(|summary| summary.discovered_count > 0)
    };

    let crawl_result = match tokio::time::timeout(
        crawl_timeout,
        start_crawl_for_example(
            config.url.clone(),
            CrawlOptions {
                listing: true,
                sizes: true,
                download: false,
                circuits: Some(config.circuits_ceiling),
                daemons: Some(config.daemons),
                agnostic_state: false,
                resume: false,
                resume_index: None,
                mega_password: None,
            },
            output_dir.to_string_lossy().to_string(),
            app.handle().clone(),
        ),
    )
    .await
    {
        Ok(Ok(result)) => Some(result),
        Ok(Err(err)) => {
            let report = SoakReport {
                target_url: config.url.clone(),
                runtime: "torforge".to_string(),
                duration_secs: config.duration_secs,
                mode: config.mode.clone(),
                circuits_ceiling: config.circuits_ceiling,
                daemons: config.daemons,
                crawl_result: None,
                partial_vfs_summary: partial_vfs_summary().await,
                crawl_error: Some(err.to_string()),
                selected_large_file: None,
                metrics_timeline: metric_samples.lock().map(|g| g.clone()).unwrap_or_default(),
                batch_progress: batch_samples.lock().map(|g| g.clone()).unwrap_or_default(),
                node_decisions: node_decisions.lock().map(|g| g.clone()).unwrap_or_default(),
                crawl_logs: crawl_logs.lock().map(|g| g.clone()).unwrap_or_default(),
                output_dir: output_dir.to_string_lossy().to_string(),
                report_written_at_epoch: unix_now(),
            };
            write_report(&report_path, &report)?;
            return Err(anyhow!(err));
        }
        Err(_) => {
            let report = SoakReport {
                target_url: config.url.clone(),
                runtime: "torforge".to_string(),
                duration_secs: config.duration_secs,
                mode: config.mode.clone(),
                circuits_ceiling: config.circuits_ceiling,
                daemons: config.daemons,
                crawl_result: None,
                partial_vfs_summary: partial_vfs_summary().await,
                crawl_error: Some("authorized soak crawl timed out".to_string()),
                selected_large_file: None,
                metrics_timeline: metric_samples.lock().map(|g| g.clone()).unwrap_or_default(),
                batch_progress: batch_samples.lock().map(|g| g.clone()).unwrap_or_default(),
                node_decisions: node_decisions.lock().map(|g| g.clone()).unwrap_or_default(),
                crawl_logs: crawl_logs.lock().map(|g| g.clone()).unwrap_or_default(),
                output_dir: output_dir.to_string_lossy().to_string(),
                report_written_at_epoch: unix_now(),
            };
            write_report(&report_path, &report)?;
            return Err(anyhow!("authorized soak crawl timed out"));
        }
    };

    let mut selected_large_file = None;
    if config.mode == "listing-plus-one-large-file" && crawl_result.is_some() {
        let remaining = config
            .duration_secs
            .saturating_sub(started_at.elapsed().as_secs());
        if remaining > 10 {
            let entries = app
                .state::<AppState>()
                .vfs
                .iter_entries()
                .await
                .context("read VFS entries after crawl")?;
            if let Some(entry) = choose_large_file(&entries) {
                selected_large_file = Some(entry.path.clone());
                let safe_target =
                    path_utils::resolve_download_target_within_root(&output_dir, &entry.path)
                        .context("resolve large-file soak target")?;
                let control = aria_downloader::activate_download_control()
                    .ok_or_else(|| anyhow!("a download is already active"))?;
                let download_result = tokio::time::timeout(
                    Duration::from_secs(remaining),
                    aria_downloader::start_download(
                        app.handle().clone(),
                        crawli_lib::aria_downloader::BatchFileEntry {
                            url: entry.raw_url.clone(),
                            path: safe_target.to_string_lossy().to_string(),
                            size_hint: entry.size_bytes,
                            jwt_exp: entry.jwt_exp,
                        },
                        config.circuits_ceiling,
                        entry.raw_url.contains(".onion"),
                        Some(output_dir.to_string_lossy().to_string()),
                        control,
                        Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
                    ),
                )
                .await;
                aria_downloader::clear_download_control();
                if download_result.is_err() {
                    let _ = aria_downloader::request_stop();
                } else {
                    download_result??;
                }
            }
        }
    }

    let report = SoakReport {
        target_url: config.url,
        runtime: "torforge".to_string(),
        duration_secs: config.duration_secs,
        mode: config.mode,
        circuits_ceiling: config.circuits_ceiling,
        daemons: config.daemons,
        crawl_result,
        partial_vfs_summary: partial_vfs_summary().await,
        crawl_error: None,
        selected_large_file,
        metrics_timeline: metric_samples.lock().map(|g| g.clone()).unwrap_or_default(),
        batch_progress: batch_samples.lock().map(|g| g.clone()).unwrap_or_default(),
        node_decisions: node_decisions.lock().map(|g| g.clone()).unwrap_or_default(),
        crawl_logs: crawl_logs.lock().map(|g| g.clone()).unwrap_or_default(),
        output_dir: output_dir.to_string_lossy().to_string(),
        report_written_at_epoch: unix_now(),
    };
    write_report(&report_path, &report)?;
    println!(
        "Authorized soak report written to {}",
        report_path.display()
    );
    Ok(())
}

fn write_report(report_path: &PathBuf, report: &SoakReport) -> Result<()> {
    std::fs::write(report_path, serde_json::to_string_pretty(report)?)?;
    Ok(())
}

fn choose_large_file(entries: &[FileEntry]) -> Option<FileEntry> {
    entries
        .iter()
        .filter(|entry| entry.entry_type == crawli_lib::adapters::EntryType::File)
        .filter(|entry| entry.size_bytes.unwrap_or(0) >= 10 * 1024 * 1024)
        .max_by_key(|entry| entry.size_bytes.unwrap_or(0))
        .cloned()
}

fn parse_args() -> Result<SoakConfig> {
    let mut url = None;
    let mut duration_secs = 300u64;
    let mut mode = "listing-plus-one-large-file".to_string();
    let mut circuits_ceiling = 120usize;
    let mut daemons = 1usize; // Rely on MultiClientPool for scaling

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--url" => url = args.next(),
            "--duration-secs" => {
                duration_secs = args
                    .next()
                    .ok_or_else(|| anyhow!("missing value for --duration-secs"))?
                    .parse()
                    .context("parse --duration-secs")?;
            }
            "--mode" => {
                mode = args
                    .next()
                    .ok_or_else(|| anyhow!("missing value for --mode"))?;
            }
            "--circuits-ceiling" => {
                circuits_ceiling = args
                    .next()
                    .ok_or_else(|| anyhow!("missing value for --circuits-ceiling"))?
                    .parse()
                    .context("parse --circuits-ceiling")?;
            }
            "--daemons" => {
                daemons = args
                    .next()
                    .ok_or_else(|| anyhow!("missing value for --daemons"))?
                    .parse()
                    .context("parse --daemons")?;
            }
            other => return Err(anyhow!("unknown arg: {}", other)),
        }
    }

    Ok(SoakConfig {
        url: url.ok_or_else(|| anyhow!("--url is required"))?,
        duration_secs,
        mode,
        circuits_ceiling,
        daemons,
    })
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
