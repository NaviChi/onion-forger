pub mod adapters;
pub mod aria_downloader;
#[cfg(feature = "azure")]
pub mod azure_connectivity;
pub mod bbr;
pub mod bft_quorum;
pub mod binary_telemetry;
pub mod circuit_health;
mod cli;
pub mod db;
pub mod frontier;
pub mod ghost_browser;
pub mod index_generator;
pub mod io_vanguard;
pub mod kalman;
pub mod mega_handler; // Phase 52: Mega.nz public folder support
pub mod multi_client_pool;
#[allow(dead_code)]
pub mod multipath; // Experimental lab engine; production downloads use aria_downloader.
pub mod network_disk;
pub mod path_utils;
pub mod resource_governor;
pub mod runtime_metrics;
pub mod scorer;
pub mod speculative_prefetch;
pub mod spillover;
pub mod work_stealing;
pub mod subtree_heatmap;
pub mod target_state;
pub mod telemetry_bridge;
pub mod timer;
pub mod tor;
pub mod tor_native;
pub mod tor_runtime; // Phase 45: Parallel chunk downloading
pub mod torrent_handler; // Phase 52: BitTorrent .torrent + magnet support // Phase 53: Optional Azure + Intranet enterprise

use std::sync::Arc;
use tauri::Emitter;
use tauri::Manager;
use tokio::sync::Mutex;

pub(crate) fn url_targets_onion(input: &str) -> bool {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return false;
    }

    if let Ok(parsed) = url::Url::parse(trimmed) {
        return parsed
            .host_str()
            .map(|host| host.to_ascii_lowercase().ends_with(".onion"))
            .unwrap_or(false);
    }

    let lowered = trimmed.to_ascii_lowercase();
    let authority = lowered
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(lowered.as_str())
        .split('/')
        .next()
        .unwrap_or_default()
        .rsplit('@')
        .next()
        .unwrap_or_default()
        .split(':')
        .next()
        .unwrap_or_default();

    authority.ends_with(".onion")
}

/// Global shared frontier for cancellation support
pub struct AppState {
    active_frontier: Mutex<Option<Arc<frontier::CrawlerFrontier>>>,
    pub(crate) current_target_dir: Mutex<Option<std::path::PathBuf>>,
    pub(crate) current_target_key: Mutex<Option<String>>,
    pub vfs: db::SledVfs,
    pub telemetry: runtime_metrics::RuntimeTelemetry,
    pub telemetry_bridge: telemetry_bridge::TelemetryBridge,
    pub crawl_swarm_guard:
        tokio::sync::Mutex<Option<Arc<tokio::sync::Mutex<tor::TorProcessGuard>>>>,
    pub download_swarm_guard:
        tokio::sync::Mutex<Option<Arc<tokio::sync::Mutex<tor::TorProcessGuard>>>>,
    /// Phase 53: Azure connectivity state (only compiled with `--features azure`)
    #[cfg(feature = "azure")]
    pub azure: tokio::sync::Mutex<azure_connectivity::AzureConnectivityState>,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct NativeWebviewSmokeConfig {
    enabled: bool,
    report_path: Option<String>,
    auto_exit: bool,
    wait_ms: u64,
    expected_test_ids: Vec<String>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct NativeWebviewSmokeResult {
    mounted: bool,
    title: String,
    href: String,
    is_tauri_runtime: bool,
    expected_test_ids: Vec<String>,
    found_test_ids: Vec<String>,
    missing_test_ids: Vec<String>,
    reported_at_epoch_ms: u64,
}

fn native_webview_smoke_expected_test_ids() -> Vec<String> {
    vec![
        "toolbar".to_string(),
        "input-target-url".to_string(),
        "btn-start-queue".to_string(),
        "btn-load-target".to_string(),
        "resource-metrics-card".to_string(),
    ]
}

fn native_webview_smoke_report_path() -> Option<std::path::PathBuf> {
    std::env::var("CRAWLI_NATIVE_SMOKE_REPORT_PATH")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(std::path::PathBuf::from)
}

fn native_webview_smoke_enabled() -> bool {
    native_webview_smoke_report_path().is_some()
}

fn native_webview_smoke_auto_exit() -> bool {
    std::env::var("CRAWLI_NATIVE_SMOKE_AUTO_EXIT")
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

fn native_webview_smoke_wait_ms() -> u64 {
    std::env::var("CRAWLI_NATIVE_SMOKE_WAIT_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(12_000)
        .clamp(1_000, 60_000)
}

impl Default for AppState {
    fn default() -> Self {
        let telemetry_bridge = telemetry_bridge::TelemetryBridge::default();
        Self {
            active_frontier: Mutex::new(None),
            current_target_dir: Mutex::new(None),
            current_target_key: Mutex::new(None),
            vfs: db::SledVfs::default(),
            telemetry: runtime_metrics::RuntimeTelemetry::default(),
            telemetry_bridge,
            crawl_swarm_guard: tokio::sync::Mutex::new(None),
            download_swarm_guard: tokio::sync::Mutex::new(None),
            #[cfg(feature = "azure")]
            azure: tokio::sync::Mutex::new(azure_connectivity::AzureConnectivityState::default()),
        }
    }
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CrawlStatusUpdate {
    phase: String,
    progress_percent: f64,
    visited_nodes: usize,
    processed_nodes: usize,
    queued_nodes: usize,
    active_workers: usize,
    worker_target: usize,
    eta_seconds: Option<u64>,
    estimation: String,
    delta_new_files: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    vanguard: Option<VanguardTelemetry>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VanguardTelemetry {
    pub current: usize,
    pub target: usize,
    pub status: String,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DownloadBatchStartedEvent {
    total_files: usize,
    total_bytes_hint: u64,
    unknown_size_files: usize,
    output_dir: String,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CrawlSessionResult {
    target_key: String,
    discovered_count: usize,
    file_count: usize,
    folder_count: usize,
    best_prior_count: usize,
    raw_this_run_count: usize,
    merged_effective_count: usize,
    crawl_outcome: String,
    retry_count_used: usize,
    stable_current_listing_path: String,
    stable_current_dirs_listing_path: String,
    stable_best_listing_path: String,
    stable_best_dirs_listing_path: String,
    auto_download_started: bool,
    output_dir: String,
}

fn estimate_progress_percent(
    visited: usize,
    processed: usize,
    queued: usize,
    active_workers: usize,
) -> f64 {
    if processed == 0 {
        return 0.0;
    }

    // Adaptive estimate for unknown totals:
    // combines discovered nodes + active backlog + queue growth bias.
    let growth_bias = ((queued as f64) * 0.35).ceil() as usize;
    let estimated_total = visited
        .max(processed + queued.max(active_workers))
        .max(processed + growth_bias)
        .max(1);

    ((processed as f64 / estimated_total as f64) * 100.0).clamp(0.0, 99.4)
}

fn crawl_status_snapshot(
    frontier: &frontier::CrawlerFrontier,
    phase: &str,
    progress_percent: f64,
    eta_seconds: Option<u64>,
) -> CrawlStatusUpdate {
    let snapshot = frontier.progress_snapshot();

    CrawlStatusUpdate {
        phase: phase.to_string(),
        progress_percent: progress_percent.clamp(0.0, 100.0),
        visited_nodes: snapshot.visited,
        processed_nodes: snapshot.processed,
        queued_nodes: snapshot.queued,
        active_workers: snapshot.active_workers,
        worker_target: snapshot.worker_target,
        eta_seconds,
        estimation: "adaptive-frontier".to_string(),
        delta_new_files: frontier
            .delta_new_files
            .load(std::sync::atomic::Ordering::Relaxed),
        vanguard: if frontier.stealth_ramp_active() {
            let active = snapshot.active_workers;
            let target = snapshot.worker_target;
            Some(VanguardTelemetry {
                current: active,
                target,
                status: if active >= target {
                    "Maxed".to_string()
                } else {
                    format!("Ramping... ({}/{})", active, target)
                },
            })
        } else {
            None
        },
    }
}

fn spawn_crawl_status_emitter(
    app: tauri::AppHandle,
    frontier: Arc<frontier::CrawlerFrontier>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(450));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        let mut last_processed = frontier.processed_count();
        let mut last_tick = std::time::Instant::now();
        let mut ewma_rate = 0.0_f64;
        let mut monotonic_percent = 0.0_f64;

        loop {
            interval.tick().await;

            let snapshot = frontier.progress_snapshot();
            let visited = snapshot.visited;
            let processed = snapshot.processed;
            let queued = snapshot.queued;
            let active_workers = snapshot.active_workers;

            let now = std::time::Instant::now();
            let dt = now.duration_since(last_tick).as_secs_f64().max(0.001);
            let delta = processed.saturating_sub(last_processed) as f64;
            let instant_rate = delta / dt;
            ewma_rate = if ewma_rate <= 0.0 {
                instant_rate
            } else {
                (ewma_rate * 0.75) + (instant_rate * 0.25)
            };
            last_tick = now;
            last_processed = processed;

            let mut estimate =
                estimate_progress_percent(visited.max(1), processed, queued, active_workers);
            if queued == 0 && active_workers == 0 && processed > 0 {
                estimate = 99.4;
            }
            monotonic_percent = monotonic_percent.max(estimate);

            let eta_seconds = if queued > 0 && ewma_rate > 0.05 {
                Some((queued as f64 / ewma_rate).ceil() as u64)
            } else {
                None
            };

            if let Some(state) = app.try_state::<AppState>() {
                state.telemetry.set_request_metrics(
                    frontier.processed_count(),
                    frontier.successful_count(),
                    frontier.failed_count(),
                );
            }

            let phase = if frontier.is_cancelled() {
                "cancelled"
            } else if processed == 0 {
                "probing"
            } else if queued > 0 || active_workers > 0 {
                "crawling"
            } else {
                "settling"
            };

            let payload =
                crawl_status_snapshot(frontier.as_ref(), phase, monotonic_percent, eta_seconds);
            telemetry_bridge::publish_crawl_status(&app, payload);

            if frontier.is_cancelled() {
                break;
            }
        }
    })
}

fn canonical_output_root(output_dir: &str) -> Result<std::path::PathBuf, String> {
    path_utils::canonicalize_output_root(output_dir)
        .map_err(|e| format!("Invalid output directory: {e}"))
}

fn direct_artifact_name(url: &str) -> String {
    url.split('/')
        .next_back()
        .unwrap_or("artifact")
        .split('?')
        .next()
        .unwrap_or("artifact")
        .to_string()
}

fn support_key_for_path(path: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let normalized_path = path_utils::normalize_windows_device_path(path);
    let mut hasher = DefaultHasher::new();
    normalized_path.hash(&mut hasher);
    let hash = hasher.finish();

    let mut base = String::with_capacity(normalized_path.len());
    let mut pending_separator = false;
    for ch in normalized_path.trim_start_matches(['/', '\\']).chars() {
        match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' => {
                base.push(ch);
                pending_separator = false;
            }
            _ => {
                if !pending_separator {
                    base.push('_');
                    pending_separator = true;
                }
            }
        }
    }
    base = base.trim_matches('_').to_string();
    if base.is_empty() {
        base = "root".to_string();
    }
    if base.len() > 96 {
        let safe_len = base.floor_char_boundary(96);
        base.truncate(safe_len);
    }

    format!("{base}_{hash:016x}")
}

pub(crate) fn support_artifact_dir_for_output_root(
    output_root: &std::path::Path,
) -> std::path::PathBuf {
    output_root
        .join(".onionforge_support")
        .join(support_key_for_path(&path_utils::display_path(output_root)))
}

pub(crate) struct SupportArtifactDirResolution {
    pub path: std::path::PathBuf,
    pub used_fallback: bool,
    pub fallback_reason: Option<String>,
    pub note: Option<String>,
}

pub(crate) fn ensure_support_artifact_dir_for_output_root(
    output_root: &std::path::Path,
) -> std::io::Result<SupportArtifactDirResolution> {
    let support_dir = support_artifact_dir_for_output_root(output_root);
    std::fs::create_dir_all(&support_dir).map_err(|err| {
        std::io::Error::new(
            err.kind(),
            format!(
                "failed to create support dir '{}' ({err})",
                path_utils::display_path(&support_dir)
            ),
        )
    })?;

    Ok(SupportArtifactDirResolution {
        path: support_dir,
        used_fallback: false,
        fallback_reason: None,
        note: None,
    })
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct DownloadSupportIndexEntry {
    logical_path: String,
    raw_url: String,
    size_bytes: Option<u64>,
    entry_type: &'static str,
}

async fn write_download_support_index(
    entries: &[adapters::FileEntry],
    support_dir: &std::path::Path,
) -> anyhow::Result<()> {
    let index_entries = entries
        .iter()
        .map(|entry| DownloadSupportIndexEntry {
            logical_path: entry.path.clone(),
            raw_url: entry.raw_url.clone(),
            size_bytes: entry.size_bytes,
            entry_type: match entry.entry_type {
                adapters::EntryType::File => "file",
                adapters::EntryType::Folder => "folder",
            },
        })
        .collect::<Vec<_>>();
    let support_index_path = support_dir.join("download_support_index.json");
    tokio::fs::write(
        &support_index_path,
        serde_json::to_vec_pretty(&index_entries)?,
    )
    .await?;
    Ok(())
}

fn build_download_manifest(entries: &[adapters::FileEntry]) -> String {
    let mut manifest = String::new();
    manifest.push_str("# OnionForge Download Manifest\n");
    manifest.push_str(&format!("# Generated: {}\n", chrono_stub()));
    manifest.push_str(&format!("# Total Entries: {}\n\n", entries.len()));
    for entry in entries {
        let type_tag = match entry.entry_type {
            adapters::EntryType::Folder => "DIR ",
            adapters::EntryType::File => "FILE",
        };
        let size_tag = entry
            .size_bytes
            .map(format_bytes)
            .unwrap_or_else(|| "0 B".to_string());
        let decoded_path = path_utils::url_decode(&entry.path);
        manifest.push_str(&format!(
            "{} {:>12}  {}  {}\n",
            type_tag, size_tag, decoded_path, entry.raw_url
        ));
    }
    manifest
}

async fn write_download_support_artifacts(
    entries: &[adapters::FileEntry],
    support_dir: &std::path::Path,
) -> anyhow::Result<()> {
    let manifest_path = support_dir.join("_onionforge_manifest.txt");
    tokio::fs::write(&manifest_path, build_download_manifest(entries).as_bytes()).await?;
    write_download_support_index(entries, support_dir).await
}

fn ranked_qilin_download_hosts(entries: &[adapters::FileEntry]) -> Vec<String> {
    let mut counts = std::collections::HashMap::<String, usize>::new();
    for entry in entries {
        if !matches!(entry.entry_type, adapters::EntryType::File) {
            continue;
        }
        let Some(host) = reqwest::Url::parse(&entry.raw_url)
            .ok()
            .and_then(|url| url.host_str().map(|host| host.to_ascii_lowercase()))
        else {
            continue;
        };
        if !host.ends_with(".onion") {
            continue;
        }
        *counts.entry(host).or_default() += 1;
    }

    // Phase 132: Inject alternate hosts from QilinNodeCache sled DB.
    // The crawl uses a single winner host, so entries only contain 1 host.
    // Reading alive alternate nodes from the cache gives us mirror hosts
    // for download striping across independent servers.
    let cache_hosts = read_qilin_cache_hosts();
    for host in cache_hosts {
        counts.entry(host).or_insert(1);
    }

    let mut ranked = counts.into_iter().collect::<Vec<_>>();
    ranked.sort_by(|(host_a, count_a), (host_b, count_b)| {
        count_b.cmp(count_a).then_with(|| host_a.cmp(host_b))
    });
    ranked.into_iter().map(|(host, _)| host).collect()
}

/// Phase 132: Read alive storage node hosts from QilinNodeCache sled DB.
/// Returns up to 5 unique .onion hosts that are not cooling down.
fn read_qilin_cache_hosts() -> Vec<String> {
    let mut path = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("."));
    path.push(".crawli");
    path.push("qilin_nodes.sled");

    let db = match sled::open(&path) {
        Ok(db) => db,
        Err(_) => return Vec::new(),
    };

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let mut hosts: Vec<(String, u32)> = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for item in db.scan_prefix(b"node:").flatten() {
        if let Ok(node) = serde_json::from_slice::<serde_json::Value>(&item.1) {
            let host = node.get("host").and_then(|h| h.as_str()).unwrap_or_default().to_string();
            let cooldown = node.get("cooldown_until").and_then(|c| c.as_u64()).unwrap_or(0);
            let last_seen = node.get("last_seen").and_then(|l| l.as_u64()).unwrap_or(0);
            let hit_count = node.get("hit_count").and_then(|h| h.as_u64()).unwrap_or(0) as u32;

            if host.is_empty() || !host.ends_with(".onion") {
                continue;
            }
            // Skip cooling down nodes and stale nodes (>7 days)
            if cooldown > now || now.saturating_sub(last_seen) > 604_800 {
                continue;
            }
            if seen.insert(host.clone()) {
                hosts.push((host, hit_count));
            }
        }
    }

    // Sort by hit count descending — most reliable mirrors first
    hosts.sort_by(|a, b| b.1.cmp(&a.1));
    hosts.into_iter().take(5).map(|(h, _)| h).collect()
}

fn stable_rotation_index(key: &str, len: usize) -> usize {
    use std::hash::{Hash, Hasher};

    if len <= 1 {
        return 0;
    }

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut hasher);
    (hasher.finish() as usize) % len
}

fn build_qilin_alternate_urls(
    raw_url: &str,
    ranked_hosts: &[String],
    rotation_key: &str,
) -> Vec<String> {
    let Some(current_host) = reqwest::Url::parse(raw_url)
        .ok()
        .and_then(|url| url.host_str().map(|host| host.to_ascii_lowercase()))
    else {
        return Vec::new();
    };

    let mut seen = std::collections::HashSet::<String>::new();
    let mut candidate_hosts = Vec::new();
    for host in ranked_hosts {
        let normalized = host.to_ascii_lowercase();
        if normalized == current_host || !seen.insert(normalized.clone()) {
            continue;
        }
        candidate_hosts.push(normalized);
    }

    if candidate_hosts.len() > 1 {
        let rotation = stable_rotation_index(rotation_key, candidate_hosts.len());
        candidate_hosts.rotate_left(rotation);
    }

    candidate_hosts
        .into_iter()
        .filter_map(|host| rewrite_qilin_seed_host(raw_url, &host))
        .collect()
}

fn build_batch_file_entry(
    entry: &adapters::FileEntry,
    safe_target: String,
    ranked_hosts: &[String],
) -> aria_downloader::BatchFileEntry {
    aria_downloader::BatchFileEntry {
        url: entry.raw_url.clone(),
        path: safe_target,
        size_hint: entry.size_bytes,
        jwt_exp: entry.jwt_exp,
        alternate_urls: build_qilin_alternate_urls(&entry.raw_url, ranked_hosts, &entry.path),
    }
}

fn looks_like_direct_artifact(url: &str, is_binary: bool) -> Option<String> {
    let name = direct_artifact_name(url);
    if is_binary || (name.contains('.') && !name.ends_with('/')) {
        Some(name)
    } else {
        None
    }
}

fn summarize_entry_slice(entries: &[adapters::FileEntry]) -> db::VfsSummary {
    let mut summary = db::VfsSummary::default();
    for entry in entries {
        summary.discovered_count += 1;
        match entry.entry_type {
            adapters::EntryType::File => {
                summary.file_count += 1;
                summary.total_size_bytes = summary
                    .total_size_bytes
                    .saturating_add(entry.size_bytes.unwrap_or(0));
            }
            adapters::EntryType::Folder => {
                summary.folder_count += 1;
            }
        }
    }
    summary
}

struct CrawlAttemptResult {
    summary: db::VfsSummary,
    was_cancelled: bool,
}

fn warmup_quorum_target(client_count: usize) -> usize {
    if client_count <= 1 {
        return client_count;
    }

    client_count.clamp(2, 4).min((client_count / 2).max(2))
}

async fn warm_onion_clients_until_quorum(
    frontier: &frontier::CrawlerFrontier,
    url: &str,
    timer: &crate::timer::CrawlTimer,
) {
    use futures::stream::{FuturesUnordered, StreamExt};

    let clients_to_warm = frontier.http_clients.len();
    if clients_to_warm <= 1 {
        return;
    }

    let ready_quorum = warmup_quorum_target(clients_to_warm);
    timer.emit_log(&format!(
        "[Tor Swarm] Executing quorum warmup across {} clients (ready quorum {})...",
        clients_to_warm, ready_quorum
    ));

    let mut warmups = FuturesUnordered::new();
    for client in &frontier.http_clients {
        let warm_client = client.clone();
        let warm_url = url.to_string();
        warmups.push(tokio::spawn(async move {
            matches!(
                tokio::time::timeout(
                    std::time::Duration::from_secs(45),
                    warm_client.head(&warm_url).send(),
                )
                .await,
                Ok(Ok(_))
            )
        }));
    }

    let mut ready = 0usize;
    while let Some(result) = warmups.next().await {
        if matches!(result, Ok(true)) {
            ready += 1;
            if ready >= ready_quorum {
                break;
            }
        }

        if ready + warmups.len() < ready_quorum {
            break;
        }
    }

    let outstanding = warmups.len();
    drop(warmups);

    if ready >= ready_quorum {
        timer.emit_log(&format!(
            "[Tor Swarm] Warmup quorum reached: {}/{} clients ready. Fingerprinting begins while {} warmups continue in background.",
            ready, clients_to_warm, outstanding
        ));
    } else {
        timer.emit_log(&format!(
            "[Tor Swarm] Warmup quorum not reached: {}/{} ready. Proceeding with best available clients.",
            ready, clients_to_warm
        ));
    }
}

async fn refresh_frontier_clients_from_swarm(
    frontier: &mut frontier::CrawlerFrontier,
    timer: &crate::timer::CrawlTimer,
    phase_label: &str,
) {
    let Some(guard_arc) = frontier.swarm_guard.clone() else {
        return;
    };

    let shared_clients = {
        let guard = guard_arc.lock().await;
        guard.get_arti_clients()
    };
    let before = frontier.http_clients.len();
    let after = frontier.sync_arti_clients(&shared_clients);

    if after > before {
        let message = format!(
            "[Tor Swarm] Refreshed frontier clients before {}: {} -> {} live Arti clients",
            phase_label, before, after
        );
        timer.emit_log(&message);
        println!("[Crawli Bootstrap] {}", message);
    }
}

async fn execute_crawl_attempt(
    url: &str,
    options: &frontier::CrawlOptions,
    output_root: &std::path::Path,
    target_paths: &target_state::TargetPaths,
    app: &tauri::AppHandle,
    vfs: &db::SledVfs,
    ledger: std::sync::Arc<crate::target_state::TargetLedger>,
    // Phase 141: Download feed sender for event-driven parallel downloads
    download_feed_tx: Option<std::sync::Arc<tokio::sync::mpsc::UnboundedSender<adapters::FileEntry>>>,
) -> Result<CrawlAttemptResult, String> {
    let timer = crate::timer::CrawlTimer::new(app.clone());
    timer.emit_log(&format!("[System] Bootstrapping Target: {}", url));

    let is_onion = url_targets_onion(url) && !options.force_clearnet;
    let support_resolution = ensure_support_artifact_dir_for_output_root(output_root)
        .map_err(|e| format!("Failed to create support directory: {e}"))?;
    let support_dir = support_resolution.path;
    if let Some(note) = support_resolution.note.as_deref() {
        timer.emit_log(&format!("[PATH] {note}"));
    }
    if support_resolution.used_fallback {
        timer.emit_log(&format!(
            "[PATH] Support artifact root fallback engaged: {}",
            path_utils::display_path(&support_dir)
        ));
        if let Some(reason) = support_resolution.fallback_reason.as_deref() {
            timer.emit_log(&format!(
                "[PATH] Preferred sibling support root was unavailable: {reason}"
            ));
        }
    }

    let app_state = app.state::<AppState>();
    let mut arti_clients = Vec::new();
    let mut swarm_guard_for_frontier = None;

    if is_onion {
        let mut pre_warmed = false;
        if let Some(guard_arc) = app_state.crawl_swarm_guard.lock().await.as_ref() {
            let guard = guard_arc.lock().await;
            arti_clients = guard.get_arti_clients();
            if !arti_clients.is_empty() {
                swarm_guard_for_frontier = Some(guard_arc.clone());
                pre_warmed = true;
                timer.emit_log(&format!(
                    "[Tor Swarm] Using pre-warmed Phantom Swarm ({} clients)",
                    arti_clients.len()
                ));
                println!(
                    "[Crawli Bootstrap] Using pre-warmed Phantom Swarm ({} clients)",
                    arti_clients.len()
                );
            }
        }

        if !pre_warmed {
            timer.emit_log("[Tor Swarm] No pre-warmed swarm found. Cleaning up...");
            tor::cleanup_stale_tor_daemons();
            timer.emit_log("[Tor Swarm] Bootstrapping new native Arti cluster...");
            println!("[Crawli Bootstrap] starting tor bootstrap for {}", url);

            // Phase 117: Hardcoded to 8 clients — MultiClientPool caps at 8 anyway
            let target_clients = 8usize;
            match tor::bootstrap_tor_cluster_for_traffic(
                app.clone(),
                target_clients,
                0,
                tor::SwarmTrafficClass::OnionService,
            )
            .await
            {
                Ok((guard, ports)) => {
                    timer.emit_log(&format!(
                        "[Tor Swarm] Bootstrap complete: {} runtime, {} clients active",
                        guard.runtime_label(),
                        ports.len()
                    ));
                    println!(
                        "[Crawli Bootstrap] tor bootstrap complete: runtime={} ports={:?}",
                        guard.runtime_label(),
                        ports
                    );
                    arti_clients = guard.get_arti_clients();
                    let arc_guard = std::sync::Arc::new(tokio::sync::Mutex::new(guard));
                    *app_state.crawl_swarm_guard.lock().await = Some(arc_guard.clone());
                    swarm_guard_for_frontier = Some(arc_guard);
                }
                Err(e) => return Err(format!("Failed to start Tor Swarm: {}", e)),
            }
        }
    }

    let client_count = if is_onion {
        arti_clients.len().max(1)
    } else {
        1 // Clearnet uses a single client
    };

    let mut frontier = frontier::CrawlerFrontier::new(
        Some(app.clone()),
        url.to_string(),
        client_count,
        is_onion,
        Vec::new(), // explicit ports removed, handled internally
        arti_clients,
        options.clone(),
        Some(target_paths.clone()),
    );
    println!(
        "[Crawli Bootstrap] frontier initialized: clients={} onion={}",
        frontier.http_clients.len(),
        frontier.is_onion
    );
    frontier.swarm_guard = swarm_guard_for_frontier;

    let vfs_path = support_dir.join(".crawli_vtdb");
    let vfs_path_str = vfs_path.to_string_lossy().to_string();
    timer.emit_log(&format!(
        "[VFS] Initializing Sled storage at {}",
        vfs_path_str
    ));
    let _ = vfs.initialize(&vfs_path_str).await;
    let _ = vfs.clear().await;
    timer.emit_log("[VFS] Storage initialized & wiped for new run");
    println!("[Crawli Bootstrap] vfs initialized at {}", vfs_path_str);

    // Phase 136: Load persisted host capabilities (range support, RTT EWMAs)
    aria_downloader::initialize_host_capability_store();

    let registry = adapters::AdapterRegistry::new().with_explorer_context(ledger.clone());
    let hinted_adapter = registry.determine_adapter_from_url_hint(url);

    if is_onion {
        refresh_frontier_clients_from_swarm(&mut frontier, &timer, "warmup").await;
        if let Some(adapter) = hinted_adapter.as_ref() {
            timer.emit_log(&format!(
                "[Tor Swarm] Strong adapter hint ({}) present. Skipping blocking warmup and proceeding directly to adapter execution.",
                adapter.name()
            ));
        } else {
            warm_onion_clients_until_quorum(&frontier, url, &timer).await;
            refresh_frontier_clients_from_swarm(&mut frontier, &timer, "fingerprinting").await;
        }
    }
    let mut fingerprint: Option<adapters::SiteFingerprint> = None;
    let mut fingerprint_latency_ms = 0u64;

    if let Some(adapter) = hinted_adapter {
        timer.emit_log(&format!(
            "[Fingerprint] Strong URL hint matched {}. Skipping network fingerprint probe.",
            adapter.name()
        ));
        app.emit(
            "crawl_log",
            format!(
                "[Fingerprint] Strong URL hint matched {}. Skipping network fingerprint probe.",
                adapter.name()
            ),
        )
        .unwrap_or_default();
        app_state.telemetry.set_fingerprint_latency_ms(0);
    } else {
        println!("[Crawli Fingerprint] requesting initial URL: {}", url);
        timer.emit_log("[Fingerprint] Initiating site capabilities probe...");
        let fingerprint_started_at = std::time::Instant::now();

        let fingerprint_attempts = if is_onion { 4 } else { 2 };
        let mut fingerprint_attempt = 1usize;
        let resp = loop {
            let attempt = fingerprint_attempt;
            let (cid, client) = frontier.get_client();
            // PR-CRAWLER-012: Every HTTP call through Tor must have explicit timeout
            match tokio::time::timeout(std::time::Duration::from_secs(30), client.get(url).send())
                .await
            {
                Ok(Ok(r)) => {
                    timer.emit_log(&format!(
                        "[Fingerprint] Probe successful! Status: {}, Final URL: {}",
                        r.status(),
                        r.url()
                    ));
                    println!(
                        "[Crawli Fingerprint] initial URL responded: status={} final={}",
                        r.status(),
                        r.url()
                    );
                    break r;
                }
                Ok(Err(e)) => {
                    let error_text = e.to_string();
                    println!(
                        "[Crawli Fingerprint] initial URL request failed on attempt {} via cid {}: {}",
                        attempt, cid, error_text
                    );

                    if attempt >= fingerprint_attempts {
                        timer.emit_log(&format!(
                            "[Fingerprint] CRITICAL FAIL: Site offline or blocking Tor. ({})",
                            error_text
                        ));

                        // PR-CRAWLER-017: If the fallback chain fails completely, the Phantom Swarm
                        // may have a burned guard node or IP ban. Shred it from AppState so
                        // the user's next 'Retry' click forces a 100% cold boot.
                        timer.emit_log("[Tor Swarm] ⚠ Evicting degraded Phantom Swarm to force cold boot next run.");
                        if let Some(guard_arc) = app_state.crawl_swarm_guard.lock().await.take() {
                            let mut g = guard_arc.lock().await;
                            g.shutdown_all();
                        }

                        return Err(format!(
                            "OFFLINE_SYNC_ERROR: The site might be down. Please manually check it to verify if it is actually functional and active. ({})",
                            error_text
                        ));
                    }

                    timer.emit_log(&format!(
                        "[Fingerprint] Probe failed on attempt {}/{}. Rotating circuit...",
                        attempt, fingerprint_attempts
                    ));
                    if is_onion {
                        frontier.trigger_circuit_isolation(cid).await;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis((attempt as u64) * 750))
                        .await;
                    fingerprint_attempt = fingerprint_attempt.saturating_add(1);
                }
                Err(_) => {
                    // Timeout — treat as failure
                    println!(
                        "[Crawli Fingerprint] attempt {} timed out after 30s via cid {}",
                        attempt, cid
                    );

                    if attempt >= fingerprint_attempts {
                        timer.emit_log("[Fingerprint] CRITICAL FAIL: All fingerprint attempts timed out (30s each).");

                        // PR-CRAWLER-017: Shred degraded Phantom Swarm to prevent infinite loop of dead circuits
                        timer.emit_log(
                            "[Tor Swarm] ⏰ Evicting dead Phantom Swarm to force cold boot next run.",
                        );
                        if let Some(guard_arc) = app_state.crawl_swarm_guard.lock().await.take() {
                            let mut g = guard_arc.lock().await;
                            g.shutdown_all();
                        }

                        return Err(
                            "OFFLINE_SYNC_ERROR: All fingerprint probes timed out. The site may be down or Tor circuits are degraded.".to_string()
                        );
                    }

                    timer.emit_log(&format!(
                        "[Fingerprint] Probe timed out on attempt {}/{}. Rotating circuit...",
                        attempt, fingerprint_attempts
                    ));
                    if is_onion {
                        frontier.trigger_circuit_isolation(cid).await;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis((attempt as u64) * 750))
                        .await;
                    fingerprint_attempt = fingerprint_attempt.saturating_add(1);
                }
            }
        };

        let status = resp.status().as_u16();
        let headers = resp.headers().clone();
        let content_type = headers
            .get("content-type")
            .and_then(|h| h.to_str().ok())
            .unwrap_or("");
        let is_binary = !content_type.starts_with("text/")
            && !content_type.contains("json")
            && !content_type.contains("xml")
            && (content_type.contains("application/")
                || url.ends_with(".7z")
                || url.ends_with(".zip")
                || url.ends_with(".rar")
                || url.ends_with(".tar.gz")
                || url.ends_with(".tar")
                || url.ends_with(".gz"));

        let body = if is_binary {
            "[BINARY_OR_ARCHIVE_DATA]".to_string()
        } else {
            // PR-CRAWLER-012: Timeout on body read to prevent hang on stalled connections
            match tokio::time::timeout(std::time::Duration::from_secs(15), resp.text()).await {
                Ok(Ok(text)) => text,
                Ok(Err(_)) => "[DECODE_ERROR]".to_string(),
                Err(_) => {
                    println!("[Crawli Fingerprint] body read timed out after 15s");
                    "[BODY_TIMEOUT]".to_string()
                }
            }
        };

        fingerprint_latency_ms = fingerprint_started_at.elapsed().as_millis() as u64;
        app_state
            .telemetry
            .set_fingerprint_latency_ms(fingerprint_latency_ms);

        let captured_fingerprint = adapters::SiteFingerprint {
            url: url.to_string(),
            status,
            headers,
            body,
        };

        if let Some(name) = looks_like_direct_artifact(url, is_binary) {
            timer.emit_log(&format!(
                "[Adapter] Target appears to be a direct file/artifact: {}",
                name
            ));
            if let Some(adapter) = registry.determine_adapter(&captured_fingerprint).await {
                timer.emit_log(&format!("[Adapter] Match found: {}", adapter.name()));
                app.emit(
                    "crawl_log",
                    format!("[Adapter] Match found: {}", adapter.name()),
                )
                .unwrap_or_default();
            } else {
                timer.emit_log(
                    "[Adapter] Match found: Direct Artifact (No specialized adapter match)",
                );
                app.emit(
                    "crawl_log",
                    "[Adapter] Match found: Direct Artifact (No specialized adapter match)"
                        .to_string(),
                )
                .unwrap_or_default();
            }

            let size = captured_fingerprint
                .headers
                .get("content-length")
                .and_then(|h| h.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok());
            let entry = adapters::FileEntry {
                jwt_exp: None,
                path: name.clone(),
                size_bytes: size,
                entry_type: adapters::EntryType::File,
                raw_url: url.to_string(),
            };
            app.emit(
                "crawl_log",
                format!(
                    "[System] Raw File Target Intercepted: {}. Enqueueing directly to Aria Forge.",
                    name
                ),
            )
            .unwrap_or_default();
            let fallback_entries = vec![entry.clone()];
            vfs.insert_entries(&fallback_entries)
                .await
                .map_err(|e| e.to_string())?;
            let _ = app.emit("crawl_progress", fallback_entries.clone());

            let arc_frontier = std::sync::Arc::new(frontier);
            let state = app.state::<AppState>();
            let mut lock = state.active_frontier.lock().await;
            *lock = Some(arc_frontier.clone());
            drop(lock);
            let _ = app.emit(
                "crawl_status_update",
                crawl_status_snapshot(arc_frontier.as_ref(), "complete", 100.0, Some(0)),
            );

            return Ok(CrawlAttemptResult {
                summary: summarize_entry_slice(&fallback_entries),
                was_cancelled: false,
            });
        }

        fingerprint = Some(captured_fingerprint);
    }

    let adapter = if let Some(adapter) = hinted_adapter {
        adapter
    } else {
        let Some(adapter) = registry
            .determine_adapter(fingerprint.as_ref().expect("fingerprint must exist"))
            .await
        else {
            timer.emit_log(
                "[Adapter] CRITICAL FAIL: No known adapter matched this site architecture.",
            );
            return Err("No known adapter matched this sites architecture.".to_string());
        };
        timer.emit_log(&format!(
            "[Fingerprint] Network probe completed in {} ms.",
            fingerprint_latency_ms
        ));
        adapter
    };

    timer.emit_log(&format!(
        "[Adapter] Match found: {}. Executing deep crawl...",
        adapter.name()
    ));
    app.emit(
        "crawl_log",
        format!("[Adapter] Match found: {}", adapter.name()),
    )
    .unwrap_or_default();
    // Phase 141: Set the download feed on the frontier if provided
    if let Some(ref tx) = download_feed_tx {
        frontier.set_download_feed(tx.clone());
    }
    let arc_frontier = std::sync::Arc::new(frontier);
    {
        let state = app.state::<AppState>();
        let mut lock = state.active_frontier.lock().await;
        *lock = Some(arc_frontier.clone());
        *state.current_target_dir.lock().await = Some(target_paths.target_dir.clone());
        *state.current_target_key.lock().await =
            Some(target_paths.target_identity.target_key.clone());
    }

    let _ = app.emit(
        "crawl_status_update",
        crawl_status_snapshot(arc_frontier.as_ref(), "probing", 0.0, None),
    );
    let status_emitter = spawn_crawl_status_emitter(app.clone(), arc_frontier.clone());
    let crawl_result = adapter.crawl(url, arc_frontier.clone(), app.clone()).await;
    status_emitter.abort();
    let _ = status_emitter.await;

    let was_cancelled = arc_frontier.is_cancelled();
    let final_payload = if was_cancelled {
        let snapshot = arc_frontier.progress_snapshot();
        let progress = estimate_progress_percent(
            snapshot.visited.max(1),
            snapshot.processed,
            snapshot.queued,
            snapshot.active_workers,
        );
        crawl_status_snapshot(arc_frontier.as_ref(), "cancelled", progress, None)
    } else if crawl_result.is_ok() {
        crawl_status_snapshot(arc_frontier.as_ref(), "complete", 100.0, Some(0))
    } else {
        let snapshot = arc_frontier.progress_snapshot();
        let progress = estimate_progress_percent(
            snapshot.visited.max(1),
            snapshot.processed,
            snapshot.queued,
            snapshot.active_workers,
        );
        crawl_status_snapshot(arc_frontier.as_ref(), "error", progress, None)
    };
    telemetry_bridge::publish_crawl_status(&app, final_payload);
    {
        let state = app.state::<AppState>();
        state.telemetry.set_worker_metrics(0, 0);
        telemetry_bridge::publish_resource_metrics(&app, state.telemetry.snapshot_counters());
    }
    arc_frontier.clear_adapter_progress();

    match crawl_result {
        Ok(files) => {
            let state = app.state::<AppState>();
            *state.current_target_dir.lock().await = None;
            *state.current_target_key.lock().await = None;
            if !files.is_empty() {
                vfs.insert_entries(&files)
                    .await
                    .map_err(|e| e.to_string())?;
                timer.emit_log("[VFS] Entries successfully committed to local storage.");
            }
            let mut summary = vfs.summarize_entries().await.map_err(|e| e.to_string())?;
            if summary.discovered_count == 0 && !files.is_empty() {
                summary = summarize_entry_slice(&files);
            }
            if summary.discovered_count == files.len() {
                timer.emit_log(&format!(
                    "[Adapter] Crawl complete! Found {} raw entries (files={} folders={}).",
                    files.len(),
                    summary.file_count,
                    summary.folder_count
                ));
            } else {
                timer.emit_log(&format!(
                    "[Adapter] Crawl complete! raw entries={} effective entries={} (files={} folders={}).",
                    files.len(),
                    summary.discovered_count,
                    summary.file_count,
                    summary.folder_count
                ));
            }
            Ok(CrawlAttemptResult {
                summary,
                was_cancelled,
            })
        }
        Err(e) => {
            timer.emit_log(&format!(
                "[Adapter] CRITICAL FAIL during crawl execution: {}",
                e
            ));
            let state = app.state::<AppState>();
            let mut lock = state.active_frontier.lock().await;
            *lock = None;
            *state.current_target_dir.lock().await = None;
            *state.current_target_key.lock().await = None;
            Err(e.to_string())
        }
    }
}

async fn scaffold_download_from_vfs(
    vfs: &db::SledVfs,
    output_root: &std::path::Path,
    app: &tauri::AppHandle,
    connections: usize,
    force_tor: bool,
) -> anyhow::Result<u32> {
    let entries = vfs.iter_entries().await?;
    scaffold_download(&entries, output_root, app, connections, force_tor).await
}

async fn scaffold_download_from_entries_with_plan(
    entries: &[adapters::FileEntry],
    ordered_entries: &[adapters::FileEntry],
    target_paths: &target_state::TargetPaths,
    output_root: &std::path::Path,
    app: &tauri::AppHandle,
    connections: usize,
    force_tor: bool,
) -> anyhow::Result<u32> {
    use anyhow::anyhow;
    use aria_downloader::BatchFileEntry;
    use std::collections::{BTreeMap, HashSet};

    let base = output_root.to_path_buf();
    tokio::fs::create_dir_all(&base).await?;
    let support_resolution = ensure_support_artifact_dir_for_output_root(&base)?;
    let support_dir = support_resolution.path;
    if let Some(note) = support_resolution.note.as_deref() {
        let _ = app.emit("crawl_log", format!("[PATH] {note}"));
    }
    if support_resolution.used_fallback {
        let _ = app.emit(
            "crawl_log",
            format!(
                "[PATH] Support artifacts fell back under output root: {}",
                path_utils::display_path(&support_dir)
            ),
        );
    }
    let ranked_hosts = ranked_qilin_download_hosts(entries);

    let mut written_final: u32 = 0;
    let mut batch_lookup: BTreeMap<String, BatchFileEntry> = BTreeMap::new();
    let mut total_bytes_hint = 0u64;
    let mut unknown_size_files = 0usize;

    for entry in entries {
        let full_path = match path_utils::resolve_path_within_root(
            &base,
            &entry.path,
            matches!(entry.entry_type, adapters::EntryType::Folder),
        ) {
            Ok(Some(path)) => path,
            Ok(None) => continue,
            Err(err) => {
                let _ = app.emit(
                    "crawl_log",
                    format!("[SECURITY] Rejected unsafe path '{}': {}", entry.path, err),
                );
                continue;
            }
        };

        match entry.entry_type {
            adapters::EntryType::Folder => {
                tokio::fs::create_dir_all(&full_path).await?;
                let gitkeep = full_path.join(".gitkeep");
                if !gitkeep.exists() {
                    let _ = tokio::fs::write(&gitkeep, b"").await;
                }
                written_final = written_final.saturating_add(1);
            }
            adapters::EntryType::File => {
                if let Some(parent) = full_path.parent() {
                    tokio::fs::create_dir_all(parent).await?;
                }

                let safe_target = full_path.to_string_lossy().to_string();
                let url_lower = entry.raw_url.to_lowercase();
                if url_lower.starts_with("http://") || url_lower.starts_with("https://") {
                    batch_lookup.insert(
                        entry.path.clone(),
                        build_batch_file_entry(entry, safe_target, &ranked_hosts),
                    );
                    total_bytes_hint =
                        total_bytes_hint.saturating_add(entry.size_bytes.unwrap_or(0));
                    if entry.size_bytes.unwrap_or(0) == 0 {
                        unknown_size_files = unknown_size_files.saturating_add(1);
                    }
                } else {
                    if !full_path.exists() {
                        tokio::fs::write(&full_path, b"").await?;
                    }
                    let _ = app.emit(
                        "crawl_log",
                        format!(
                            "[MIRROR] Placeholder scaffolded for {} (no valid source URL).",
                            entry.path
                        ),
                    );
                    written_final = written_final.saturating_add(1);
                }
            }
        }
    }

    let batch_files: Vec<BatchFileEntry> = ordered_entries
        .iter()
        .filter_map(|entry| batch_lookup.remove(&entry.path))
        .collect();
    write_download_support_artifacts(entries, &support_dir).await?;

    if !batch_files.is_empty() {
        let _ = app.emit(
            "download_batch_started",
            DownloadBatchStartedEvent {
                total_files: batch_files.len(),
                total_bytes_hint,
                unknown_size_files,
                output_dir: base.to_string_lossy().to_string(),
            },
        );
        let _ = app.emit(
            "crawl_log",
            format!(
                "[ARIA] Failure-first batch engaged: {} planned files | Circuits: {}",
                batch_files.len(),
                connections.max(1)
            ),
        );

        let control = aria_downloader::activate_download_control()
            .ok_or_else(|| anyhow!("A download is already active."))?;
        let batch_result = aria_downloader::start_batch_download(
            app.clone(),
            batch_files,
            connections.max(1),
            force_tor,
            Some(base.to_string_lossy().to_string()),
            control,
        )
        .await;
        aria_downloader::clear_download_control();

        let previous_failures =
            target_state::load_failure_manifest(&target_paths.failure_manifest_path)?;
        let next_failures = target_state::reconcile_failure_manifest(
            &previous_failures,
            ordered_entries,
            entries,
            output_root,
            if batch_result.is_err() {
                "failed"
            } else {
                "resume"
            },
        )?;
        target_state::save_failure_manifest(&target_paths.failure_manifest_path, &next_failures)?;

        let unresolved_paths: HashSet<&str> = next_failures
            .iter()
            .map(|record| record.path.as_str())
            .collect();
        let successful_downloads = ordered_entries
            .iter()
            .filter(|entry| !unresolved_paths.contains(entry.path.as_str()))
            .count() as u32;
        written_final = written_final.saturating_add(successful_downloads);

        batch_result?;
    } else {
        let next_failures = target_state::reconcile_failure_manifest(
            &target_state::load_failure_manifest(&target_paths.failure_manifest_path)?,
            &[],
            entries,
            output_root,
            "skipped",
        )?;
        target_state::save_failure_manifest(&target_paths.failure_manifest_path, &next_failures)?;
    }

    Ok(written_final)
}

#[tauri::command]
async fn get_vfs_children(
    parent_path: String,
    app: tauri::AppHandle,
) -> Result<Vec<adapters::FileEntry>, String> {
    let state = app.state::<AppState>();
    state
        .vfs
        .get_children(&parent_path)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn get_adapter_support_catalog() -> Vec<adapters::AdapterSupportInfo> {
    adapters::support_catalog()
}

// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
#[tauri::command]
async fn start_crawl(
    url: String,
    mut options: frontier::CrawlOptions,
    output_dir: String,
    app: tauri::AppHandle,
) -> Result<CrawlSessionResult, String> {
    // Phase 52: Auto-detect Mega.nz and Torrent inputs — route directly
    if mega_handler::is_mega_link(&url) {
        return mega_handler::mega_crawl(
            &url,
            &output_dir,
            options.download,
            options.mega_password.as_deref(),
            app,
        )
        .await;
    }
    if torrent_handler::is_magnet_link(&url) || torrent_handler::is_torrent_file(&url) {
        return torrent_handler::torrent_crawl(&url, &output_dir, options.download, app).await;
    }

    let state = app.state::<AppState>();
    let telemetry = state.telemetry.clone();
    let vfs = state.vfs.clone();
    let _crawl_session_guard = runtime_metrics::CrawlSessionGuard::new(telemetry);
    let output_root = canonical_output_root(&output_dir)?;
    let output_root_str = output_root.to_string_lossy().to_string();
    let auto_download = options.download;
    let target_paths = target_state::target_paths(&output_root, &url).map_err(|e| e.to_string())?;
    let mut ledger =
        target_state::load_or_default_ledger(&target_paths).map_err(|e| e.to_string())?;
    let best_prior_entries = target_state::load_entries_snapshot(&target_paths.best_snapshot_path)
        .map_err(|e| e.to_string())?;
    let best_prior_count = best_prior_entries.len();
    let retry_budget = 2usize;
    let started_at_epoch = chrono::Local::now().timestamp().max(0) as u64;

    // Phase 133 + Phase 140: Activate download mode globally with RAM-aware auto-demotion.
    // On low-RAM systems, Aggressive/Medium modes are automatically demoted to prevent OOM.
    let output_path = std::path::Path::new(&output_dir);
    let (effective_mode, demotion_warning) =
        resource_governor::clamp_mode_for_hardware(options.download_mode, Some(output_path));
    if let Some(warning) = &demotion_warning {
        app.emit("crawl_log", format!("[RAM Guard] {warning}"))
            .unwrap_or_default();
    }
    resource_governor::set_active_download_mode(effective_mode);
    app.emit(
        "crawl_log",
        format!(
            "[System] Download mode: {} (circuits={}, pd_cap={}, workers={})",
            effective_mode,
            effective_mode.default_circuits(),
            effective_mode.parallel_download_cap(),
            effective_mode.crawl_worker_ceiling(),
        ),
    )
    .unwrap_or_default();

    if options.resume && options.resume_index.is_none() {
        if best_prior_count > 0 && target_paths.stable_best_dirs_listing_path.exists() {
            options.resume_index = Some(
                target_paths
                    .stable_best_dirs_listing_path
                    .to_string_lossy()
                    .to_string(),
            );
        } else if target_paths.stable_current_dirs_listing_path.exists() {
            options.resume_index = Some(
                target_paths
                    .stable_current_dirs_listing_path
                    .to_string_lossy()
                    .to_string(),
            );
        }

        if options.resume_index.is_some() {
            app.emit(
                "crawl_log",
                format!("[SYSTEM] 🔭 Phase 68 Ledger Resume: Auto-injecting persistent snapshot across system restarts.")
            ).unwrap_or_default();
        }
    }

    if options.resume_index.is_some() {
        app.emit(
            "crawl_log",
            "[SYSTEM] Advanced baseline override active: manual resume index selected.".to_string(),
        )
        .unwrap_or_default();
    } else if best_prior_count > 0 {
        app.emit(
            "crawl_log",
            format!(
                "[SYSTEM] Auto baseline loaded for {}: {} best-known entries.",
                target_paths.target_identity.target_key, best_prior_count
            ),
        )
        .unwrap_or_default();
    }

    let mut final_attempt_result: Option<CrawlAttemptResult> = None;
    let mut final_entries: Vec<adapters::FileEntry> = Vec::new();
    let mut final_merged_entries = best_prior_entries.clone();
    let mut final_instability_reasons = Vec::new();
    let mut retry_count_used = 0usize;

    // Phase 141: Event-driven parallel download pipeline.
    // Instead of polling the VFS every 10s (O(N) scan with 200K+ entries),
    // we create a channel that adapters push FileEntry into as they discover them.
    // The consumer processes entries in chunks of 100, starting immediately.
    //
    // Architecture:
    //   Adapter → ui_tx → VFS Flush Task → download_feed_tx → Download Consumer
    //                                                          ↓
    //                                              scaffold_download(chunk)
    //
    // Benefits:
    //   1. Zero VFS scanning overhead (was O(N) per 10s cycle)
    //   2. Discovery-to-download latency: 25s → <3s
    //   3. Natural backpressure through channel buffer
    //   4. Chunked processing: download chunk 1 while chunk 2 accumulates
    let parallel_download_cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    // Phase 141: Create the download feed channel in start_crawl scope
    let (download_feed_tx_raw, download_feed_rx) = tokio::sync::mpsc::unbounded_channel::<adapters::FileEntry>();
    let download_feed_tx = std::sync::Arc::new(download_feed_tx_raw);

    let parallel_download_handle: Option<tokio::task::JoinHandle<()>> = if options.parallel_download && options.download {
        let pd_app = app.clone();
        let pd_output = output_root.clone();
        let pd_target_dir = target_paths.target_dir.clone();
        let pd_cancel = parallel_download_cancel.clone();
        let pd_vfs = vfs.clone(); // Keep VFS ref for final sweep
        // Phase 140D: Use full mode budget
        let pd_cap = effective_mode.parallel_download_cap();
        let pd_circuits = options.circuits.unwrap_or(effective_mode.default_circuits()).max(1).min(pd_cap);
        let pd_is_onion = url_targets_onion(&url) && !options.force_clearnet;
        let mut pd_rx = download_feed_rx;

        app.emit(
            "crawl_log",
            format!(
                "[PARALLEL] ⚡ Phase 141: Event-driven download consumer armed ({} circuits, {} cap). Chunk size: 100. Speed threshold: 0.3 MB/s.",
                pd_circuits, pd_cap
            ),
        )
        .unwrap_or_default();

        Some(tokio::spawn(async move {
            use tauri::Emitter;
            let mut downloaded_paths = std::collections::HashSet::<String>::new();
            let mut batch_round = 0u32;
            let mut pending_entries: Vec<adapters::FileEntry> = Vec::new();
            let chunk_size: usize = 100;

            // Phase 141B: Stall detection state
            let mut last_progress_at = std::time::Instant::now();
            let mut last_downloaded_count: usize = 0;
            let mut stall_recoveries: u8 = 0;
            let stall_threshold = std::time::Duration::from_secs(90);
            let max_recoveries: u8 = 3;

            // Phase 141: Small initial delay to let circuits warm up
            tokio::time::sleep(std::time::Duration::from_secs(8)).await;

            loop {
                if pd_cancel.load(std::sync::atomic::Ordering::Relaxed) {
                    break;
                }

                // Phase 141B: Check for stall before draining new entries
                if last_progress_at.elapsed() > stall_threshold && downloaded_paths.len() > last_downloaded_count {
                    // Downloads were made but none recently — we're stalled
                    if stall_recoveries < max_recoveries {
                        stall_recoveries += 1;
                        let _ = pd_app.emit(
                            "crawl_log",
                            format!(
                                "[PARALLEL] ⚠️ Stall detected (no progress for {}s). Recovery {}/{}: refreshing Tor circuits + 30s cooldown...",
                                stall_threshold.as_secs(), stall_recoveries, max_recoveries
                            ),
                        );

                        // NEWNYM: Request fresh circuits on all managed Tor daemons
                        let active_ports = crate::tor::detect_active_managed_tor_ports();
                        for port in active_ports {
                            let _ = crate::tor::request_newnym(port).await;
                        }

                        // 30s cooldown — let new circuits establish
                        let _ = pd_app.emit(
                            "crawl_log",
                            "[PARALLEL] 🔄 Circuits refreshed. Pausing 30s for new paths to establish...".to_string(),
                        );
                        tokio::time::sleep(std::time::Duration::from_secs(30)).await;

                        // Reset progress tracker
                        last_progress_at = std::time::Instant::now();
                        last_downloaded_count = downloaded_paths.len();

                        let _ = pd_app.emit(
                            "crawl_log",
                            format!(
                                "[PARALLEL] ✅ Recovery {} complete. Resuming downloads (already completed: {} files).",
                                stall_recoveries, downloaded_paths.len()
                            ),
                        );
                        continue; // Re-enter the loop with fresh circuits
                    } else {
                        let _ = pd_app.emit(
                            "crawl_log",
                            format!(
                                "[PARALLEL] ❌ Max recoveries ({}) exhausted. Falling through to post-crawl sweep.",
                                max_recoveries
                            ),
                        );
                        break;
                    }
                }

                // Phase 141: Drain all available entries from the channel (non-blocking)
                let drained = {
                    let mut buf = Vec::with_capacity(500);
                    // First: try to get at least one entry (with timeout so we don't block forever)
                    if pending_entries.is_empty() {
                        match tokio::time::timeout(
                            std::time::Duration::from_secs(5),
                            pd_rx.recv(),
                        ).await {
                            Ok(Some(entry)) => buf.push(entry),
                            Ok(None) => {
                                // Channel closed — adapter is done. Do a final VFS sweep.
                                break;
                            }
                            Err(_) => {
                                // Timeout — no new entries in 5s, check if we should exit
                                if pd_cancel.load(std::sync::atomic::Ordering::Relaxed) {
                                    break;
                                }
                                continue;
                            }
                        }
                    }
                    // Then: drain any others that are already buffered (non-blocking)
                    while let Ok(entry) = pd_rx.try_recv() {
                        buf.push(entry);
                        if buf.len() >= 500 {
                            break;
                        }
                    }
                    buf
                };

                // Filter: only files with valid URLs, not already downloaded
                for entry in drained {
                    if matches!(entry.entry_type, adapters::EntryType::File)
                        && !entry.raw_url.is_empty()
                        && (entry.raw_url.starts_with("http://") || entry.raw_url.starts_with("https://"))
                        && !downloaded_paths.contains(&entry.path)
                    {
                        pending_entries.push(entry);
                    }
                }

                if pending_entries.is_empty() {
                    continue;
                }

                // Phase 141: Process in chunks of 100 — start downloading immediately
                // while more entries continue to arrive via the channel
                while !pending_entries.is_empty() && !pd_cancel.load(std::sync::atomic::Ordering::Relaxed) {
                    let chunk: Vec<adapters::FileEntry> = pending_entries
                        .drain(..pending_entries.len().min(chunk_size))
                        .collect();

                    // Sort chunk by size: small files first for rapid progress
                    let mut sorted_chunk = chunk;
                    sorted_chunk.sort_by_key(|e| e.size_bytes.unwrap_or(u64::MAX));

                    batch_round += 1;
                    let chunk_len = sorted_chunk.len();
                    for e in &sorted_chunk {
                        downloaded_paths.insert(e.path.clone());
                    }

                    let _ = pd_app.emit(
                        "crawl_log",
                        format!(
                            "[PARALLEL] ⚡ Chunk #{}: {} files (total downloaded: {})",
                            batch_round, chunk_len, downloaded_paths.len()
                        ),
                    );

                    // Repin Qilin URLs if route summary exists
                    let _ = maybe_repin_qilin_entries_from_context(
                        &mut sorted_chunk,
                        Some(pd_target_dir.as_path()),
                        &pd_app,
                    );

                    match scaffold_download(
                        &sorted_chunk,
                        &pd_output,
                        &pd_app,
                        pd_circuits,
                        pd_is_onion,
                    )
                    .await
                    {
                        Ok(count) => {
                            // Phase 141B: Update progress tracker on success
                            if count > 0 {
                                last_progress_at = std::time::Instant::now();
                                last_downloaded_count = downloaded_paths.len();
                            }
                            let _ = pd_app.emit(
                                "crawl_log",
                                format!("[PARALLEL] ⚡ Chunk #{} complete: {} items", batch_round, count),
                            );
                        }
                        Err(e) => {
                            let _ = pd_app.emit(
                                "crawl_log",
                                format!("[PARALLEL] Chunk #{} error: {}", batch_round, e),
                            );
                        }
                    }

                    // Drain more entries that arrived during this chunk's download
                    while let Ok(entry) = pd_rx.try_recv() {
                        if matches!(entry.entry_type, adapters::EntryType::File)
                            && !entry.raw_url.is_empty()
                            && (entry.raw_url.starts_with("http://") || entry.raw_url.starts_with("https://"))
                            && !downloaded_paths.contains(&entry.path)
                        {
                            pending_entries.push(entry);
                        }
                    }
                }
            }

            // Phase 141: Final VFS sweep — catch any entries the channel missed
            // (e.g., non-Qilin adapters that insert directly into VFS without the channel)
            if !pd_cancel.load(std::sync::atomic::Ordering::Relaxed) {
                if let Ok(all_entries) = pd_vfs.iter_entries().await {
                    let mut missed: Vec<adapters::FileEntry> = all_entries
                        .into_iter()
                        .filter(|e| {
                            matches!(e.entry_type, adapters::EntryType::File)
                                && !e.raw_url.is_empty()
                                && (e.raw_url.starts_with("http://") || e.raw_url.starts_with("https://"))
                                && !downloaded_paths.contains(&e.path)
                        })
                        .collect();

                    if !missed.is_empty() {
                        missed.sort_by_key(|e| e.size_bytes.unwrap_or(u64::MAX));
                        batch_round += 1;
                        let sweep_count = missed.len();
                        for e in &missed {
                            downloaded_paths.insert(e.path.clone());
                        }
                        let _ = pd_app.emit(
                            "crawl_log",
                            format!(
                                "[PARALLEL] 🧹 Final sweep: {} files missed by channel (total: {})",
                                sweep_count, downloaded_paths.len()
                            ),
                        );
                        let _ = scaffold_download(
                            &missed,
                            &pd_output,
                            &pd_app,
                            pd_circuits,
                            pd_is_onion,
                        )
                        .await;
                    }
                }
            }
        }))
    } else {
        // Drop the receiver so the sender doesn't block
        drop(download_feed_rx);
        None
    };

    for attempt_idx in 0..=retry_budget {
        if attempt_idx > 0 {
            retry_count_used = attempt_idx;
            app.emit(
                "crawl_log",
                format!(
                    "[SYSTEM] Baseline catch-up retry {}/{} engaged for {}.",
                    attempt_idx, retry_budget, target_paths.target_identity.target_key
                ),
            )
            .unwrap_or_default();
        }

        let active_ledger = std::sync::Arc::new(ledger.clone());
        let attempt_result = match execute_crawl_attempt(
            &url,
            &options,
            &output_root,
            &target_paths,
            &app,
            &vfs,
            active_ledger,
            if options.parallel_download && options.download { Some(download_feed_tx.clone()) } else { None },
        )
        .await
        {
            Ok(res) => res,
            Err(e) => return Err(e.to_string()),
        };
        let current_entries = vfs.iter_entries().await.map_err(|e| e.to_string())?;
        let raw_this_run_count = current_entries.len();
        let telemetry_snapshot = state.telemetry.snapshot_counters();
        let mut instability_reasons = Vec::new();
        if telemetry_snapshot.timeout_count > 0 {
            instability_reasons.push(format!("timeouts={}", telemetry_snapshot.timeout_count));
        }
        if telemetry_snapshot.throttle_count > 0 {
            instability_reasons.push(format!("throttles={}", telemetry_snapshot.throttle_count));
        }
        if telemetry_snapshot.node_failovers > 0 {
            instability_reasons.push(format!("failovers={}", telemetry_snapshot.node_failovers));
        }
        if attempt_result.was_cancelled {
            instability_reasons.push("cancelled".to_string());
        }

        let merged_entries = target_state::merge_entries(&best_prior_entries, &current_entries);
        let should_retry = attempt_idx < retry_budget
            && best_prior_count > 0
            && raw_this_run_count < best_prior_count
            && !attempt_result.was_cancelled
            && !instability_reasons.is_empty();

        if should_retry {
            app.emit(
                "crawl_log",
                format!(
                    "[SYSTEM] Crawl underperformed baseline for {} (raw {} < best {}). Retrying after instability: {}",
                    target_paths.target_identity.target_key,
                    raw_this_run_count,
                    best_prior_count,
                    instability_reasons.join(", ")
                ),
            )
            .unwrap_or_default();
            continue;
        }

        final_instability_reasons = instability_reasons;
        final_merged_entries = merged_entries;
        final_entries = current_entries;
        final_attempt_result = Some(attempt_result);
        break;
    }

    // Phase 128: Signal parallel download consumer to stop after crawl completes
    parallel_download_cancel.store(true, std::sync::atomic::Ordering::Relaxed);

    let final_attempt_result = final_attempt_result.ok_or_else(|| {
        "No crawl attempt result was produced for baseline evaluation.".to_string()
    })?;
    let raw_this_run_count = final_entries.len();
    let merged_effective_count = final_merged_entries.len();
    let crawl_outcome = if best_prior_count == 0 {
        target_state::CrawlOutcome::FirstRun
    } else if merged_effective_count > best_prior_count {
        target_state::CrawlOutcome::ExceededBest
    } else if raw_this_run_count < best_prior_count && !final_instability_reasons.is_empty() {
        target_state::CrawlOutcome::Degraded
    } else {
        target_state::CrawlOutcome::MatchedBest
    };

    let authoritative_entries = if matches!(
        crawl_outcome,
        target_state::CrawlOutcome::FirstRun | target_state::CrawlOutcome::ExceededBest
    ) {
        final_merged_entries.clone()
    } else if !best_prior_entries.is_empty() {
        best_prior_entries.clone()
    } else {
        final_entries.clone()
    };

    target_state::save_entries_snapshot(&target_paths.current_snapshot_path, &final_entries)
        .map_err(|e| e.to_string())?;
    let listing_paths =
        target_state::write_current_and_history_listings(&target_paths, &final_entries, &url)
            .map_err(|e| e.to_string())?;

    if matches!(
        crawl_outcome,
        target_state::CrawlOutcome::FirstRun | target_state::CrawlOutcome::ExceededBest
    ) || !target_paths.best_snapshot_path.exists()
    {
        target_state::save_entries_snapshot(
            &target_paths.best_snapshot_path,
            &authoritative_entries,
        )
        .map_err(|e| e.to_string())?;
        target_state::write_best_listings(&target_paths, &authoritative_entries, &url)
            .map_err(|e| e.to_string())?;

        let _ = crate::index_generator::generate_index_html(
            &target_paths.target_identity.target_key,
            &target_paths.target_dir.join("index.html"),
            &target_paths.stable_best_listing_path,
        );
        ledger.best_snapshot_version = ledger.best_snapshot_version.saturating_add(1);
    }

    let finished_at_epoch = chrono::Local::now().timestamp().max(0) as u64;
    target_state::append_run_record(
        &mut ledger,
        target_state::CrawlRunRecord {
            started_at_epoch,
            finished_at_epoch,
            raw_this_run_count,
            best_prior_count,
            merged_effective_count,
            outcome: target_state::crawl_outcome_label(crawl_outcome.clone()).to_string(),
            retry_count_used,
            instability_reasons: final_instability_reasons.clone(),
            current_listing_path: listing_paths
                .current_canonical_path
                .to_string_lossy()
                .to_string(),
            current_dirs_listing_path: listing_paths
                .current_dirs_path
                .to_string_lossy()
                .to_string(),
            history_canonical_path: listing_paths
                .history_canonical_path
                .to_string_lossy()
                .to_string(),
            history_dirs_path: listing_paths
                .history_dirs_path
                .to_string_lossy()
                .to_string(),
        },
        crawl_outcome.clone(),
        authoritative_entries.len(),
    );
    target_state::save_ledger(&target_paths, &ledger).map_err(|e| e.to_string())?;

    app.emit(
        "crawl_log",
        format!(
            "[SYSTEM] Stable listings ready: {} | {} | {} | {}",
            target_paths.stable_current_listing_path.display(),
            target_paths.stable_current_dirs_listing_path.display(),
            target_paths.stable_best_listing_path.display(),
            target_paths.stable_best_dirs_listing_path.display(),
        ),
    )
    .unwrap_or_default();

    let mut auto_download_started = false;

    // Phase 128 + Phase 140: Wait for parallel download consumer with hard timeout.
    // Without this timeout, the consumer can hang indefinitely when the last few files
    // are stuck in retry loops (503 throttles, connection drops). The post-crawl sweep
    // via build_download_resume_plan() will pick up any remaining files.
    if let Some(handle) = parallel_download_handle {
        app.emit(
            "crawl_log",
            "[PARALLEL] Waiting for parallel download consumer to finish (120s max)...".to_string(),
        )
        .unwrap_or_default();
        match tokio::time::timeout(std::time::Duration::from_secs(120), handle).await {
            Ok(Ok(_)) => {
                auto_download_started = true;
                app.emit(
                    "crawl_log",
                    "[PARALLEL] ⚡ Parallel download consumer finished.".to_string(),
                )
                .unwrap_or_default();
            }
            Ok(Err(e)) => {
                app.emit(
                    "crawl_log",
                    format!("[PARALLEL] Consumer task error: {}", e),
                )
                .unwrap_or_default();
            }
            Err(_) => {
                // Timeout: consumer is stuck retrying. The post-crawl sweep will handle
                // remaining files via build_download_resume_plan().
                app.emit(
                    "crawl_log",
                    "[PARALLEL] ⚠️ Consumer timed out after 120s. Post-crawl sweep will handle remaining files.".to_string(),
                )
                .unwrap_or_default();
            }
        }
    }

    // Phase 128: Auto download runs as post-crawl final sweep.
    // Resume plan naturally skips files already downloaded by the parallel consumer.
    if auto_download {
        let failure_records =
            target_state::load_failure_manifest(&target_paths.failure_manifest_path)
                .map_err(|e| e.to_string())?;
        let resume_build = target_state::build_download_resume_plan(
            &target_paths.target_identity.target_key,
            &authoritative_entries,
            &failure_records,
            &output_root,
            &target_paths.failure_manifest_path,
        )
        .map_err(|e| e.to_string())?;
        target_state::save_resume_plan(&target_paths.latest_resume_plan_path, &resume_build.plan)
            .map_err(|e| e.to_string())?;
        let _ = app.emit("download_resume_plan", resume_build.plan.clone());

        if resume_build.plan.all_items_skipped {
            app.emit(
                "crawl_log",
                format!(
                    "[OPSEC] Auto-Mirror skipped for {}: all items already complete.",
                    target_paths.target_identity.target_key
                ),
            )
            .unwrap_or_default();
            telemetry_bridge::publish_batch_progress(
                &app,
                telemetry_bridge::BridgeBatchProgress {
                    completed: resume_build.plan.skipped_exact_matches_count,
                    failed: 0,
                    total: resume_build.plan.skipped_exact_matches_count,
                    current_file: "All items skipped".to_string(),
                    speed_mbps: 0.0,
                    downloaded_bytes: 0,
                    active_circuits: Some(0),
                    bbr_bottleneck_mbps: None,
                    ekf_covariance: None,
                },
            );
        } else {
            let circuits = options.circuits.unwrap_or(120).max(1);
            app.emit(
                "crawl_log",
                format!(
                    "[OPSEC] Auto-Mirror engaged. Failures first: {} | Missing/Mismatch: {} | Skipped exact: {}",
                    resume_build.plan.failed_first_count,
                    resume_build.plan.missing_or_mismatch_count,
                    resume_build.plan.skipped_exact_matches_count
                ),
            )
            .unwrap_or_default();
            let is_onion = url_targets_onion(&url) && !options.force_clearnet;
            match scaffold_download_from_entries_with_plan(
                &authoritative_entries,
                &resume_build.ordered_entries,
                &target_paths,
                &output_root,
                &app,
                circuits,
                is_onion,
            )
            .await
            {
                Ok(count) => {
                    auto_download_started = true;
                    app.emit(
                        "crawl_log",
                        format!("[OPSEC] Mirror complete. {} items written to disk.", count),
                    )
                    .unwrap_or_default();
                }
                Err(e) => {
                    app.emit("crawl_log", format!("[ERROR] Mirror failed: {}", e))
                        .unwrap_or_default();
                }
            }
        }
    }

    // Phase 72: Trigger Aerospace-Grade VFS Leveling (Sled compact bounds + wait layer sync)
    // Runs right before session shutdown blocks to guarantee no dangling memory artifacts
    let _ = vfs.compact_database().await;

    Ok(CrawlSessionResult {
        target_key: target_paths.target_identity.target_key.clone(),
        discovered_count: final_attempt_result.summary.discovered_count,
        file_count: final_attempt_result.summary.file_count,
        folder_count: final_attempt_result.summary.folder_count,
        best_prior_count,
        raw_this_run_count,
        merged_effective_count,
        crawl_outcome: target_state::crawl_outcome_label(crawl_outcome).to_string(),
        retry_count_used,
        stable_current_listing_path: target_paths
            .stable_current_listing_path
            .to_string_lossy()
            .to_string(),
        stable_current_dirs_listing_path: target_paths
            .stable_current_dirs_listing_path
            .to_string_lossy()
            .to_string(),
        stable_best_listing_path: target_paths
            .stable_best_listing_path
            .to_string_lossy()
            .to_string(),
        stable_best_dirs_listing_path: target_paths
            .stable_best_dirs_listing_path
            .to_string_lossy()
            .to_string(),
        auto_download_started,
        output_dir: output_root_str,
    })
}

pub async fn start_crawl_for_example(
    url: String,
    options: frontier::CrawlOptions,
    output_dir: String,
    app: tauri::AppHandle,
) -> Result<CrawlSessionResult, String> {
    start_crawl(url, options, output_dir, app).await
}

/// Sled IPC Receiver: Persist incoming entries silently in the background
#[tauri::command]
async fn ingest_vfs_entries(
    entries: Vec<adapters::FileEntry>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    state
        .vfs
        .insert_entries(&entries)
        .await
        .map_err(|e| e.to_string())
}

/// Mirrors all discovered entries onto disk.
/// - Folders are scaffolded immediately.
/// - Files with valid HTTP(S) URLs are routed through Aria batch download.
/// - Files without valid URLs are scaffolded as placeholders.
#[tauri::command]
async fn download_files(
    entries: Vec<adapters::FileEntry>,
    output_dir: String,
    connections: Option<usize>,
    app: tauri::AppHandle,
) -> Result<u32, String> {
    let output_root = canonical_output_root(&output_dir)?;
    let force_tor = entries
        .iter()
        .any(|entry| url_targets_onion(&entry.raw_url));
    let circuits = connections.unwrap_or(120).max(1);
    scaffold_download(&entries, &output_root, &app, circuits, force_tor)
        .await
        .map_err(|e| e.to_string())
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedQilinSubtreeRouteSummary {
    #[allow(dead_code)]
    updated_at_epoch: u64,
    winner_host: Option<String>,
    entries: Vec<PersistedQilinSubtreeRouteEntry>,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedQilinSubtreeRouteEntry {
    subtree_key: String,
    preferred_host: String,
    #[allow(dead_code)]
    success_count: u32,
    #[allow(dead_code)]
    last_success_epoch: u64,
}

fn find_qilin_route_summary_path(context_path: &std::path::Path) -> Option<std::path::PathBuf> {
    let mut cursor = if context_path.is_dir() {
        Some(context_path)
    } else {
        context_path.parent()
    };

    for _ in 0..6 {
        let current = cursor?;
        let candidate = current.join("qilin_subtree_route_summary.json");
        if candidate.exists() {
            return Some(candidate);
        }
        cursor = current.parent();
    }

    None
}

fn split_qilin_download_seed_and_relative_path(url: &str) -> Option<(String, String)> {
    let parsed = reqwest::Url::parse(url).ok()?;
    let segments: Vec<_> = parsed.path_segments()?.collect();
    let uuid_idx = segments.iter().position(|segment| {
        segment.len() == 36 && segment.chars().filter(|c| *c == '-').count() == 4
    })?;

    let mut seed_path = String::new();
    for segment in segments.iter().take(uuid_idx + 1) {
        seed_path.push('/');
        seed_path.push_str(segment);
    }
    seed_path.push('/');

    let seed_url = format!("{}://{}{}", parsed.scheme(), parsed.host_str()?, seed_path);
    let relative_path = segments
        .iter()
        .skip(uuid_idx + 1)
        .filter(|segment| !segment.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join("/");
    Some((seed_url, relative_path))
}

fn rewrite_qilin_seed_host(raw_url: &str, preferred_host: &str) -> Option<String> {
    let (seed_url, _) = split_qilin_download_seed_and_relative_path(raw_url)?;
    let parsed_seed = reqwest::Url::parse(&seed_url).ok()?;
    let current_host = parsed_seed.host_str()?.trim();
    if current_host.eq_ignore_ascii_case(preferred_host) {
        return None;
    }
    let next_seed = seed_url.replacen(current_host, preferred_host, 1);
    Some(raw_url.replacen(&seed_url, &next_seed, 1))
}

fn subtree_key_for_qilin_download(raw_url: &str) -> Option<String> {
    let (_, relative_path) = split_qilin_download_seed_and_relative_path(raw_url)?;
    let trimmed = relative_path.trim_matches('/');
    if trimmed.is_empty() {
        return None;
    }
    let subtree = trimmed
        .rsplit_once('/')
        .map(|(parent, _)| parent)
        .unwrap_or(trimmed);
    if subtree.is_empty() {
        None
    } else {
        Some(subtree.to_string())
    }
}

#[cfg(test)]
fn preferred_qilin_host_for_subtree(
    subtree: &str,
    routes_by_subtree: &std::collections::HashMap<String, String>,
    winner_host: Option<&str>,
) -> Option<String> {
    let mut cursor = Some(subtree);
    while let Some(candidate) = cursor {
        if let Some(host) = routes_by_subtree.get(candidate) {
            return Some(host.clone());
        }
        cursor = candidate.rsplit_once('/').map(|(parent, _)| parent);
    }
    winner_host.map(|host| host.to_string())
}

#[cfg(test)]
fn preferred_qilin_download_host_for_subtree(
    subtree: &str,
    routes_by_subtree: &std::collections::HashMap<String, PersistedQilinSubtreeRouteEntry>,
    winner_host: Option<&str>,
) -> Option<String> {
    let mut cursor = Some(subtree);
    while let Some(candidate) = cursor {
        if let Some(route) = routes_by_subtree.get(candidate) {
            return Some(route.preferred_host.clone());
        }
        cursor = candidate.rsplit_once('/').map(|(parent, _)| parent);
    }
    winner_host.map(|host| host.to_string())
}

fn should_repin_qilin_download_to_preferred_host(
    route: &PersistedQilinSubtreeRouteEntry,
    winner_host: Option<&str>,
    updated_at_epoch: u64,
) -> bool {
    let route_age = updated_at_epoch.saturating_sub(route.last_success_epoch);
    if route.success_count >= 4 && route_age <= 900 {
        return true;
    }

    winner_host
        .map(|winner| winner.eq_ignore_ascii_case(&route.preferred_host))
        .unwrap_or(false)
        && route.success_count >= 2
        && route_age <= 180
}

fn balanced_qilin_repin_host_cap(total_files: usize, distinct_hosts: usize) -> usize {
    if distinct_hosts <= 1 {
        return total_files.max(1);
    }

    let average = total_files.div_ceil(distinct_hosts).max(1);
    average
        .saturating_add((average / 4).max(1))
        .saturating_add(24)
}

fn maybe_repin_qilin_entries_from_context(
    entries: &mut [adapters::FileEntry],
    context_path: Option<&std::path::Path>,
    app: &tauri::AppHandle,
) -> anyhow::Result<usize> {
    let Some(context_path) = context_path else {
        return Ok(0);
    };
    let Some(route_summary_path) = find_qilin_route_summary_path(context_path) else {
        return Ok(0);
    };

    let content = std::fs::read_to_string(&route_summary_path)?;
    let summary: PersistedQilinSubtreeRouteSummary = serde_json::from_str(&content)?;
    let winner_host = summary.winner_host.clone();
    let updated_at_epoch = summary.updated_at_epoch;
    let routes_by_subtree: std::collections::HashMap<String, PersistedQilinSubtreeRouteEntry> =
        summary
            .entries
            .into_iter()
            .map(|entry| (entry.subtree_key.clone(), entry))
            .collect();
    let mut host_counts = std::collections::HashMap::<String, usize>::new();
    let mut total_onion_files = 0usize;
    for entry in entries.iter() {
        if !matches!(entry.entry_type, adapters::EntryType::File)
            || !url_targets_onion(&entry.raw_url)
        {
            continue;
        }
        if let Some(host) = reqwest::Url::parse(&entry.raw_url)
            .ok()
            .and_then(|url| url.host_str().map(|host| host.to_ascii_lowercase()))
        {
            *host_counts.entry(host).or_default() += 1;
            total_onion_files = total_onion_files.saturating_add(1);
        }
    }
    let host_cap = balanced_qilin_repin_host_cap(total_onion_files, host_counts.len().max(1));

    let mut rewrites = 0usize;
    for entry in entries.iter_mut() {
        if !matches!(entry.entry_type, adapters::EntryType::File)
            || !url_targets_onion(&entry.raw_url)
        {
            continue;
        }
        let Some(subtree_key) = subtree_key_for_qilin_download(&entry.raw_url) else {
            continue;
        };
        let Some(route) = routes_by_subtree.get(&subtree_key).or_else(|| {
            let mut cursor = subtree_key.rsplit_once('/').map(|(parent, _)| parent);
            while let Some(candidate) = cursor {
                if let Some(route) = routes_by_subtree.get(candidate) {
                    return Some(route);
                }
                cursor = candidate.rsplit_once('/').map(|(parent, _)| parent);
            }
            None
        }) else {
            continue;
        };
        if !should_repin_qilin_download_to_preferred_host(
            route,
            winner_host.as_deref(),
            updated_at_epoch,
        ) {
            continue;
        }
        let preferred_host = route.preferred_host.to_ascii_lowercase();
        let Some(current_host) = reqwest::Url::parse(&entry.raw_url)
            .ok()
            .and_then(|url| url.host_str().map(|host| host.to_ascii_lowercase()))
        else {
            continue;
        };
        if preferred_host == current_host {
            continue;
        }
        if host_counts.get(&preferred_host).copied().unwrap_or(0) >= host_cap {
            continue;
        }
        let Some(remapped) = rewrite_qilin_seed_host(&entry.raw_url, &preferred_host) else {
            continue;
        };
        if let Some(count) = host_counts.get_mut(&current_host) {
            *count = count.saturating_sub(1);
        }
        *host_counts.entry(preferred_host).or_default() += 1;
        entry.raw_url = remapped;
        rewrites = rewrites.saturating_add(1);
    }

    if rewrites > 0 {
        let winner_host = winner_host.unwrap_or_else(|| "-".to_string());
        let _ = app.emit(
            "crawl_log",
            format!(
                "[Qilin Download] Repinned {} saved URLs using subtree route memory with host_cap={} (winner host {}).",
                rewrites, host_cap, winner_host
            ),
        );
    }

    Ok(rewrites)
}

#[tauri::command]
fn get_native_smoke_config() -> NativeWebviewSmokeConfig {
    NativeWebviewSmokeConfig {
        enabled: native_webview_smoke_enabled(),
        report_path: native_webview_smoke_report_path()
            .map(|path| path.to_string_lossy().to_string()),
        auto_exit: native_webview_smoke_auto_exit(),
        wait_ms: native_webview_smoke_wait_ms(),
        expected_test_ids: native_webview_smoke_expected_test_ids(),
    }
}

#[tauri::command]
async fn report_native_smoke_result(
    result: NativeWebviewSmokeResult,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let Some(report_path) = native_webview_smoke_report_path() else {
        return Ok(());
    };

    if let Some(parent) = report_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| e.to_string())?;
    }

    let json = serde_json::to_vec_pretty(&result).map_err(|e| e.to_string())?;
    tokio::fs::write(&report_path, json)
        .await
        .map_err(|e| e.to_string())?;

    let _ = app.emit(
        "log",
        format!(
            "[NATIVE_SMOKE] mounted={} missing={}",
            result.mounted,
            result.missing_test_ids.len()
        ),
    );

    if native_webview_smoke_auto_exit() {
        app.exit(0);
    }

    Ok(())
}

async fn scaffold_download(
    entries: &[adapters::FileEntry],
    output_root: &std::path::Path,
    app: &tauri::AppHandle,
    connections: usize,
    force_tor: bool,
) -> anyhow::Result<u32> {
    use anyhow::anyhow;
    use aria_downloader::BatchFileEntry;

    let base = output_root.to_path_buf();
    tokio::fs::create_dir_all(&base).await?;
    let support_resolution = ensure_support_artifact_dir_for_output_root(&base)?;
    let support_dir = support_resolution.path;
    if let Some(note) = support_resolution.note.as_deref() {
        let _ = app.emit("crawl_log", format!("[PATH] {note}"));
    }
    if support_resolution.used_fallback {
        let _ = app.emit(
            "crawl_log",
            format!(
                "[PATH] Support artifacts fell back under output root: {}",
                path_utils::display_path(&support_dir)
            ),
        );
    }
    let ranked_hosts = ranked_qilin_download_hosts(entries);

    let mut written_final: u32 = 0;
    let mut batch_files: Vec<BatchFileEntry> = Vec::new();

    for entry in entries {
        let full_path = match path_utils::resolve_path_within_root(
            &base,
            &entry.path,
            matches!(entry.entry_type, adapters::EntryType::Folder),
        ) {
            Ok(Some(path)) => path,
            Ok(None) => continue,
            Err(err) => {
                let _ = app.emit(
                    "crawl_log",
                    format!("[SECURITY] Rejected unsafe path '{}': {}", entry.path, err),
                );
                continue;
            }
        };

        match entry.entry_type {
            adapters::EntryType::Folder => {
                tokio::fs::create_dir_all(&full_path).await?;
                let gitkeep = full_path.join(".gitkeep");
                if !gitkeep.exists() {
                    let _ = tokio::fs::write(&gitkeep, b"").await;
                }
                written_final = written_final.saturating_add(1);
            }
            adapters::EntryType::File => {
                if let Some(parent) = full_path.parent() {
                    tokio::fs::create_dir_all(parent).await?;
                }

                let safe_target = full_path.to_string_lossy().to_string();
                let url_lower = entry.raw_url.to_lowercase();
                if url_lower.starts_with("http://") || url_lower.starts_with("https://") {
                    batch_files.push(build_batch_file_entry(entry, safe_target, &ranked_hosts));
                } else {
                    if !full_path.exists() {
                        tokio::fs::write(&full_path, b"").await?;
                    }
                    let _ = app.emit(
                        "crawl_log",
                        format!(
                            "[MIRROR] Placeholder scaffolded for {} (no valid source URL).",
                            entry.path
                        ),
                    );
                    written_final = written_final.saturating_add(1);
                }
            }
        }
    }
    write_download_support_artifacts(entries, &support_dir).await?;

    if !batch_files.is_empty() {
        let batch_count = batch_files.len();
        let total_bytes_hint = entries
            .iter()
            .filter_map(|entry| entry.size_bytes)
            .sum::<u64>();
        let unknown_size_files = entries
            .iter()
            .filter(|entry| {
                matches!(entry.entry_type, adapters::EntryType::File)
                    && entry.size_bytes.unwrap_or(0) == 0
            })
            .count();
        let _ = app.emit(
            "download_batch_started",
            DownloadBatchStartedEvent {
                total_files: batch_count,
                total_bytes_hint,
                unknown_size_files,
                output_dir: base.to_string_lossy().to_string(),
            },
        );
        let _ = app.emit(
            "crawl_log",
            format!(
                "[ARIA] Batch mirror engaged: {} files | Circuits: {}",
                batch_count,
                connections.max(1)
            ),
        );

        let control = aria_downloader::activate_download_control()
            .ok_or_else(|| anyhow!("A download is already active."))?;
        let batch_result = aria_downloader::start_batch_download(
            app.clone(),
            batch_files,
            connections.max(1),
            force_tor,
            Some(base.to_string_lossy().to_string()),
            control,
        )
        .await;
        aria_downloader::clear_download_control();
        batch_result?;

        written_final = written_final.saturating_add(batch_count as u32);
    }

    Ok(written_final)
}

fn chrono_stub() -> String {
    // Simple timestamp without adding chrono dependency
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("epoch:{}", secs)
}

fn format_bytes(bytes: u64) -> String {
    if bytes >= 1_073_741_824 {
        format!("{:.2} GB", bytes as f64 / 1_073_741_824.0)
    } else if bytes >= 1_048_576 {
        format!("{:.2} MB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.2} KB", bytes as f64 / 1024.0)
    } else {
        format!("{} B", bytes)
    }
}

/// Cancel the active crawl
#[tauri::command]
async fn cancel_crawl(app: tauri::AppHandle) -> Result<String, String> {
    let state = app.state::<AppState>();
    let mut lock: tokio::sync::MutexGuard<'_, Option<Arc<frontier::CrawlerFrontier>>> =
        state.active_frontier.lock().await;
    let active_frontier = lock.take();
    drop(lock);

    // Force-stop any active single file download first.
    let _ = stop_active_download(app.clone());
    // Phase 134: Immediately release the download control lock so a restarted
    // download can acquire it. Without this, the previous start_crawl is still
    // unwinding and holds the ACTIVE_CONTROL mutex, causing the next
    // activate_download_control() to return None → "A download is already active"
    // → batch download skipped → files land flat without folder scaffolding.
    aria_downloader::clear_download_control();

    if let Some(frontier) = active_frontier {
        frontier.cancel();
        let snapshot = frontier.progress_snapshot();
        let progress = estimate_progress_percent(
            snapshot.visited.max(1),
            snapshot.processed,
            snapshot.queued,
            snapshot.active_workers,
        );
        let _ = app.emit(
            "crawl_status_update",
            crawl_status_snapshot(frontier.as_ref(), "cancelled", progress, None),
        );
    }

    // Hard cleanup for all Crawli-managed Tor daemons/circuits.
    // This guarantees no lingering proxy swarm survives cancel.
    tor::cleanup_stale_tor_daemons();

    app.emit(
        "crawl_log",
        "[System] ⚠ FORCE CANCEL executed: crawl workers, active downloads, and Tor daemons terminated.",
    )
    .unwrap_or_default();

    Ok("Force cancel completed. Use Sync Now / Start Queue to restart.".to_string())
}

#[tauri::command]
async fn export_json(output_path: String, app: tauri::AppHandle) -> Result<String, String> {
    let state = app.state::<AppState>();
    let entries = state.vfs.iter_entries().await.map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(&entries).map_err(|e| e.to_string())?;
    tokio::fs::write(&output_path, json.as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    Ok(format!(
        "Exported {} entries to {}",
        entries.len(),
        output_path
    ))
}

#[tauri::command]
async fn download_all(
    output_dir: String,
    connections: Option<usize>,
    target_url: Option<String>,
    app: tauri::AppHandle,
) -> Result<u32, String> {
    use tauri::Emitter;
    let _ = app.emit(
        "crawl_log",
        "[System] Querying Sled VFS for full mirroring operation...",
    );
    let state = app.state::<AppState>();
    let output_root = canonical_output_root(&output_dir)?;
    let circuits = connections.unwrap_or(120).max(1);

    if let Some(target_url) = target_url.filter(|value| !value.trim().is_empty()) {
        let target_paths =
            target_state::target_paths(&output_root, &target_url).map_err(|e| e.to_string())?;
        let mut authoritative_entries =
            target_state::load_entries_snapshot(&target_paths.best_snapshot_path)
                .map_err(|e| e.to_string())?;
        maybe_repin_qilin_entries_from_context(
            &mut authoritative_entries,
            Some(target_paths.target_dir.as_path()),
            &app,
        )
        .map_err(|e| e.to_string())?;
        if !authoritative_entries.is_empty() {
            let failure_records =
                target_state::load_failure_manifest(&target_paths.failure_manifest_path)
                    .map_err(|e| e.to_string())?;
            let resume_build = target_state::build_download_resume_plan(
                &target_paths.target_identity.target_key,
                &authoritative_entries,
                &failure_records,
                &output_root,
                &target_paths.failure_manifest_path,
            )
            .map_err(|e| e.to_string())?;
            target_state::save_resume_plan(
                &target_paths.latest_resume_plan_path,
                &resume_build.plan,
            )
            .map_err(|e| e.to_string())?;
            let _ = app.emit("download_resume_plan", resume_build.plan.clone());

            if resume_build.plan.all_items_skipped {
                let _ = app.emit(
                    "crawl_log",
                    format!(
                        "[SYSTEM] Download resume skipped for {}: all items already complete.",
                        target_paths.target_identity.target_key
                    ),
                );
                telemetry_bridge::publish_batch_progress(
                    &app,
                    telemetry_bridge::BridgeBatchProgress {
                        completed: resume_build.plan.skipped_exact_matches_count,
                        failed: 0,
                        total: resume_build.plan.skipped_exact_matches_count,
                        current_file: "All items skipped".to_string(),
                        speed_mbps: 0.0,
                        downloaded_bytes: 0,
                        active_circuits: Some(0),
                        bbr_bottleneck_mbps: None,
                        ekf_covariance: None,
                    },
                );
                return Ok(0);
            }

            return scaffold_download_from_entries_with_plan(
                &authoritative_entries,
                &resume_build.ordered_entries,
                &target_paths,
                &output_root,
                &app,
                circuits,
                url_targets_onion(&target_url),
            )
            .await
            .map_err(|e| e.to_string());
        }
    }

    let entries = state.vfs.iter_entries().await.map_err(|e| e.to_string())?;
    let force_tor = entries
        .iter()
        .any(|entry| url_targets_onion(&entry.raw_url));
    if entries.is_empty() {
        return Ok(0);
    }
    scaffold_download_from_vfs(&state.vfs, &output_root, &app, circuits, force_tor)
        .await
        .map_err(|e| e.to_string())
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct DownloadArgs {
    url: String,
    path: String,
    output_root: String,
    connections: usize,
    force_tor: bool,
}

#[derive(Clone, serde::Serialize)]
pub struct DownloadFailedEvent {
    url: String,
    path: String,
    error: String,
}

async fn run_single_download_blocking(
    app: tauri::AppHandle,
    args: DownloadArgs,
) -> Result<(), String> {
    let control = aria_downloader::activate_download_control()
        .ok_or_else(|| "A download is already active.".to_string())?;

    let DownloadArgs {
        url,
        path,
        output_root,
        connections,
        force_tor,
    } = args;
    let output_root = canonical_output_root(&output_root)?;
    let safe_target = path_utils::resolve_download_target_within_root(&output_root, &path)
        .map_err(|e| format!("Unsafe target path rejected: {e}"))?;
    let safe_target_str = safe_target.to_string_lossy().to_string();
    let safe_target_display = path_utils::display_path(&safe_target);

    app.emit("log", format!("Initiating extraction for: {url}"))
        .ok();
    app.emit(
        "log",
        format!(
            "[PATH] Output root: {}",
            path_utils::display_path(&output_root)
        ),
    )
    .ok();
    let support_resolution = ensure_support_artifact_dir_for_output_root(&output_root)
        .map_err(|e| format!("Failed to create support directory: {e}"))?;
    let support_dir = support_resolution.path;
    if let Some(note) = support_resolution.note.as_deref() {
        app.emit("log", format!("[PATH] {note}")).ok();
    }
    app.emit(
        "log",
        format!(
            "[PATH] Support artifact dir: {}",
            path_utils::display_path(&support_dir)
        ),
    )
    .ok();
    if support_resolution.used_fallback {
        app.emit(
            "log",
            "[PATH] Preferred sibling support root was unavailable; using in-output hidden support directory.".to_string(),
        )
        .ok();
    }
    app.emit(
        "log",
        format!("[PATH] Resolved target path: {}", safe_target_display),
    )
    .ok();

    let jwt_cache: Arc<tokio::sync::RwLock<std::collections::HashMap<String, String>>> =
        Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()));
    let dummy_entry = aria_downloader::BatchFileEntry {
        url,
        path: safe_target_str,
        size_hint: None,
        jwt_exp: None,
        alternate_urls: Vec::new(),
    };
    let result = aria_downloader::start_download(
        app,
        dummy_entry,
        connections,
        force_tor,
        Some(output_root.to_string_lossy().to_string()),
        control,
        jwt_cache,
    )
    .await;

    aria_downloader::clear_download_control();
    result.map_err(|e| e.to_string())
}

#[tauri::command]
async fn initiate_download(app: tauri::AppHandle, args: DownloadArgs) -> Result<(), String> {
    let app_clone = app.clone();
    let fail_url = args.url.clone();
    let fail_path = args.path.clone();

    tokio::spawn(async move {
        if let Err(err) = run_single_download_blocking(app_clone.clone(), args).await {
            let message = err.to_string();
            let _ = app_clone.emit("log", format!("[ERROR] {message}"));
            let _ = app_clone.emit(
                "download_failed",
                DownloadFailedEvent {
                    url: fail_url,
                    path: fail_path,
                    error: message,
                },
            );
        }
    });

    Ok(())
}

#[tauri::command]
fn pause_active_download(app: tauri::AppHandle) -> Result<bool, String> {
    let paused = aria_downloader::request_pause();
    if paused {
        let _ = app.emit(
            "log",
            "[*] Pause requested for active download.".to_string(),
        );
    }
    Ok(paused)
}

#[tauri::command]
fn stop_active_download(app: tauri::AppHandle) -> Result<bool, String> {
    let stopped = aria_downloader::request_stop();
    if stopped {
        let _ = app.emit("log", "[*] Stop requested for active download.".to_string());
    }
    Ok(stopped)
}

pub(crate) async fn ensure_crawl_swarm(
    app: &tauri::AppHandle,
) -> Result<Arc<tokio::sync::Mutex<tor::TorProcessGuard>>, String> {
    let state = app.state::<AppState>();
    if let Some(existing) = state.crawl_swarm_guard.lock().await.clone() {
        return Ok(existing);
    }

    // Phase 117: Hardcoded to 8 — MultiClientPool caps at 8 TorClients
    let (guard, _) = tor::bootstrap_tor_cluster_for_traffic(
        app.clone(),
        8,
        0,
        tor::SwarmTrafficClass::OnionService,
    )
    .await
    .map_err(|e| format!("Failed to bootstrap Tor swarm: {e}"))?;
    let guard_arc = Arc::new(tokio::sync::Mutex::new(guard));
    *state.crawl_swarm_guard.lock().await = Some(guard_arc.clone());
    Ok(guard_arc)
}

async fn perform_pre_resolve_onion(url: &str, app: &tauri::AppHandle) -> Result<(), String> {
    if !url_targets_onion(url) {
        return Ok(());
    }

    let guard_arc = ensure_crawl_swarm(app).await?;
    let guard = guard_arc.lock().await;
    let shared_client = guard
        .get_arti_clients()
        .first()
        .cloned()
        .ok_or_else(|| "Tor swarm did not expose an arti client.".to_string())?;
    drop(guard);

    let tor_client = shared_client.read().unwrap().clone();
    let token = ::arti_client::IsolationToken::new();
    let arti = arti_client::ArtiClient::new((*tor_client).clone(), Some(token));
    tokio::time::timeout(std::time::Duration::from_secs(10), arti.head(url).send())
        .await
        .map_err(|_| format!("Pre-resolve timed out for {url}"))?
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
async fn pre_resolve_onion(url: String, app: tauri::AppHandle) -> Result<(), String> {
    if !url_targets_onion(&url) {
        return Ok(());
    }
    let app_handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let _ = perform_pre_resolve_onion(&url, &app_handle).await;
    });
    Ok(())
}

#[tauri::command]
async fn get_subtree_heatmap(target_key: String) -> Result<serde_json::Value, String> {
    let target_dir = crate::canonical_output_root("")
        .unwrap_or_default()
        .join("targets")
        .join(&target_key);
    let heatmap_path = target_dir.join("qilin_bad_subtrees.json");

    if heatmap_path.exists() {
        if let Ok(content) = tokio::fs::read_to_string(&heatmap_path).await {
            if let Ok(json) = serde_json::from_str(&content) {
                return Ok(json);
            }
        }
    }
    Ok(serde_json::json!({}))
}

#[tauri::command]
async fn open_folder_os(path: String) -> Result<(), String> {
    open::that(&path).map_err(|e| e.to_string())
}

#[tauri::command]
fn set_telemetry_enabled(enabled: bool) {
    crate::binary_telemetry::TELEMETRY_ENABLED.store(enabled, std::sync::atomic::Ordering::Relaxed);
}

/// Phase 52: Returns the detected input mode for the frontend auto-detect badge.
#[tauri::command]
fn detect_input_mode(input: String) -> String {
    torrent_handler::detect_input_mode(&input).to_string()
}

/// Phase 67H: Returns the detected system profile for GUI auto-selection
#[tauri::command]
fn get_system_profile() -> serde_json::Value {
    let profile = crate::resource_governor::recommended_concurrency_preset();
    serde_json::json!({
        "preset": profile.preset,
        "circuits": profile.circuits,
        "workers": profile.workers,
        "cpuCores": profile.cpu_cores,
        "totalRamGb": profile.total_ram_gb,
        "availableRamGb": profile.available_ram_gb,
        "storageClass": profile.storage_class,
        "os": profile.os
    })
}

fn install_runtime_prereqs() {
    // Phase 62: Install missing global CryptoProvider for rustls >= 0.23
    // Without this, ArtiClient::new panics silently on the tokio async thread,
    // dropping the Tauri IPC resolver and creating an infinite hang in the frontend proxy.
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        rustls::crypto::CryptoProvider::install_default(rustls::crypto::ring::default_provider())
            .expect(
                "Failed to universally install ring CryptoProvider. Backend cannot run safely.",
            );
    }

    // Flush any zombie Tor daemons from previous sessions on startup
    tor::cleanup_stale_tor_daemons();
}

pub(crate) fn tauri_context() -> tauri::Context<tauri::Wry> {
    tauri::generate_context!()
}

fn run_gui() {
    tauri::Builder::default()
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::default())
        .setup(|app| {
            let state = app.state::<AppState>();
            runtime_metrics::spawn_metrics_emitter(app.handle().clone(), state.telemetry.clone());
            telemetry_bridge::spawn_bridge_emitter(
                app.handle().clone(),
                state.telemetry_bridge.clone(),
            );
            if native_webview_smoke_enabled() {
                let _ = app.emit(
                    "log",
                    "[NATIVE_SMOKE] startup bootstrap bypass enabled".to_string(),
                );
                return Ok(());
            }
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let state = handle.state::<AppState>();
                if let Ok((guard, _)) = tor::bootstrap_tor_cluster_for_traffic(
                    handle.clone(),
                    4,
                    0,
                    tor::SwarmTrafficClass::OnionService,
                )
                .await
                {
                    *state.crawl_swarm_guard.lock().await =
                        Some(Arc::new(tokio::sync::Mutex::new(guard)));
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            start_crawl,
            download_files,
            download_all,
            cancel_crawl,
            export_json,
            initiate_download,
            pause_active_download,
            stop_active_download,
            get_vfs_children,
            get_adapter_support_catalog,
            ingest_vfs_entries,
            pre_resolve_onion,
            get_subtree_heatmap,
            open_folder_os,
            set_telemetry_enabled,
            get_native_smoke_config,
            report_native_smoke_result,
            crate::binary_telemetry::drain_telemetry_ring,
            detect_input_mode,
            get_system_profile,
            crate::network_disk::fetch_network_disk_block_cmd,
            crate::network_disk::fetch_network_disk_extents_cmd,
            // Phase 53: Azure + Intranet enterprise commands (feature-gated)
            #[cfg(feature = "azure")]
            azure_connectivity::configure_azure_storage,
            #[cfg(feature = "azure")]
            azure_connectivity::test_azure_connection,
            #[cfg(feature = "azure")]
            azure_connectivity::enable_azure_storage,
            #[cfg(feature = "azure")]
            azure_connectivity::disable_azure_storage,
            #[cfg(feature = "azure")]
            azure_connectivity::toggle_intranet_server,
            #[cfg(feature = "azure")]
            azure_connectivity::get_azure_status
        ])
        .on_window_event(|_window, event| {
            if let tauri::WindowEvent::Destroyed = event {
                // Kill any remaining Tor processes on window close
                tor::cleanup_stale_tor_daemons();
            }
        })
        .run(tauri_context())
        .expect("error while running tauri application");
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    install_runtime_prereqs();
    if let Some(exit_code) = cli::try_run_from_env() {
        std::process::exit(exit_code);
    }
    run_gui();
}

pub fn run_cli() {
    install_runtime_prereqs();
    std::process::exit(cli::run_cli_from_env());
}

#[cfg(test)]
mod tests {
    use super::{
        balanced_qilin_repin_host_cap, build_qilin_alternate_urls,
        preferred_qilin_download_host_for_subtree, preferred_qilin_host_for_subtree,
        rewrite_qilin_seed_host, should_repin_qilin_download_to_preferred_host,
        split_qilin_download_seed_and_relative_path, subtree_key_for_qilin_download,
        support_artifact_dir_for_output_root, support_key_for_path, url_targets_onion,
        PersistedQilinSubtreeRouteEntry,
    };
    use std::collections::HashMap;

    #[test]
    fn split_qilin_download_seed_preserves_relative_tree() {
        let url = "http://hosta.onion/6749d00a-277a-4575-a398-2120f37889d6/kent/Business%20Related/1%20and%201%20internet/file.pdf";
        let (seed, relative) =
            split_qilin_download_seed_and_relative_path(url).expect("qilin url should parse");
        assert_eq!(
            seed,
            "http://hosta.onion/6749d00a-277a-4575-a398-2120f37889d6/"
        );
        assert_eq!(
            relative,
            "kent/Business%20Related/1%20and%201%20internet/file.pdf"
        );
        assert_eq!(
            subtree_key_for_qilin_download(url).as_deref(),
            Some("kent/Business%20Related/1%20and%201%20internet")
        );
    }

    #[test]
    fn rewrite_qilin_seed_host_keeps_storage_uuid_and_suffix() {
        let url =
            "http://hosta.onion/6749d00a-277a-4575-a398-2120f37889d6/kent/Bankrupcy/report.pdf";
        let remapped =
            rewrite_qilin_seed_host(url, "hostb.onion").expect("host rewrite should succeed");
        assert_eq!(
            remapped,
            "http://hostb.onion/6749d00a-277a-4575-a398-2120f37889d6/kent/Bankrupcy/report.pdf"
        );
    }

    #[test]
    fn url_targets_onion_checks_host_not_path() {
        assert!(url_targets_onion("http://example.onion/files/"));
        assert!(url_targets_onion("example.onion/files/"));
        assert!(!url_targets_onion(
            "https://cdn.breachforums.as/pay_or_leak/shouldve_paid_the_ransom_pathstone.com_shinyhunters.7z"
        ));
        assert!(!url_targets_onion(
            "https://example.com/path/onion/report.txt"
        ));
    }

    #[test]
    fn preferred_qilin_host_walks_up_to_parent_subtree() {
        let mut routes = HashMap::new();
        routes.insert(
            "kent/Business%20Related".to_string(),
            "winner.onion".to_string(),
        );
        let preferred = preferred_qilin_host_for_subtree(
            "kent/Business%20Related/1%20and%201%20internet",
            &routes,
            Some("fallback.onion"),
        );
        assert_eq!(preferred.as_deref(), Some("winner.onion"));

        let fallback =
            preferred_qilin_host_for_subtree("kent/Unknown", &routes, Some("fallback.onion"));
        assert_eq!(fallback.as_deref(), Some("fallback.onion"));
    }

    #[test]
    fn download_host_bias_uses_subtree_preference_before_winner_fallback() {
        let mut routes = HashMap::new();
        routes.insert(
            "kent/Business%20Related".to_string(),
            PersistedQilinSubtreeRouteEntry {
                subtree_key: "kent/Business%20Related".to_string(),
                preferred_host: "stale.onion".to_string(),
                success_count: 2,
                last_success_epoch: 100,
            },
        );

        let preferred = preferred_qilin_download_host_for_subtree(
            "kent/Business%20Related/1%20and%201%20internet",
            &routes,
            Some("winner.onion"),
        );

        assert_eq!(preferred.as_deref(), Some("stale.onion"));
    }

    fn strong_subtree_route_is_required_before_download_repin() {
        let weak_route = PersistedQilinSubtreeRouteEntry {
            subtree_key: "kent/Business%20Related".to_string(),
            preferred_host: "winner.onion".to_string(),
            success_count: 1,
            last_success_epoch: 100,
        };
        assert!(!should_repin_qilin_download_to_preferred_host(
            &weak_route,
            Some("winner.onion"),
            260,
        ));

        let strong_route = PersistedQilinSubtreeRouteEntry {
            success_count: 4,
            last_success_epoch: 250,
            ..weak_route
        };
        assert!(should_repin_qilin_download_to_preferred_host(
            &strong_route,
            Some("winner.onion"),
            260,
        ));
    }

    #[test]
    fn balanced_qilin_repin_cap_preserves_host_diversity() {
        assert_eq!(balanced_qilin_repin_host_cap(1, 1), 1);
        assert!(balanced_qilin_repin_host_cap(2_394, 3) < 1_100);
    }

    #[test]
    fn support_artifact_dir_stays_inside_selected_output_root() {
        let output_root = std::path::Path::new("/tmp/onionforge/output");
        let support_dir = support_artifact_dir_for_output_root(output_root);
        assert!(support_dir.starts_with(output_root.join(".onionforge_support")));
    }

    #[test]
    fn support_artifact_dir_ignores_blocked_sibling_anchor_and_uses_output_root() {
        let unique = format!(
            "onionforge-support-fallback-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let base = std::env::temp_dir().join(unique);
        let output_root = base.join("output");
        std::fs::create_dir_all(&output_root).unwrap();

        let blocked_anchor = base.join(".onionforge_support");
        std::fs::write(&blocked_anchor, b"blocked").unwrap();

        let resolution = super::ensure_support_artifact_dir_for_output_root(&output_root).unwrap();
        assert!(!resolution.used_fallback);
        assert!(resolution
            .path
            .starts_with(output_root.join(".onionforge_support")));
        assert!(resolution.path.is_dir());
        assert!(resolution.fallback_reason.is_none());
        assert!(resolution.note.is_none());

        let _ = std::fs::remove_file(&blocked_anchor);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn support_key_for_windows_device_path_removes_reserved_chars() {
        let key = support_key_for_path(r"\\?\X:\Exports\Case 1");
        assert!(key.starts_with("X_Exports_Case_1_"));
        assert!(!key.contains('?'));
        assert!(!key.contains(':'));
        assert!(!key.contains('\\'));
        assert!(!key.contains('/'));
        assert!(!key.contains('*'));
    }

    #[test]
    fn qilin_alternate_urls_skip_current_host_and_preserve_path() {
        let url =
            "http://hosta.onion/6749d00a-277a-4575-a398-2120f37889d6/kent/Bankrupcy/report.pdf";
        let alternates = build_qilin_alternate_urls(
            url,
            &[
                "hosta.onion".to_string(),
                "hostb.onion".to_string(),
                "hostc.onion".to_string(),
            ],
            "kent/Bankrupcy/report.pdf",
        );
        assert_eq!(alternates.len(), 2);
        assert!(alternates[0].contains("hostb.onion") || alternates[0].contains("hostc.onion"));
        assert_ne!(alternates[0], alternates[1]);
        assert!(alternates
            .iter()
            .all(|url| url.contains("/kent/Bankrupcy/report.pdf")));
    }
}
pub mod arti_client;
pub mod arti_connector;
pub mod seed_manager;
mod sp_test;
