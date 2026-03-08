/// Comprehensive Multi-Adapter Benchmark Binary
///
/// This binary bootstraps a Tor swarm, loads the test database of URLs,
/// runs a 5-minute crawl benchmark for each adapter/URL, and produces
/// a CSV-like tabular output of results.
///
/// Run with:
///   cargo run --bin adapter-benchmark
///
/// For a specific adapter:
///   BENCHMARK_ADAPTER=dragonforce cargo run --bin adapter-benchmark
///
/// Custom duration (seconds):
///   BENCHMARK_DURATION=60 cargo run --bin adapter-benchmark
use crawli_lib::adapters::{AdapterRegistry, SiteFingerprint};
use crawli_lib::frontier::{CrawlOptions, CrawlerFrontier};
use crawli_lib::telemetry_bridge;
use crawli_lib::{tor, AppState};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{Listener, Manager};

fn default_benchmark_duration() -> u64 {
    std::env::var("BENCHMARK_DURATION")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(300) // 5 minutes default
}

const TOR_DAEMONS: usize = 1;
const CIRCUIT_COUNT: usize = 60;

#[derive(Debug, Clone, serde::Deserialize)]
#[allow(dead_code)]
struct TestUrl {
    id: String,
    adapter: String,
    name: String,
    url: String,
    expected_adapter: String,
    notes: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[allow(dead_code)]
struct TestDatabase {
    version: u32,
    description: String,
    urls: Vec<TestUrl>,
}

#[derive(Debug, Clone)]
struct BenchmarkResult {
    test_id: String,
    adapter_name: String,
    url: String,
    status: String,
    matched_adapter: String,
    total_files: usize,
    total_folders: usize,
    total_entries: usize,
    total_size_bytes: u64,
    crawl_duration_secs: f64,
    entries_per_second: f64,
    bytes_per_second: f64,
    tor_bootstrap_secs: f64,
    fingerprint_secs: f64,
    error_message: String,
    diagnosis: String,
}

impl BenchmarkResult {
    fn header() -> String {
        format!(
            "{:<18} {:<35} {:<55} {:<12} {:<35} {:>8} {:>8} {:>8} {:>14} {:>10} {:>12} {:>12} {:>10} {:>10} {:<30} {:<60}",
            "TEST_ID", "ADAPTER", "URL", "STATUS", "MATCHED_ADAPTER",
            "FILES", "FOLDERS", "ENTRIES", "SIZE_BYTES",
            "DURATION", "ENTRIES/s", "BYTES/s",
            "TOR_BOOT", "FP_SECS", "ERROR", "DIAGNOSIS"
        )
    }

    fn to_row(&self) -> String {
        let url_display = if self.url.len() > 52 {
            format!("{}...", &self.url[..49])
        } else {
            self.url.clone()
        };
        let error_display = if self.error_message.len() > 28 {
            format!("{}...", &self.error_message[..25])
        } else {
            self.error_message.clone()
        };
        let diag_display = if self.diagnosis.len() > 58 {
            format!("{}...", &self.diagnosis[..55])
        } else {
            self.diagnosis.clone()
        };

        format!(
            "{:<18} {:<35} {:<55} {:<12} {:<35} {:>8} {:>8} {:>8} {:>14} {:>10.2} {:>12.2} {:>12.2} {:>10.2} {:>10.2} {:<30} {:<60}",
            self.test_id, self.adapter_name, url_display,
            self.status, self.matched_adapter,
            self.total_files, self.total_folders, self.total_entries, self.total_size_bytes,
            self.crawl_duration_secs, self.entries_per_second, self.bytes_per_second,
            self.tor_bootstrap_secs, self.fingerprint_secs,
            error_display, diag_display,
        )
    }
}

fn load_test_database() -> TestDatabase {
    let db_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("benchmark_test_db.json");
    let data = std::fs::read_to_string(&db_path)
        .unwrap_or_else(|e| panic!("Failed to read test database at {:?}: {}", db_path, e));
    serde_json::from_str(&data).unwrap_or_else(|e| panic!("Failed to parse test database: {}", e))
}

fn diagnose_result(result: &BenchmarkResult) -> String {
    if result.status == "ERROR" {
        if result.error_message.contains("timeout") || result.error_message.contains("timed out") {
            return "NETWORK: Tor circuit timeout — site may be down or Tor network congested"
                .to_string();
        }
        if result.error_message.contains("hidden service") {
            return "NETWORK: Hidden service descriptor not found — .onion may be offline"
                .to_string();
        }
        if result.error_message.contains("connection refused")
            || result.error_message.contains("reset")
        {
            return "SERVER: Connection refused/reset — target server is likely down".to_string();
        }
        return format!("UNKNOWN: {}", result.error_message);
    }
    if result.total_entries == 0 {
        if result.crawl_duration_secs < 5.0 {
            return "ADAPTER: Crawl completed too fast with 0 results — possible parsing failure or empty listing".to_string();
        }
        if result.matched_adapter.is_empty() || result.matched_adapter == "None" {
            return "ADAPTER: No adapter matched fingerprint — need new adapter or site format changed".to_string();
        }
        return "ADAPTER: Adapter matched but 0 entries — possible HTML structure change or empty data set".to_string();
    }
    if result.total_files == 0 && result.total_folders > 0 {
        return "PARTIAL: Only folders discovered (no files) — deeper traversal needed or files not exposed".to_string();
    }
    if result.entries_per_second < 0.1 {
        return "SLOW: Very low throughput — possible throttling or Tor congestion".to_string();
    }
    "OK: Benchmark completed successfully".to_string()
}

fn main() {
    let benchmark_duration = default_benchmark_duration();

    // Build Tauri app on the main thread FIRST (macOS EventLoop requirement)
    let app = tauri::Builder::default()
        .manage(AppState::default())
        .build(tauri::generate_context!())
        .expect("build tauri app");
    let app_handle = app.handle().clone();
    let bridge = app.state::<AppState>().telemetry_bridge.clone();

    // Now use a multi-threaded runtime for async work (required by frontier's block_in_place)
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(8)
        .build()
        .expect("build tokio runtime");

    rt.block_on(async {
        telemetry_bridge::spawn_bridge_emitter(app_handle.clone(), bridge);

        let db = load_test_database();
        println!("\n{}", "=".repeat(120));
        println!("  CRAWLI MULTI-ADAPTER BENCHMARK");
        println!("  Database: {} URLs across {} adapters", db.urls.len(), {
            let mut adapters: Vec<_> = db.urls.iter().map(|u| u.adapter.as_str()).collect();
            adapters.sort();
            adapters.dedup();
            adapters.len()
        });
        println!("  Duration: {}s per adapter", benchmark_duration);
        println!("  Circuits: {} across {} daemons", CIRCUIT_COUNT, TOR_DAEMONS);
        println!("{}\n", "=".repeat(120));

        // Optional filter: only run specific adapter
        let filter = std::env::var("BENCHMARK_ADAPTER").ok();
        let test_urls: Vec<_> = if let Some(ref f) = filter {
            println!("[FILTER] Only testing adapter: {}\n", f);
            db.urls.iter().filter(|u| u.adapter == *f).cloned().collect()
        } else {
            db.urls.clone()
        };

        if test_urls.is_empty() {
            println!("[ERROR] No URLs to test!");
            return;
        }

        // Bootstrap Tor swarm
        println!("[STEP] Cleaning stale Tor daemons...");
        tor::cleanup_stale_tor_daemons();

        println!("[STEP] Bootstrapping {}-daemon Tor swarm...", TOR_DAEMONS);
        let tor_start = Instant::now();
        let (guard, active_ports) = tor::bootstrap_tor_cluster(app_handle.clone(), TOR_DAEMONS)
            .await
            .expect("bootstrap tor cluster");
        let tor_bootstrap_secs = tor_start.elapsed().as_secs_f64();
        println!(
            "[STEP] Tor ready in {:.2}s on ports {:?}\n",
            tor_bootstrap_secs, active_ports
        );

        let arti_clients = guard.get_arti_clients();
        let mut all_results: Vec<BenchmarkResult> = Vec::new();

        // Run benchmark for each URL
        for (idx, test_url) in test_urls.iter().enumerate() {
            println!("\n{}", "─".repeat(100));
            println!(
                "  BENCHMARK {}/{}: {} ({})",
                idx + 1, test_urls.len(), test_url.name, test_url.adapter
            );
            println!("  URL: {}", test_url.url);
            println!("{}", "─".repeat(100));

            let result = run_single_benchmark(
                test_url,
                &app_handle,
                &active_ports,
                &arti_clients,
                tor_bootstrap_secs,
                benchmark_duration,
            )
            .await;

            println!(
                "\n  Result: {} | {} entries ({} files, {} folders) in {:.2}s | {:.2} entries/s",
                result.status, result.total_entries, result.total_files,
                result.total_folders, result.crawl_duration_secs, result.entries_per_second,
            );
            println!("  Diagnosis: {}", result.diagnosis);

            all_results.push(result);
        }

        // Print final summary table
        println!("\n\n{}", "=".repeat(200));
        println!("  BENCHMARK RESULTS SUMMARY");
        println!("{}", "=".repeat(200));
        println!("{}", BenchmarkResult::header());
        println!("{}", "─".repeat(200));
        for result in &all_results {
            println!("{}", result.to_row());
        }
        println!("{}", "─".repeat(200));

        // Aggregate stats
        let total_entries: usize = all_results.iter().map(|r| r.total_entries).sum();
        let total_success = all_results.iter().filter(|r| r.status == "OK" || r.status == "PARTIAL").count();
        let total_error = all_results.iter().filter(|r| r.status == "ERROR" || r.status == "ZERO").count();
        let avg_eps: f64 = if !all_results.is_empty() {
            all_results.iter().map(|r| r.entries_per_second).sum::<f64>() / all_results.len() as f64
        } else {
            0.0
        };

        println!("\n  AGGREGATE:");
        println!(
            "  Total URLs tested: {} | Success: {} | Error/Zero: {} | Total entries: {} | Avg entries/s: {:.2}",
            all_results.len(), total_success, total_error, total_entries, avg_eps
        );
        println!(
            "  Tor bootstrap: {:.2}s | Total benchmark time: {:.2}s",
            tor_bootstrap_secs,
            all_results.iter().map(|r| r.crawl_duration_secs + r.fingerprint_secs).sum::<f64>()
        );
        println!("{}\n", "=".repeat(200));

        // CSV output
        let csv_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("benchmark_results.csv");
        let mut csv = String::new();
        csv.push_str("test_id,adapter,url,status,matched_adapter,files,folders,entries,size_bytes,duration_secs,entries_per_sec,bytes_per_sec,tor_boot_secs,fp_secs,error,diagnosis\n");
        for r in &all_results {
            csv.push_str(&format!(
                "{},{},{},{},{},{},{},{},{},{:.2},{:.2},{:.2},{:.2},{:.2},{},{}\n",
                r.test_id, r.adapter_name,
                r.url.replace(',', "%2C"), r.status, r.matched_adapter,
                r.total_files, r.total_folders, r.total_entries, r.total_size_bytes,
                r.crawl_duration_secs, r.entries_per_second, r.bytes_per_second,
                r.tor_bootstrap_secs, r.fingerprint_secs,
                r.error_message.replace(',', ";"), r.diagnosis.replace(',', ";"),
            ));
        }
        let _ = std::fs::write(&csv_path, &csv);
        println!("[OUTPUT] CSV results written to: {}", csv_path.display());
    });
}

async fn run_single_benchmark(
    test_url: &TestUrl,
    app_handle: &tauri::AppHandle,
    active_ports: &[u16],
    arti_clients: &[crawli_lib::tor_native::SharedTorClient],
    tor_bootstrap_secs: f64,
    benchmark_duration: u64,
) -> BenchmarkResult {
    let mut result = BenchmarkResult {
        test_id: test_url.id.clone(),
        adapter_name: test_url.adapter.clone(),
        url: test_url.url.clone(),
        status: "PENDING".to_string(),
        matched_adapter: String::new(),
        total_files: 0,
        total_folders: 0,
        total_entries: 0,
        total_size_bytes: 0,
        crawl_duration_secs: 0.0,
        entries_per_second: 0.0,
        bytes_per_second: 0.0,
        tor_bootstrap_secs,
        fingerprint_secs: 0.0,
        error_message: String::new(),
        diagnosis: String::new(),
    };

    let options = CrawlOptions {
        listing: true,
        sizes: true,
        download: false,
        circuits: Some(CIRCUIT_COUNT),
        daemons: Some(TOR_DAEMONS),
        agnostic_state: false,
        resume: false,
        resume_index: None,
        mega_password: None,
    };

    let daemon_count = active_ports.len().max(1);
    let frontier = CrawlerFrontier::new(
        None, // No app_handle for ephemeral benchmarks
        test_url.url.clone(),
        daemon_count,
        true, // is_onion
        active_ports.to_vec(),
        arti_clients.to_vec(),
        options,
        None, // No TargetPaths persistence for ephemeral benchmarks
    );

    // Phase 1: Fingerprint the target
    println!("  [FP] Fetching fingerprint for {}...", test_url.url);
    let fp_start = Instant::now();
    let fp_timeout = Duration::from_secs(60);

    let fingerprint_result = tokio::time::timeout(fp_timeout, async {
        for attempt in 1..=3 {
            let (retry_cid, retry_client) = frontier.get_client();

            if attempt > 1 {
                println!("  [FP] Retry #{} with fresh circuit...", attempt);
            }

            match tokio::time::timeout(
                Duration::from_secs(30),
                retry_client.get(&test_url.url).send(),
            )
            .await
            {
                Ok(Ok(resp)) => {
                    let status = resp.status().as_u16();
                    let headers: http::HeaderMap = resp.headers().clone();
                    match tokio::time::timeout(Duration::from_secs(30), resp.text()).await {
                        Ok(Ok(body)) => {
                            frontier.record_success(
                                retry_cid,
                                body.len() as u64,
                                fp_start.elapsed().as_millis() as u64,
                            );
                            return Ok(SiteFingerprint {
                                url: test_url.url.clone(),
                                status,
                                headers: convert_headers(&headers),
                                body,
                            });
                        }
                        Ok(Err(e)) => println!("  [FP] Body read error: {}", e),
                        Err(_) => println!("  [FP] Body read timeout"),
                    }
                }
                Ok(Err(e)) => println!("  [FP] Request error: {}", e),
                Err(_) => println!("  [FP] Request timeout (30s)"),
            }
            frontier.record_failure(retry_cid);
        }
        Err(anyhow::anyhow!("All 3 fingerprint attempts failed"))
    })
    .await;

    let fingerprint = match fingerprint_result {
        Ok(Ok(fp)) => {
            result.fingerprint_secs = fp_start.elapsed().as_secs_f64();
            println!(
                "  [FP] Fingerprint obtained in {:.2}s (status={}, body_len={})",
                result.fingerprint_secs,
                fp.status,
                fp.body.len()
            );
            fp
        }
        Ok(Err(e)) => {
            result.status = "ERROR".to_string();
            result.error_message = format!("Fingerprint failed: {}", e);
            result.fingerprint_secs = fp_start.elapsed().as_secs_f64();
            result.diagnosis = diagnose_result(&result);
            return result;
        }
        Err(_) => {
            result.status = "ERROR".to_string();
            result.error_message = format!("Fingerprint timed out after {}s", fp_timeout.as_secs());
            result.fingerprint_secs = fp_start.elapsed().as_secs_f64();
            result.diagnosis = diagnose_result(&result);
            return result;
        }
    };

    // Phase 2: Determine adapter
    let registry = AdapterRegistry::new();
    let adapter = match registry.determine_adapter(&fingerprint).await {
        Some(a) => {
            result.matched_adapter = a.name().to_string();
            println!("  [ADAPT] Matched adapter: {}", a.name());
            a
        }
        None => {
            result.status = "ERROR".to_string();
            result.error_message = "No adapter matched fingerprint".to_string();
            result.diagnosis = diagnose_result(&result);
            return result;
        }
    };

    // Phase 3: Run the crawl in the background
    println!("  [CRAWL] Waiting for first file discovery...");
    let frontier_arc = Arc::new(frontier);
    let frontier_clone_for_adapter = frontier_arc.clone();
    let app_handle_clone = app_handle.clone();
    let target_url_clone = test_url.url.clone();

    let total_files_discovered = Arc::new(AtomicUsize::new(0));
    let total_files_clone = total_files_discovered.clone();

    // Listen to the Tauri event to count files correctly before they get lost in the batch
    let event_id = app_handle.listen_any("crawl_progress", move |event: tauri::Event| {
        let payload = event.payload();
        // Count occurrences of "entryType":"File" as a fast proxy for parsing the JSON
        let count = payload.matches("\"entryType\":\"File\"").count();
        // Also count folders just so we see something if there are no files immediately
        let folder_count = payload.matches("\"entryType\":\"Folder\"").count();
        total_files_clone.fetch_add(count + folder_count, Ordering::Relaxed);
    });

    // The supervisor future
    let supervisor_future = async {
        let supervisor_timeout = Duration::from_secs(benchmark_duration);
        let mut stats_map = std::collections::BTreeMap::new();
        let mut files_at_60s = 0;

        let start_time = Instant::now();
        let total_files_for_stats = total_files_discovered.clone();

        println!(
            "  [STATS] >>> CRAWL PHASE STARTED (Global time remaining: {:.1}s) <<<",
            benchmark_duration as f64 - tor_bootstrap_secs - result.fingerprint_secs
        );

        let _ = tokio::time::timeout(supervisor_timeout, async {
            let mut target_intervals =
                vec![30, 60, 90, 120, 150, 180, 210, 240, 270, 300, 330, 360, 390];

            loop {
                let current_count = total_files_for_stats.load(Ordering::Relaxed);
                let elapsed_secs = start_time.elapsed().as_secs();
                let total_global_secs =
                    tor_bootstrap_secs as u64 + result.fingerprint_secs as u64 + elapsed_secs;

                if let Some(&next_target) = target_intervals.first() {
                    if elapsed_secs >= next_target {
                        println!(
                            "  [STATS] at {}s in crawl ({}s total) -> {} entries discovered",
                            next_target, total_global_secs, current_count
                        );
                        stats_map.insert(next_target, current_count);
                        if next_target == 60 {
                            files_at_60s = current_count;
                        }
                        target_intervals.remove(0);
                    }
                }

                if total_global_secs >= benchmark_duration {
                    println!(
                        "  [STATS] >>> {} SECOND EXPERIMENT LIMIT REACHED. Terminating. <<<",
                        benchmark_duration
                    );
                    frontier_arc.cancel();
                    break;
                }
                tokio::time::sleep(tokio::time::Duration::from_millis(250)).await;
            }
        })
        .await;

        let final_elapsed = start_time.elapsed().as_secs_f64();
        (
            stats_map,
            files_at_60s,
            total_files_for_stats.load(Ordering::Relaxed),
            final_elapsed,
        )
    };

    let crawl_future = adapter.crawl(
        &target_url_clone,
        frontier_clone_for_adapter,
        app_handle_clone,
    );

    let (_crawl_res, (stats_map, files_at_60s, final_files, crawl_duration_secs)) =
        tokio::join!(crawl_future, supervisor_future);

    app_handle.unlisten(event_id);

    // Final stats calc
    let files_per_second_total = if crawl_duration_secs > 0.0 {
        final_files as f64 / crawl_duration_secs
    } else {
        0.0
    };

    println!("\n{}", "=".repeat(100));
    println!(
        "  FINAL CRAWL STATS ({:.1} seconds active crawl)",
        crawl_duration_secs
    );
    println!("{}", "=".repeat(100));
    println!(
        "  Files per second (avg over 300s): {:.2} files/sec",
        files_per_second_total
    );
    println!("  First 60s files: {}", files_at_60s);
    for interval in vec![60, 90, 120, 150, 180, 210, 240, 270, 300] {
        if let Some(count) = stats_map.get(&interval) {
            println!("  Files up to {}s: {}", interval, count);
        }
    }

    result.total_entries = final_files;
    result.crawl_duration_secs = 300.0;
    result.entries_per_second = files_per_second_total;
    result.status = "OK".to_string();
    result.diagnosis = "Crawl monitored manually".to_string();
    result
}

/// Convert http::HeaderMap to reqwest::header::HeaderMap
fn convert_headers(src: &http::HeaderMap) -> reqwest::header::HeaderMap {
    let mut dst = reqwest::header::HeaderMap::new();
    for (key, value) in src.iter() {
        if let Ok(key) = reqwest::header::HeaderName::from_bytes(key.as_str().as_bytes()) {
            if let Ok(value) = reqwest::header::HeaderValue::from_bytes(value.as_bytes()) {
                dst.insert(key, value);
            }
        }
    }
    dst
}
