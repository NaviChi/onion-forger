/// Comprehensive Multi-Adapter Benchmark Test
///
/// This test bootstraps a Tor swarm, loads the test database of URLs,
/// runs a 5-minute crawl benchmark for each adapter/URL, and produces
/// a CSV-like tabular output of results.
///
/// Run with:
///   cargo test --test adapter_benchmark_test -- --nocapture --ignored
///
/// Or for a specific adapter:
///   BENCHMARK_ADAPTER=dragonforce cargo test --test adapter_benchmark_test -- --nocapture --ignored

use crawli_lib::adapters::{AdapterRegistry, EntryType, SiteFingerprint};
use crawli_lib::frontier::{CrawlOptions, CrawlerFrontier};
use crawli_lib::telemetry_bridge;
use crawli_lib::{tor, AppState};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::Manager;

const BENCHMARK_DURATION_SECS: u64 = 300; // 5 minutes per adapter
const TOR_DAEMONS: usize = 4;
const CIRCUIT_COUNT: usize = 120;

fn now_epoch_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[derive(Debug, Clone, serde::Deserialize)]
struct TestUrl {
    id: String,
    adapter: String,
    name: String,
    url: String,
    expected_adapter: String,
    notes: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
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
            "TEST_ID",
            "ADAPTER",
            "URL",
            "STATUS",
            "MATCHED_ADAPTER",
            "FILES",
            "FOLDERS",
            "ENTRIES",
            "SIZE_BYTES",
            "DURATION",
            "ENTRIES/s",
            "BYTES/s",
            "TOR_BOOT",
            "FP_SECS",
            "ERROR",
            "DIAGNOSIS"
        )
    }

    fn to_row(&self) -> String {
        let url_display = if self.url.len() > 52 {
            format!("{}...", &self.url[..49])
        } else {
            self.url.clone()
        };

        format!(
            "{:<18} {:<35} {:<55} {:<12} {:<35} {:>8} {:>8} {:>8} {:>14} {:>10.2} {:>12.2} {:>12.2} {:>10.2} {:>10.2} {:<30} {:<60}",
            self.test_id,
            self.adapter_name,
            url_display,
            self.status,
            self.matched_adapter,
            self.total_files,
            self.total_folders,
            self.total_entries,
            self.total_size_bytes,
            self.crawl_duration_secs,
            self.entries_per_second,
            self.bytes_per_second,
            self.tor_bootstrap_secs,
            self.fingerprint_secs,
            if self.error_message.len() > 28 { format!("{}...", &self.error_message[..25]) } else { self.error_message.clone() },
            if self.diagnosis.len() > 58 { format!("{}...", &self.diagnosis[..55]) } else { self.diagnosis.clone() },
        )
    }
}

fn load_test_database() -> TestDatabase {
    let db_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("benchmark_test_db.json");
    let data = std::fs::read_to_string(&db_path)
        .unwrap_or_else(|e| panic!("Failed to read test database at {:?}: {}", db_path, e));
    serde_json::from_str(&data)
        .unwrap_or_else(|e| panic!("Failed to parse test database: {}", e))
}

fn diagnose_result(result: &BenchmarkResult) -> String {
    if result.status == "ERROR" {
        if result.error_message.contains("timeout")
            || result.error_message.contains("timed out")
        {
            return "NETWORK: Tor circuit timeout — site may be down or Tor network congested".to_string();
        }
        if result.error_message.contains("hidden service") {
            return "NETWORK: Hidden service descriptor not found — .onion may be offline".to_string();
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

#[test]
#[ignore = "live network benchmark; run with: cargo test --test adapter_benchmark_test -- --nocapture --ignored"]
fn multi_adapter_benchmark() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build tokio runtime");

    rt.block_on(async {
        // Load test database
        let db = load_test_database();
        println!("\n{}", "=".repeat(80));
        println!("  CRAWLI MULTI-ADAPTER BENCHMARK");
        println!("  Database: {} URLs across {} adapters", db.urls.len(), {
            let mut adapters: Vec<_> = db.urls.iter().map(|u| u.adapter.as_str()).collect();
            adapters.sort();
            adapters.dedup();
            adapters.len()
        });
        println!("  Duration: {}s per adapter", BENCHMARK_DURATION_SECS);
        println!("  Circuits: {} across {} daemons", CIRCUIT_COUNT, TOR_DAEMONS);
        println!("{}\n", "=".repeat(80));

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

        // Bootstrap Tauri test app
        let app = tauri::Builder::default()
            .manage(AppState::default())
            .build(tauri::generate_context!())
            .expect("build tauri app");
        let app_handle = app.handle().clone();
        let bridge = app.state::<AppState>().telemetry_bridge.clone();
        telemetry_bridge::spawn_bridge_emitter(app.handle().clone(), bridge);

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
            println!("\n{}", "─".repeat(70));
            println!(
                "  BENCHMARK {}/{}: {} ({})",
                idx + 1,
                test_urls.len(),
                test_url.name,
                test_url.adapter
            );
            println!("  URL: {}", test_url.url);
            println!("{}", "─".repeat(70));

            let result = run_single_benchmark(
                test_url,
                &app_handle,
                &active_ports,
                &arti_clients,
                tor_bootstrap_secs,
            )
            .await;

            println!(
                "\n  Result: {} | {} entries ({} files, {} folders) in {:.2}s | {:.2} entries/s",
                result.status,
                result.total_entries,
                result.total_files,
                result.total_folders,
                result.crawl_duration_secs,
                result.entries_per_second,
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
        let total_success = all_results
            .iter()
            .filter(|r| r.status == "OK" || r.status == "PARTIAL")
            .count();
        let total_error = all_results
            .iter()
            .filter(|r| r.status == "ERROR" || r.status == "ZERO")
            .count();
        let avg_eps: f64 = if !all_results.is_empty() {
            all_results.iter().map(|r| r.entries_per_second).sum::<f64>()
                / all_results.len() as f64
        } else {
            0.0
        };

        println!("\n  AGGREGATE:");
        println!(
            "  Total URLs tested: {} | Success: {} | Error/Zero: {} | Total entries: {} | Avg entries/s: {:.2}",
            all_results.len(), total_success, total_error, total_entries, avg_eps
        );
        println!("  Tor bootstrap: {:.2}s | Total benchmark time: {:.2}s",
            tor_bootstrap_secs,
            all_results.iter().map(|r| r.crawl_duration_secs + r.fingerprint_secs).sum::<f64>()
        );
        println!("{}\n", "=".repeat(200));

        // Generate CSV output for programmatic consumption
        let csv_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("benchmark_results.csv");
        let mut csv = String::new();
        csv.push_str("test_id,adapter,url,status,matched_adapter,files,folders,entries,size_bytes,duration_secs,entries_per_sec,bytes_per_sec,tor_boot_secs,fp_secs,error,diagnosis\n");
        for r in &all_results {
            csv.push_str(&format!(
                "{},{},{},{},{},{},{},{},{},{:.2},{:.2},{:.2},{:.2},{:.2},{},{}\n",
                r.test_id,
                r.adapter_name,
                r.url.replace(',', "%2C"),
                r.status,
                r.matched_adapter,
                r.total_files,
                r.total_folders,
                r.total_entries,
                r.total_size_bytes,
                r.crawl_duration_secs,
                r.entries_per_second,
                r.bytes_per_second,
                r.tor_bootstrap_secs,
                r.fingerprint_secs,
                r.error_message.replace(',', ";"),
                r.diagnosis.replace(',', ";"),
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
    };

    let daemon_count = active_ports.len().max(1);
    let frontier = CrawlerFrontier::new(
        Some(app_handle.clone()),
        test_url.url.clone(),
        daemon_count,
        true,
        active_ports.to_vec(),
        arti_clients.to_vec(),
        options,
    );

    // Phase 1: Fingerprint the target
    println!("  [FP] Fetching fingerprint for {}...", test_url.url);
    let fp_start = Instant::now();
    let fp_timeout = Duration::from_secs(60);

    let fingerprint_result = tokio::time::timeout(fp_timeout, async {
        let (_, _client) = frontier.get_client();

        // Try up to 3 times with different circuits
        for attempt in 1..=3 {
            let (retry_cid, retry_client) = if attempt == 1 {
                let (c, cl) = frontier.get_client();
                (c, cl)
            } else {
                println!("  [FP] Retry #{} with fresh circuit...", attempt);
                frontier.get_client()
            };

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
                            return Ok(SiteFingerprint {
                                url: test_url.url.clone(),
                                status,
                                headers: convert_headers(&headers),
                                body,
                            });
                        }
                        Ok(Err(e)) => {
                            println!("  [FP] Body read error: {}", e);
                        }
                        Err(_) => {
                            println!("  [FP] Body read timeout");
                        }
                    }
                }
                Ok(Err(e)) => {
                    println!("  [FP] Request error: {}", e);
                }
                Err(_) => {
                    println!("  [FP] Request timeout (30s)");
                }
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
            result.error_message =
                format!("Fingerprint timed out after {}s", fp_timeout.as_secs());
            result.fingerprint_secs = fp_start.elapsed().as_secs_f64();
            result.diagnosis = diagnose_result(&result);
            return result;
        }
    };

    // Phase 2: Determine adapter
    let registry = AdapterRegistry::new();
    let adapter = registry.determine_adapter(&fingerprint).await;

    match adapter {
        Some(adapter) => {
            result.matched_adapter = adapter.name().to_string();
            println!("  [ADAPT] Matched adapter: {}", adapter.name());
        }
        None => {
            // Fallback: try using known domain fast-path to force adapter selection
            result.status = "ERROR".to_string();
            result.error_message = "No adapter matched fingerprint".to_string();
            result.diagnosis = diagnose_result(&result);
            return result;
        }
    }

    let adapter = registry.determine_adapter(&fingerprint).await.unwrap();

    // Phase 3: Run the crawl with time limit
    println!(
        "  [CRAWL] Starting {}-second benchmark crawl...",
        BENCHMARK_DURATION_SECS
    );
    let crawl_start = Instant::now();
    let frontier_arc = Arc::new(frontier);

    let crawl_result = tokio::time::timeout(
        Duration::from_secs(BENCHMARK_DURATION_SECS),
        adapter.crawl(&test_url.url, frontier_arc.clone(), app_handle.clone()),
    )
    .await;

    result.crawl_duration_secs = crawl_start.elapsed().as_secs_f64();

    match crawl_result {
        Ok(Ok(files)) => {
            result.total_files = files
                .iter()
                .filter(|e| matches!(e.entry_type, EntryType::File))
                .count();
            result.total_folders = files
                .iter()
                .filter(|e| matches!(e.entry_type, EntryType::Folder))
                .count();
            result.total_entries = files.len();
            result.total_size_bytes = files.iter().filter_map(|e| e.size_bytes).sum();

            if result.total_entries > 0 {
                result.status = "OK".to_string();
            } else {
                result.status = "ZERO".to_string();
            }

            result.entries_per_second = if result.crawl_duration_secs > 0.0 {
                result.total_entries as f64 / result.crawl_duration_secs
            } else {
                0.0
            };

            result.bytes_per_second = if result.crawl_duration_secs > 0.0 {
                result.total_size_bytes as f64 / result.crawl_duration_secs
            } else {
                0.0
            };

            println!(
                "  [CRAWL] Complete: {} entries ({} files, {} folders) in {:.2}s",
                result.total_entries,
                result.total_files,
                result.total_folders,
                result.crawl_duration_secs,
            );
        }
        Ok(Err(e)) => {
            result.status = "ERROR".to_string();
            result.error_message = format!("Crawl error: {}", e);
        }
        Err(_) => {
            // Timeout — check frontier stats
            let visited = frontier_arc.visited_count();
            let processed = frontier_arc.processed_count();

            if visited > 0 || processed > 0 {
                result.status = "PARTIAL".to_string();
                result.total_entries = visited;
                result.error_message = format!(
                    "Hit {}s limit — visited={} processed={}",
                    BENCHMARK_DURATION_SECS, visited, processed
                );
            } else {
                result.status = "TIMEOUT".to_string();
                result.error_message = format!(
                    "Crawl timed out after {}s with no results",
                    BENCHMARK_DURATION_SECS
                );
            }
        }
    }

    // Cancel frontier to clean up any background workers
    frontier_arc.cancel();

    result.diagnosis = diagnose_result(&result);
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
