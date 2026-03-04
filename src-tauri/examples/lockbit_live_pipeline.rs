use crawli_lib::adapters::{AdapterRegistry, EntryType, FileEntry, SiteFingerprint};
use crawli_lib::aria_downloader::{self, BatchFileEntry};
use crawli_lib::frontier::{CrawlOptions, CrawlerFrontier};
use crawli_lib::{path_utils, tor};
use reqwest::header::HeaderMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{Event, Listener};

const DEFAULT_LOCKBIT_URL: &str = "http://lockbit6vhrjaqzsdj6pqalyideigxv4xycfeyunpx35znogiwmojnid.onion/secret/212f70e703d758fbccbda3013a21f5de-f033da37-5fa7-31df-b10c-cc04b8538e85/jobberswarehouse.com/";

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct BatchProgressPayload {
    completed: usize,
    failed: usize,
    total: usize,
    current_file: String,
    speed_mbps: f64,
    downloaded_bytes: u64,
}

fn now_epoch_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn dir_size_bytes(root: &Path) -> u64 {
    fn walk(path: &Path, acc: &mut u64) {
        let entries = match std::fs::read_dir(path) {
            Ok(v) => v,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                walk(&p, acc);
            } else if let Ok(meta) = entry.metadata() {
                *acc = acc.saturating_add(meta.len());
            }
        }
    }

    let mut total = 0u64;
    walk(root, &mut total);
    total
}

fn bytes_to_gb(bytes: u64) -> f64 {
    bytes as f64 / 1_073_741_824.0
}

fn main() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build tokio runtime");

    rt.block_on(async {
        let url =
            std::env::var("LOCKBIT_LIVE_URL").unwrap_or_else(|_| DEFAULT_LOCKBIT_URL.to_string());
        let run_id = now_epoch_secs();
        let output_raw = format!("/tmp/onionforge_lockbit_live_{}", run_id);
        let output_root =
            path_utils::canonicalize_output_root(&output_raw).expect("canonicalize output root");

        println!("\\n=== LOCKBIT LIVE PIPELINE HARNESS ===");
        println!("URL: {}", url);
        println!("Output: {}", output_root.display());
        println!(
            "Settings: listing=true, sizes=true, download=false, circuits=120 (UI defaults)"
        );

        let app = tauri::Builder::default()
            .build(tauri::generate_context!())
            .expect("build tauri app");
        let app_handle = app.handle().clone();

        let completed = Arc::new(AtomicUsize::new(0));
        let failed = Arc::new(AtomicUsize::new(0));
        let total = Arc::new(AtomicUsize::new(0));
        let downloaded_bytes = Arc::new(AtomicU64::new(0));
        let speed_mbps_x100 = Arc::new(AtomicU64::new(0));
        let latest_file = Arc::new(std::sync::Mutex::new(String::new()));
        let stop_reporter = Arc::new(AtomicBool::new(false));

        let c_completed = completed.clone();
        let c_failed = failed.clone();
        let c_total = total.clone();
        let c_downloaded = downloaded_bytes.clone();
        let c_speed = speed_mbps_x100.clone();
        let c_latest = latest_file.clone();
        let batch_listener_id = app.listen_any("batch_progress", move |evt: Event| {
            if let Ok(payload) = serde_json::from_str::<BatchProgressPayload>(evt.payload()) {
                c_completed.store(payload.completed, Ordering::Relaxed);
                c_failed.store(payload.failed, Ordering::Relaxed);
                c_total.store(payload.total, Ordering::Relaxed);
                c_downloaded.store(payload.downloaded_bytes, Ordering::Relaxed);
                c_speed.store((payload.speed_mbps * 100.0).max(0.0) as u64, Ordering::Relaxed);
                if let Ok(mut guard) = c_latest.lock() {
                    *guard = payload.current_file;
                }
            }
        });

        let log_listener_id = app.listen_any("log", |evt: Event| {
            if let Ok(line) = serde_json::from_str::<String>(evt.payload()) {
                if line.contains("Phase 1")
                    || line.contains("Phase 2")
                    || line.contains("Batch routing")
                    || line.contains("Batch probe optimization")
                    || line.contains("Range mode")
                    || line.contains("Tournament")
                {
                    println!("[LOG] {}", line);
                }
            }
        });

        let crawl_log_listener_id = app.listen_any("crawl_log", |evt: Event| {
            if let Ok(line) = serde_json::from_str::<String>(evt.payload()) {
                if line.contains("Match found")
                    || line.contains("Finish signaled")
                    || line.contains("Bootstrapping")
                {
                    println!("[CRAWL] {}", line);
                }
            }
        });

        let tor_status_listener_id = app.listen_any("tor_status", |evt: Event| {
            let payload: serde_json::Value = serde_json::from_str(evt.payload()).unwrap_or_default();
            let state = payload
                .get("state")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let message = payload
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if state == "ready" || message.contains("winner") || message.contains("Consensus") {
                println!("[TOR] {} | {}", state, message);
            }
        });

        let stop_reporter_flag = stop_reporter.clone();
        let r_completed = completed.clone();
        let r_failed = failed.clone();
        let r_total = total.clone();
        let r_downloaded = downloaded_bytes.clone();
        let r_speed = speed_mbps_x100.clone();
        let r_latest = latest_file.clone();
        let reporter = tokio::spawn(async move {
            let started = Instant::now();
            loop {
                if stop_reporter_flag.load(Ordering::Relaxed) {
                    break;
                }
                tokio::time::sleep(Duration::from_secs(8)).await;
                let done = r_completed.load(Ordering::Relaxed);
                let fail = r_failed.load(Ordering::Relaxed);
                let all = r_total.load(Ordering::Relaxed);
                let rem = all.saturating_sub(done + fail);
                let speed = r_speed.load(Ordering::Relaxed) as f64 / 100.0;
                let dl_gb = bytes_to_gb(r_downloaded.load(Ordering::Relaxed));
                let elapsed = started.elapsed().as_secs();
                let current = r_latest
                    .lock()
                    .map(|g| g.clone())
                    .unwrap_or_else(|_| "<unknown>".to_string());
                println!(
                    "[MON] t={}s total={} done={} fail={} rem={} speed={:.2}MB/s downloaded={:.3}GB current={}",
                    elapsed, all, done, fail, rem, speed, dl_gb, current
                );
            }
        });

        let whole_start = Instant::now();

        println!("[STEP] Cleaning stale Tor daemons and bootstrapping 4-daemon swarm...");
        tor::cleanup_stale_tor_daemons();
        let tor_start = Instant::now();
        let (guard, active_ports) = tor::bootstrap_tor_cluster(app_handle.clone(), 4)
            .await
            .expect("bootstrap tor cluster");
        println!(
            "[STEP] Tor ready in {:.2}s on ports {:?}",
            tor_start.elapsed().as_secs_f64(),
            active_ports
        );

        let options = CrawlOptions {
            listing: true,
            sizes: true,
            download: false,
            circuits: Some(120),
            daemons: None,
        };

        let daemon_count = active_ports.len().max(1);
        let mut frontier = CrawlerFrontier::new(
            Some(app_handle.clone()),
            url.clone(),
            daemon_count,
            true,
            active_ports,
            options,
        );
        frontier.swarm_guard = Some(guard);

        println!("[STEP] Fingerprinting and selecting adapter...");
        let fp_start = Instant::now();
        let client = frontier.get_client().1;
        let resp = client
            .get(&url)
            .send()
            .await
            .expect("fetch target fingerprint response");
        let status = resp.status().as_u16();
        let headers: HeaderMap = resp.headers().clone();
        let body = resp
            .text()
            .await
            .unwrap_or_else(|_| "[DECODE_ERROR]".to_string());
        let fingerprint = SiteFingerprint {
            url: url.clone(),
            status,
            headers,
            body,
        };

        let registry = AdapterRegistry::new();
        let adapter = registry
            .determine_adapter(&fingerprint)
            .await
            .expect("adapter match");
        println!(
            "[STEP] Adapter={} determined in {:.2}s",
            adapter.name(),
            fp_start.elapsed().as_secs_f64()
        );

        println!("[STEP] Crawling recursively (default 120-circuit target)...");
        let crawl_start = Instant::now();
        let frontier_arc = Arc::new(frontier);
        let files: Vec<FileEntry> = adapter
            .crawl(&url, frontier_arc.clone(), app_handle.clone())
            .await
            .expect("crawl success");
        let crawl_secs = crawl_start.elapsed().as_secs_f64();

        let file_count = files
            .iter()
            .filter(|e| matches!(e.entry_type, EntryType::File))
            .count();
        let dir_count = files
            .iter()
            .filter(|e| matches!(e.entry_type, EntryType::Folder))
            .count();
        let hinted_bytes: u64 = files.iter().filter_map(|e| e.size_bytes).sum();

        println!(
            "[STEP] Crawl complete in {:.2}s | files={} dirs={} hinted_size={:.3}GB",
            crawl_secs,
            file_count,
            dir_count,
            bytes_to_gb(hinted_bytes)
        );

        println!("[STEP] Scaffolding folders and building batch list...");
        let mut batch_files: Vec<BatchFileEntry> = Vec::new();
        for entry in &files {
            let full_path = match path_utils::resolve_path_within_root(
                &output_root,
                &entry.path,
                matches!(entry.entry_type, EntryType::Folder),
            ) {
                Ok(Some(path)) => path,
                Ok(None) => continue,
                Err(err) => {
                    eprintln!("[WARN] rejected path '{}': {}", entry.path, err);
                    continue;
                }
            };

            match entry.entry_type {
                EntryType::Folder => {
                    std::fs::create_dir_all(&full_path).expect("create folder path");
                }
                EntryType::File => {
                    if let Some(parent) = full_path.parent() {
                        std::fs::create_dir_all(parent).expect("create parent path");
                    }
                    if entry.raw_url.starts_with("http://") || entry.raw_url.starts_with("https://")
                    {
                        batch_files.push(BatchFileEntry {
                            url: entry.raw_url.clone(),
                            path: full_path.to_string_lossy().to_string(),
                            size_hint: entry.size_bytes,
                        });
                    }
                }
            }
        }

        println!(
            "[STEP] Starting batch download with 120 circuits for {} files...",
            batch_files.len()
        );
        let download_start = Instant::now();
        let control =
            aria_downloader::activate_download_control().expect("activate download control");
        let download_result = aria_downloader::start_batch_download(
            app_handle.clone(),
            batch_files.clone(),
            120,
            true,
            Some(output_root.to_string_lossy().to_string()),
            control,
        )
        .await;
        aria_downloader::clear_download_control();

        stop_reporter.store(true, Ordering::Relaxed);
        let _ = reporter.await;

        app.unlisten(batch_listener_id);
        app.unlisten(log_listener_id);
        app.unlisten(crawl_log_listener_id);
        app.unlisten(tor_status_listener_id);

        if let Err(err) = download_result {
            eprintln!("[FAIL] download pipeline failed: {err}");
            std::process::exit(2);
        }

        let total_elapsed = whole_start.elapsed().as_secs_f64();
        let download_elapsed = download_start.elapsed().as_secs_f64();

        let done = completed.load(Ordering::Relaxed);
        let fail = failed.load(Ordering::Relaxed);
        let all = total.load(Ordering::Relaxed);
        let dl_bytes = downloaded_bytes.load(Ordering::Relaxed);
        let disk_bytes = dir_size_bytes(&output_root);
        let avg_mbps = if download_elapsed > 0.0 {
            (dl_bytes as f64 / download_elapsed) / 1_048_576.0
        } else {
            0.0
        };

        println!("\\n=== LOCKBIT LIVE SUMMARY ===");
        println!("Total elapsed: {:.2}s", total_elapsed);
        println!("Crawl elapsed: {:.2}s", crawl_secs);
        println!("Download elapsed: {:.2}s", download_elapsed);
        println!("Batch totals: total={} done={} failed={}", all, done, fail);
        println!("Transferred (event bytes): {:.3} GB", bytes_to_gb(dl_bytes));
        println!("On-disk size: {:.3} GB", bytes_to_gb(disk_bytes));
        println!("Average throughput: {:.2} MB/s", avg_mbps);
        println!("Output root: {}", output_root.display());

        if all == 0 || done == 0 || fail > 0 || disk_bytes == 0 {
            eprintln!(
                "[FAIL] Invalid completion state: total={} done={} fail={} disk_bytes={}",
                all, done, fail, disk_bytes
            );
            std::process::exit(3);
        }
    });
}
