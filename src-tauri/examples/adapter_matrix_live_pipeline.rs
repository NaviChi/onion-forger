use crawli_lib::adapters::{self, AdapterRegistry, EntryType, FileEntry, SiteFingerprint};
use crawli_lib::aria_downloader::{self, BatchFileEntry};
use crawli_lib::frontier::{CrawlOptions, CrawlerFrontier};
use crawli_lib::telemetry_bridge::{self, TelemetryBridgeUpdate};
use crawli_lib::{path_utils, tor, AppState};
use reqwest::header::HeaderMap;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{Event, Listener, Manager};

const MAX_ADAPTERS: usize = 10;
const CRAWL_TIMEOUT_SECS: u64 = 600;
const DOWNLOAD_TIMEOUT_SECS: u64 = 600;
const UNKNOWN_FILE_BUDGET_BYTES: u64 = 10 * 1_048_576; // 10MB per unknown-size file
const SAFETY_RESERVE_BYTES: u64 = 10 * 1_073_741_824; // keep at least 10GB free

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "snake_case")]
struct AdapterRunResult {
    adapter_id: String,
    adapter_name: String,
    url: String,
    status: String,
    reason: String,
    detected_adapter: String,
    crawl_secs: f64,
    download_secs: f64,
    file_count: usize,
    dir_count: usize,
    unknown_size_files: usize,
    hinted_bytes: u64,
    est_required_bytes: u64,
    free_before_bytes: u64,
    free_before_download_bytes: u64,
    downloaded_event_bytes: u64,
    disk_bytes: u64,
    batch_total: usize,
    batch_done: usize,
    batch_failed: usize,
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug)]
struct SignatureMap {
    files: usize,
    dirs: usize,
}

fn now_epoch_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn bytes_to_gb(bytes: u64) -> f64 {
    bytes as f64 / 1_073_741_824.0
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

fn available_bytes(path: &Path) -> Option<u64> {
    let output = std::process::Command::new("df")
        .arg("-k")
        .arg(path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let line = text.lines().nth(1)?;
    let cols: Vec<&str> = line.split_whitespace().collect();
    if cols.len() < 4 {
        return None;
    }
    let avail_kb = cols[3].parse::<u64>().ok()?;
    Some(avail_kb.saturating_mul(1024))
}

fn normalize_url(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return trimmed.to_string();
    }
    if trimmed.contains(".onion") {
        return format!("http://{}", trimmed);
    }
    trimmed.to_string()
}

fn priority_url_key(url: &str) -> usize {
    let lower = url.to_lowercase();
    if lower.contains("unknown.onion") {
        return 9;
    }
    if lower.contains("worldleaks.onion")
        || lower.contains("dragonforce.onion")
        || lower.contains("lockbit.onion")
        || lower.contains("nu-server.onion")
    {
        return 5;
    }
    0
}

fn safe_remove_dir(path: &Path) {
    if path.exists() {
        let _ = std::fs::remove_dir_all(path);
    }
}

fn build_batch_files(entries: &[FileEntry], output_root: &Path) -> Vec<BatchFileEntry> {
    let mut batch_files: Vec<BatchFileEntry> = Vec::new();

    for entry in entries {
        let full_path = match path_utils::resolve_path_within_root(
            output_root,
            &entry.path,
            matches!(entry.entry_type, EntryType::Folder),
        ) {
            Ok(Some(path)) => path,
            _ => continue,
        };

        match entry.entry_type {
            EntryType::Folder => {
                let _ = std::fs::create_dir_all(&full_path);
            }
            EntryType::File => {
                if let Some(parent) = full_path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                if entry.raw_url.starts_with("http://") || entry.raw_url.starts_with("https://") {
                    batch_files.push(BatchFileEntry {
                        url: entry.raw_url.clone(), // Assuming 'url' in instruction was a typo and meant entry.raw_url
                        alternate_urls: Vec::new(),
                        path: full_path.to_string_lossy().to_string(),
                        size_hint: entry.size_bytes,
                        jwt_exp: entry.jwt_exp,
                    });
                }
            }
        }
    }

    batch_files
}

async fn run_single_adapter(
    app: &tauri::AppHandle,
    adapter: &adapters::AdapterSupportInfo,
    url: &str,
    output_root_raw: &Path,
) -> AdapterRunResult {
    let started = Instant::now();
    let fs_probe_path = output_root_raw
        .parent()
        .unwrap_or(output_root_raw)
        .to_path_buf();
    let mut result = AdapterRunResult {
        adapter_id: adapter.id.to_string(),
        adapter_name: adapter.name.to_string(),
        url: url.to_string(),
        status: "fail".to_string(),
        reason: "uninitialized".to_string(),
        detected_adapter: "Unidentified".to_string(),
        crawl_secs: 0.0,
        download_secs: 0.0,
        file_count: 0,
        dir_count: 0,
        unknown_size_files: 0,
        hinted_bytes: 0,
        est_required_bytes: 0,
        free_before_bytes: available_bytes(&fs_probe_path).unwrap_or(0),
        free_before_download_bytes: 0,
        downloaded_event_bytes: 0,
        disk_bytes: 0,
        batch_total: 0,
        batch_done: 0,
        batch_failed: 0,
    };

    println!(
        "\n[ADAPTER] {} ({})\n[URL] {}",
        adapter.name, adapter.id, url
    );
    println!(
        "[SPACE] free before run: {:.2} GB",
        bytes_to_gb(result.free_before_bytes)
    );

    safe_remove_dir(output_root_raw);
    if let Err(err) = std::fs::create_dir_all(output_root_raw) {
        result.reason = format!("failed to create output dir: {}", err);
        return result;
    }
    let canonical_output_root =
        match path_utils::canonicalize_output_root(&output_root_raw.to_string_lossy()) {
            Ok(p) => p,
            Err(err) => {
                result.reason = format!("failed to canonicalize output dir: {}", err);
                return result;
            }
        };
    let output_root_string = canonical_output_root.to_string_lossy().to_string();

    tor::cleanup_stale_tor_daemons();

    let is_onion = url.contains(".onion");
    let mut swarm_guard = None;
    let mut active_ports = Vec::new();
    if is_onion {
        match tor::bootstrap_tor_cluster(app.clone(), 8, 8).await {
            Ok((guard, ports)) => {
                swarm_guard = Some(Arc::new(tokio::sync::Mutex::new(guard)));
                active_ports = ports;
            }
            Err(err) => {
                result.reason = format!("tor bootstrap failed: {}", err);
                return result;
            }
        }
    }

    let options = CrawlOptions {
        listing: true,
        sizes: true,
        download: true,
        circuits: Some(250),
        resume: false,
        agnostic_state: false,
        resume_index: None,
        mega_password: None,
        stealth_ramp: true, parallel_download: false,
            force_clearnet: false,
    };
    let daemon_count = if is_onion {
        active_ports.len().max(1)
    } else {
        8
    };
    let arti_clients = if let Some(guard) = &swarm_guard {
        let locked = guard.lock().await;
        locked.get_arti_clients()
    } else {
        Vec::new()
    };
    let mut frontier = CrawlerFrontier::new(
        Some(app.clone()),
        url.to_string(),
        daemon_count,
        is_onion,
        Vec::new(), // explicit ports removed, handled internally
        arti_clients,
        options.clone(),
        None,
    );
    frontier.swarm_guard = swarm_guard;
    let frontier_arc = Arc::new(frontier);

    let fingerprint = {
        let mut final_resp = None;
        for _ in 0..4 {
            let client = frontier_arc.get_client().1;
            let fetched =
                tokio::time::timeout(Duration::from_secs(45), client.get(url).send()).await;
            if let Ok(Ok(resp)) = fetched {
                final_resp = Some(resp);
                break;
            }
        }

        let resp = match final_resp {
            Some(resp) => resp,
            None => {
                result.status = "skip".to_string();
                result.reason = "fingerprint fetch failed after 4 retries".to_string();
                return result;
            }
        };

        let status = resp.status().as_u16();
        let headers: HeaderMap = resp.headers().clone();
        let body = match tokio::time::timeout(Duration::from_secs(45), resp.text()).await {
            Ok(Ok(txt)) => txt,
            _ => "[DECODE_ERROR]".to_string(),
        };
        SiteFingerprint {
            url: url.to_string(),
            status,
            headers,
            body,
        }
    };

    let registry = AdapterRegistry::new();
    let detected = match registry.determine_adapter(&fingerprint).await {
        Some(adapter_impl) => adapter_impl,
        None => {
            result.status = "skip".to_string();
            result.reason = "no adapter matched fingerprint".to_string();
            return result;
        }
    };
    result.detected_adapter = detected.name().to_string();
    println!(
        "[DETECT] expected={} | detected={}",
        adapter.name, result.detected_adapter
    );

    let crawl_start = Instant::now();
    let entries = match tokio::time::timeout(
        Duration::from_secs(CRAWL_TIMEOUT_SECS),
        detected.crawl(url, frontier_arc.clone(), app.clone()),
    )
    .await
    {
        Ok(Ok(entries)) => entries,
        Ok(Err(err)) => {
            result.reason = format!("crawl failed: {}", err);
            return result;
        }
        Err(_) => {
            result.reason = format!("crawl timeout ({}s)", CRAWL_TIMEOUT_SECS);
            return result;
        }
    };
    result.crawl_secs = crawl_start.elapsed().as_secs_f64();

    result.file_count = entries
        .iter()
        .filter(|e| matches!(e.entry_type, EntryType::File))
        .count();
    result.dir_count = entries
        .iter()
        .filter(|e| matches!(e.entry_type, EntryType::Folder))
        .count();
    result.hinted_bytes = entries.iter().filter_map(|e| e.size_bytes).sum::<u64>();
    result.unknown_size_files = entries
        .iter()
        .filter(|e| matches!(e.entry_type, EntryType::File) && e.size_bytes.unwrap_or(0) == 0)
        .count();
    result.est_required_bytes = result
        .hinted_bytes
        .saturating_add(result.unknown_size_files as u64 * UNKNOWN_FILE_BUDGET_BYTES);

    println!(
        "[CRAWL] files={} dirs={} hinted={:.3}GB unknown={} est_required={:.3}GB in {:.2}s",
        result.file_count,
        result.dir_count,
        bytes_to_gb(result.hinted_bytes),
        result.unknown_size_files,
        bytes_to_gb(result.est_required_bytes),
        result.crawl_secs
    );

    // [PHASE 15 & 16] CI Dynamic Anti-Contamination Verification Limits
    let mut signature_breach = false;
    let mut expected_message = String::new();

    let sig_path = Path::new("tests").join("matrix_signatures.json");
    let mut registry: HashMap<String, SignatureMap> = HashMap::new();

    if let Ok(data) = fs::read_to_string(&sig_path) {
        if let Ok(parsed) = serde_json::from_str(&data) {
            registry = parsed;
        }
    }

    if let Some(expected) = registry.get(adapter.id) {
        // We only trigger hard failure if the parse yields LESS than historical (Structural Broken DOM).
        // 0/0 allows gracefully passing for known-offline targets or intentionally empty yields.
        if result.file_count < expected.files {
            signature_breach = true;
            expected_message = format!(
                "Expected >= {} files. Got {} files.",
                expected.files, result.file_count
            );
        } else if result.file_count > expected.files || result.dir_count > expected.dirs {
            println!(
                "\n[WARNING] Adapter {} naturally Grew! Files: {} -> {}, Dirs: {} -> {}",
                adapter.id, expected.files, result.file_count, expected.dirs, result.dir_count
            );
            // Autonomous Learning Update: Automatically bump the HWM threshold avoiding manual CI maintenance.
            if let Some(entry) = registry.get_mut(adapter.id) {
                entry.files = std::cmp::max(entry.files, result.file_count);
                entry.dirs = std::cmp::max(entry.dirs, result.dir_count);
            }
            if let Ok(new_json) = serde_json::to_string_pretty(&registry) {
                let _ = fs::write(&sig_path, new_json);
            }
        }
    } else {
        println!(
            "\n[WARNING] Adapter {} missing from matrix_signatures.json registry bounds!",
            adapter.id
        );
    }

    if signature_breach {
        result.status = "fail".to_string();
        result.reason = format!(
            "ANTI_CONTAMINATION_ERROR: Adapter {} violated historical DOM extraction signature! {}",
            adapter.id, expected_message
        );
        println!("\n[FATAL] {}", result.reason);
        // Force the pipeline to return error immediately to break CI
        return result;
    }

    result.free_before_download_bytes = available_bytes(&fs_probe_path).unwrap_or(0);
    println!(
        "[SPACE] free before download: {:.2} GB",
        bytes_to_gb(result.free_before_download_bytes)
    );
    if result.free_before_download_bytes.lt(&result
        .est_required_bytes
        .saturating_add(SAFETY_RESERVE_BYTES))
    {
        result.status = "skip".to_string();
        result.reason = format!(
            "insufficient space for safe download window: free={:.2}GB required={:.2}GB reserve={:.2}GB",
            bytes_to_gb(result.free_before_download_bytes),
            bytes_to_gb(result.est_required_bytes),
            bytes_to_gb(SAFETY_RESERVE_BYTES)
        );
        return result;
    }

    if !options.download {
        result.status = "pass".to_string();
        result.reason = format!(
            "crawl completed in {:.2}s. download bypassed by options.",
            result.crawl_secs
        );
        return result;
    }

    let batch_files = build_batch_files(&entries, &canonical_output_root);
    result.batch_total = batch_files.len();
    if batch_files.is_empty() {
        result.status = "skip".to_string();
        result.reason = "no downloadable file URLs found".to_string();
        return result;
    }

    let progress_done = Arc::new(AtomicUsize::new(0));
    let progress_fail = Arc::new(AtomicUsize::new(0));
    let progress_bytes = Arc::new(AtomicU64::new(0));
    let c_done = progress_done.clone();
    let c_fail = progress_fail.clone();
    let c_bytes = progress_bytes.clone();
    let batch_listener_id = app.listen_any("telemetry_bridge_update", move |evt: Event| {
        if let Ok(update) = serde_json::from_str::<TelemetryBridgeUpdate>(evt.payload()) {
            if let Some(payload) = update.batch_progress {
                c_done.store(payload.completed, Ordering::Relaxed);
                c_fail.store(payload.failed, Ordering::Relaxed);
                c_bytes.store(payload.downloaded_bytes, Ordering::Relaxed);
                // Ignore the BBR / EKF fields in the CLI matrix output for now since we just check disk.
            }
        }
    });

    let _log_listener_id = app.listen_any("log", move |evt: Event| {
        let text = evt.payload().trim_matches('"').replace("\\\"", "\"");
        println!("[Tauri::Log] {}", text);
    });

    let download_start = Instant::now();
    let control = match aria_downloader::activate_download_control() {
        Some(c) => c,
        None => {
            app.unlisten(batch_listener_id);
            result.reason = "download control already active".to_string();
            return result;
        }
    };
    let download_result = tokio::time::timeout(
        Duration::from_secs(DOWNLOAD_TIMEOUT_SECS),
        aria_downloader::start_batch_download(
            app.clone(),
            batch_files,
            120,
            is_onion,
            Some(output_root_string),
            control,
        ),
    )
    .await;
    aria_downloader::clear_download_control();
    app.unlisten(batch_listener_id);

    result.download_secs = download_start.elapsed().as_secs_f64();
    result.batch_done = progress_done.load(Ordering::Relaxed);
    result.batch_failed = progress_fail.load(Ordering::Relaxed);
    result.downloaded_event_bytes = progress_bytes.load(Ordering::Relaxed);
    result.disk_bytes = dir_size_bytes(&canonical_output_root);

    match download_result {
        Ok(Ok(())) => {
            result.status = "pass".to_string();
            result.reason = format!(
                "completed in {:.2}s (crawl {:.2}s + download {:.2}s)",
                started.elapsed().as_secs_f64(),
                result.crawl_secs,
                result.download_secs
            );
        }
        Ok(Err(err)) => {
            result.status = "fail".to_string();
            result.reason = format!("download failed: {}", err);
        }
        Err(_) => {
            result.status = "fail".to_string();
            result.reason = format!("download timeout ({}s)", DOWNLOAD_TIMEOUT_SECS);
        }
    }

    println!(
        "[DONE] status={} reason={} batch={}/{}/{} event_bytes={:.3}GB disk={:.3}GB",
        result.status,
        result.reason,
        result.batch_done,
        result.batch_failed,
        result.batch_total,
        bytes_to_gb(result.downloaded_event_bytes),
        bytes_to_gb(result.disk_bytes)
    );

    result
}

fn select_candidate_urls(adapter: &adapters::AdapterSupportInfo) -> Vec<String> {
    if adapter.id == "dragonforce" {
        return vec![
            normalize_url("http://dragonforxxbp3awc7mzs5dkswrua3znqyx5roefmi4smjrsdi22xwqd.onion/www.rjzavoral.com"),
        ];
    }

    if adapter.id == "worldleaks" {
        return vec![
            normalize_url("https://worldleaksartrjm3c6vasllvgacbi5u3mgzkluehrzhk2jz4taufuid.onion/companies/1829380564/storage"),
            normalize_url("http://worldleaksartrjm3c6vasllvgacbi5u3mgzkluehrzhk2jz4taufuid.onion/companies/1829380564/storage"),
        ];
    }

    let mut urls: Vec<String> = adapter
        .sample_urls
        .iter()
        .map(|u| normalize_url(u))
        .collect();
    urls.sort_by_key(|u| priority_url_key(u));
    urls.dedup();
    urls
}

fn print_summary(results: &[AdapterRunResult], report_path: &Path) {
    let pass = results.iter().filter(|r| r.status == "pass").count();
    let fail = results.iter().filter(|r| r.status == "fail").count();
    let skip = results.iter().filter(|r| r.status == "skip").count();
    let total_downloaded = results
        .iter()
        .map(|r| r.downloaded_event_bytes)
        .sum::<u64>();
    let total_disk = results.iter().map(|r| r.disk_bytes).sum::<u64>();
    println!("\n=== ADAPTER MATRIX SUMMARY ===");
    println!(
        "Adapters tested: {} | pass={} fail={} skip={}",
        results.len(),
        pass,
        fail,
        skip
    );
    println!(
        "Aggregates: event_bytes={:.3}GB disk_bytes={:.3}GB",
        bytes_to_gb(total_downloaded),
        bytes_to_gb(total_disk)
    );
    println!("Report: {}", report_path.display());

    let mut ci_fail = false;
    for r in results {
        println!(
            "- [{}] {} | detected={} | files={} dirs={} | reason={}",
            r.status.to_uppercase(),
            r.adapter_name,
            r.detected_adapter,
            r.file_count,
            r.dir_count,
            r.reason
        );
        if r.status == "fail" {
            ci_fail = true;
        }
    }

    // [PHASE 15] CI Anti-Contamination Enforcement
    if ci_fail {
        println!("Exit code: 1 (ANTI_CONTAMINATION_ERROR)");
        std::process::exit(1);
    } else {
        println!("Exit code: 0");
    }
}

fn main() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build tokio runtime");

    rt.block_on(async {
        let run_id = now_epoch_secs();
        // Cleanup old lockbit/matrix temp folders from prior runs to keep disk pressure low.
        if let Ok(entries) = std::fs::read_dir("/tmp") {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if name.starts_with("onionforge_lockbit_live_")
                    || name.starts_with("onionforge_adapter_matrix_")
                {
                    safe_remove_dir(&entry.path());
                }
            }
        }

        let root_raw = PathBuf::from(format!("/tmp/onionforge_adapter_matrix_{}", run_id));
        let root = match path_utils::canonicalize_output_root(&root_raw.to_string_lossy()) {
            Ok(path) => path,
            Err(err) => {
                eprintln!("[FATAL] failed to canonicalize matrix root: {}", err);
                std::process::exit(2);
            }
        };
        let report_path = root.join("adapter_matrix_report.json");
        let _ = std::fs::create_dir_all(&root);

        println!("=== ADAPTER MATRIX LIVE PIPELINE ===");
        println!("Root temp: {}", root.display());
        println!(
            "Free space now: {:.2} GB",
            bytes_to_gb(available_bytes(&root).unwrap_or(0))
        );

        let app = tauri::Builder::default()
            .manage(AppState::default())
            .build(tauri::generate_context!())
            .expect("build tauri app");
        let app_handle = app.handle().clone();
        let bridge = app.state::<AppState>().telemetry_bridge.clone();
        telemetry_bridge::spawn_bridge_emitter(app.handle().clone(), bridge);

        let mut results: Vec<AdapterRunResult> = Vec::new();
        let catalog = adapters::support_catalog();
        let adapter_filter_env = std::env::var("ADAPTER_FILTER").ok();
        let adapter_filter: Option<Vec<String>> = adapter_filter_env.map(|value| {
            value
                .split(',')
                .map(|v| v.trim().to_lowercase())
                .filter(|v| !v.is_empty())
                .collect()
        });
        let adapters_to_test: Vec<_> = catalog
            .into_iter()
            .filter(|a| a.id != "autoindex" && a.id != "nu_server")
            .filter(|a| {
                if let Some(filter) = &adapter_filter {
                    return filter.iter().any(|item| item == a.id);
                }
                true
            })
            .take(MAX_ADAPTERS)
            .collect();

        for adapter in adapters_to_test {
            let mut run_result: Option<AdapterRunResult> = None;
            let candidates = select_candidate_urls(&adapter);
            let adapter_output = root.join(format!("adapter_{}", adapter.id));

            if candidates.is_empty() {
                println!(
                    "[SKIP] Adapter {} has no configured candidate testing URLs.",
                    adapter.name
                );
                continue;
            }

            for candidate in candidates {
                let result =
                    run_single_adapter(&app_handle, &adapter, &candidate, &adapter_output).await;
                let should_try_next = result.status == "skip"
                    && (result.reason.contains("fingerprint fetch failed")
                        || result.reason.contains("fingerprint fetch timed out")
                        || result.reason.contains("no adapter matched"));

                run_result = Some(result);
                safe_remove_dir(&adapter_output);
                tor::cleanup_stale_tor_daemons();

                if !should_try_next {
                    break;
                }
            }

            if let Some(result) = run_result {
                results.push(result);
            }
        }

        let report_json = serde_json::to_string_pretty(&results).unwrap_or_else(|_| "[]".into());
        let _ = std::fs::write(&report_path, report_json.as_bytes());
        print_summary(&results, &report_path);
    });
}
