pub mod adapters;
pub mod aria_downloader;
#[cfg(feature = "azure")]
pub mod azure_connectivity;
pub mod bbr;
pub mod bft_quorum;
pub mod binary_telemetry;
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
pub mod path_utils;
pub mod resource_governor;
pub mod runtime_metrics;
pub mod scorer;
pub mod speculative_prefetch;
pub mod subtree_heatmap;
pub mod target_state;
pub mod telemetry_bridge;
pub mod tor;
pub mod tor_native;
pub mod tor_runtime; // Phase 45: Parallel chunk downloading
pub mod torrent_handler; // Phase 52: BitTorrent .torrent + magnet support // Phase 53: Optional Azure + Intranet enterprise

use std::sync::Arc;
use tauri::Emitter;
use tauri::Manager;
use tokio::sync::Mutex;

/// Global shared frontier for cancellation support
pub struct AppState {
    active_frontier: Mutex<Option<Arc<frontier::CrawlerFrontier>>>,
    pub(crate) current_target_dir: Mutex<Option<std::path::PathBuf>>,
    pub(crate) current_target_key: Mutex<Option<String>>,
    pub vfs: db::SledVfs,
    pub telemetry: runtime_metrics::RuntimeTelemetry,
    pub telemetry_bridge: telemetry_bridge::TelemetryBridge,
    pub swarm_guard: tokio::sync::Mutex<Option<Arc<tokio::sync::Mutex<tor::TorProcessGuard>>>>,
    /// Phase 53: Azure connectivity state (only compiled with `--features azure`)
    #[cfg(feature = "azure")]
    pub azure: tokio::sync::Mutex<azure_connectivity::AzureConnectivityState>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            active_frontier: Mutex::new(None),
            current_target_dir: Mutex::new(None),
            current_target_key: Mutex::new(None),
            vfs: db::SledVfs::default(),
            telemetry: runtime_metrics::RuntimeTelemetry::default(),
            telemetry_bridge: telemetry_bridge::TelemetryBridge::default(),
            swarm_guard: tokio::sync::Mutex::new(None),
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
    let visited = frontier.visited_count();
    let processed = frontier.processed_count();
    let queued = visited.saturating_sub(processed);

    CrawlStatusUpdate {
        phase: phase.to_string(),
        progress_percent: progress_percent.clamp(0.0, 100.0),
        visited_nodes: visited,
        processed_nodes: processed,
        queued_nodes: queued,
        active_workers: frontier.active_workers(),
        worker_target: frontier.worker_target(),
        eta_seconds,
        estimation: "adaptive-frontier".to_string(),
        delta_new_files: frontier
            .delta_new_files
            .load(std::sync::atomic::Ordering::Relaxed),
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

            let visited = frontier.visited_count();
            let processed = frontier.processed_count();
            let queued = visited.saturating_sub(processed);
            let active_workers = frontier.active_workers();

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

    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    let hash = hasher.finish();

    let mut base = path
        .trim_start_matches('/')
        .replace(['/', '\\'], "_")
        .replace(':', "_");
    if base.is_empty() {
        base = "root".to_string();
    }
    if base.len() > 96 {
        base.truncate(96);
    }

    format!("{base}_{hash:016x}")
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

async fn execute_crawl_attempt(
    url: &str,
    options: &frontier::CrawlOptions,
    output_root: &std::path::Path,
    target_paths: &target_state::TargetPaths,
    app: &tauri::AppHandle,
    vfs: &db::SledVfs,
    ledger: std::sync::Arc<crate::target_state::TargetLedger>,
) -> Result<CrawlAttemptResult, String> {
    let is_onion = url.contains(".onion");
    let support_dir = output_root.join("temp_onionforge_forger");
    tokio::fs::create_dir_all(&support_dir)
        .await
        .map_err(|e| format!("Failed to create support directory: {e}"))?;

    let mut swarm_guard = None;
    let mut arti_clients = Vec::new();

    if is_onion {
        let mut pre_warmed = false;
        if let Some(guard_arc) = app.state::<AppState>().swarm_guard.lock().await.as_ref() {
            let guard = guard_arc.lock().await;
            arti_clients = guard.get_arti_clients();
            if !arti_clients.is_empty() {
                swarm_guard = Some(guard_arc.clone());
                pre_warmed = true;
                println!(
                    "[Crawli Bootstrap] Using pre-warmed Phantom Swarm ({} clients)",
                    arti_clients.len()
                );
            }
        }

        if !pre_warmed {
            tor::cleanup_stale_tor_daemons();
            app.emit(
                "crawl_log",
                format!("[System] Bootstrapping Target: {}", url),
            )
            .unwrap_or_default();
            println!("[Crawli Bootstrap] starting tor bootstrap for {}", url);

            let target_daemons = options
                .daemons
                .unwrap_or(if cfg!(target_os = "windows") { 8 } else { 12 })
                .max(1);
            match tor::bootstrap_tor_cluster(app.clone(), target_daemons).await {
                Ok((guard, ports)) => {
                    println!(
                        "[Crawli Bootstrap] tor bootstrap complete: runtime={} ports={:?}",
                        guard.runtime_label(),
                        ports
                    );
                    arti_clients = guard.get_arti_clients();
                    swarm_guard = Some(std::sync::Arc::new(tokio::sync::Mutex::new(guard)));
                }
                Err(e) => return Err(format!("Failed to start Tor Swarm: {}", e)),
            }
        }
    }

    let daemon_count = if is_onion {
        arti_clients.len().max(1)
    } else {
        options
            .daemons
            .unwrap_or(if cfg!(target_os = "windows") { 8 } else { 12 })
            .max(1)
    };

    let mut frontier = frontier::CrawlerFrontier::new(
        Some(app.clone()),
        url.to_string(),
        daemon_count,
        is_onion,
        Vec::new(), // explicit ports removed, handled internally
        arti_clients,
        options.clone(),
        Some(target_paths.clone()),
    );
    println!(
        "[Crawli Bootstrap] frontier initialized: clients={} daemons={} onion={}",
        frontier.http_clients.len(),
        frontier.num_daemons,
        frontier.is_onion
    );
    frontier.swarm_guard = swarm_guard;

    let vfs_path = support_dir.join(".crawli_vtdb");
    let vfs_path_str = vfs_path.to_string_lossy().to_string();
    let _ = vfs.initialize(&vfs_path_str).await;
    let _ = vfs.clear().await;
    println!("[Crawli Bootstrap] vfs initialized at {}", vfs_path_str);

    println!("[Crawli Fingerprint] requesting initial URL: {}", url);
    app.emit(
        "crawl_log",
        "[System] Generating Site Fingerprint...".to_string(),
    )
    .unwrap_or_default();

    let fingerprint_attempts = if is_onion { 4 } else { 2 };
    let mut fingerprint_attempt = 1usize;
    let resp = loop {
        let attempt = fingerprint_attempt;
        let (cid, client) = frontier.get_client();
        match client.get(url).send().await {
            Ok(r) => {
                println!(
                    "[Crawli Fingerprint] initial URL responded: status={} final={}",
                    r.status(),
                    r.url()
                );
                break r;
            }
            Err(e) => {
                let error_text = e.to_string();
                println!(
                    "[Crawli Fingerprint] initial URL request failed on attempt {} via cid {}: {}",
                    attempt, cid, error_text
                );

                if attempt >= fingerprint_attempts {
                    return Err(format!(
                        "OFFLINE_SYNC_ERROR: The site might be down. Please manually check it to verify if it is actually functional and active. ({})",
                        error_text
                    ));
                }

                let _ = app.emit(
                    "crawl_log",
                    format!(
                        "[System] Initial fingerprint connect failed on attempt {}. Rotating client slot and retrying...",
                        attempt
                    ),
                );
                if is_onion {
                    frontier.trigger_circuit_isolation(cid).await;
                }
                tokio::time::sleep(std::time::Duration::from_millis((attempt as u64) * 750)).await;
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
            || url.ends_with(".rar"));

    let body = if is_binary {
        "[BINARY_OR_ARCHIVE_DATA]".to_string()
    } else {
        resp.text()
            .await
            .unwrap_or_else(|_| "[DECODE_ERROR]".to_string())
    };

    let fingerprint = adapters::SiteFingerprint {
        url: url.to_string(),
        status,
        headers,
        body,
    };

    if let Some(name) = looks_like_direct_artifact(url, is_binary) {
        let registry = adapters::AdapterRegistry::new().with_explorer_context(ledger.clone());
        if let Some(adapter) = registry.determine_adapter(&fingerprint).await {
            app.emit(
                "crawl_log",
                format!("[Adapter] Match found: {}", adapter.name()),
            )
            .unwrap_or_default();
        } else {
            app.emit(
                "crawl_log",
                "[Adapter] Match found: Direct Artifact (No specialized adapter match)".to_string(),
            )
            .unwrap_or_default();
        }

        let size = fingerprint
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

    let registry = adapters::AdapterRegistry::new().with_explorer_context(ledger.clone());
    let Some(adapter) = registry.determine_adapter(&fingerprint).await else {
        return Err("No known adapter matched this sites architecture.".to_string());
    };

    app.emit(
        "crawl_log",
        format!("[Adapter] Match found: {}", adapter.name()),
    )
    .unwrap_or_default();

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
        let visited = arc_frontier.visited_count();
        let processed = arc_frontier.processed_count();
        let queued = visited.saturating_sub(processed);
        let progress = estimate_progress_percent(
            visited.max(1),
            processed,
            queued,
            arc_frontier.active_workers(),
        );
        crawl_status_snapshot(arc_frontier.as_ref(), "cancelled", progress, None)
    } else if crawl_result.is_ok() {
        crawl_status_snapshot(arc_frontier.as_ref(), "complete", 100.0, Some(0))
    } else {
        let visited = arc_frontier.visited_count();
        let processed = arc_frontier.processed_count();
        let queued = visited.saturating_sub(processed);
        let progress = estimate_progress_percent(
            visited.max(1),
            processed,
            queued,
            arc_frontier.active_workers(),
        );
        crawl_status_snapshot(arc_frontier.as_ref(), "error", progress, None)
    };
    telemetry_bridge::publish_crawl_status(&app, final_payload);

    match crawl_result {
        Ok(files) => {
            let state = app.state::<AppState>();
            *state.current_target_dir.lock().await = None;
            *state.current_target_key.lock().await = None;
            if !files.is_empty() {
                vfs.insert_entries(&files)
                    .await
                    .map_err(|e| e.to_string())?;
            }
            let mut summary = vfs.summarize_entries().await.map_err(|e| e.to_string())?;
            if summary.discovered_count == 0 && !files.is_empty() {
                summary = summarize_entry_slice(&files);
            }
            Ok(CrawlAttemptResult {
                summary,
                was_cancelled,
            })
        }
        Err(e) => {
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
    use anyhow::anyhow;
    use aria_downloader::BatchFileEntry;

    let base = output_root.to_path_buf();
    tokio::fs::create_dir_all(&base).await?;
    let support_dir = base.join("temp_onionforge_forger");
    tokio::fs::create_dir_all(&support_dir).await?;

    let mut written_final: u32 = 0;
    let mut batch_files: Vec<BatchFileEntry> = Vec::new();
    let mut manifest = String::new();
    let mut total_entries = 0usize;
    let mut total_bytes_hint = 0u64;
    let mut unknown_size_files = 0usize;

    manifest.push_str("# OnionForge Download Manifest\n");
    manifest.push_str(&format!("# Generated: {}\n", chrono_stub()));

    vfs.with_entry_batches(512, |entries| {
        for entry in entries {
            total_entries = total_entries.saturating_add(1);
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
                    std::fs::create_dir_all(&full_path)?;
                    let gitkeep = full_path.join(".gitkeep");
                    if !gitkeep.exists() {
                        let _ = std::fs::write(&gitkeep, b"");
                    }
                    written_final = written_final.saturating_add(1);
                }
                adapters::EntryType::File => {
                    if let Some(parent) = full_path.parent() {
                        std::fs::create_dir_all(parent)?;
                    }

                    let safe_target = full_path.to_string_lossy().to_string();
                    let url_lower = entry.raw_url.to_lowercase();
                    if url_lower.starts_with("http://") || url_lower.starts_with("https://") {
                        batch_files.push(BatchFileEntry {
                            url: entry.raw_url.clone(),
                            path: safe_target,
                            size_hint: entry.size_bytes,
                            jwt_exp: entry.jwt_exp,
                        });
                        total_bytes_hint =
                            total_bytes_hint.saturating_add(entry.size_bytes.unwrap_or(0));
                        if entry.size_bytes.unwrap_or(0) == 0 {
                            unknown_size_files = unknown_size_files.saturating_add(1);
                        }
                    } else {
                        if !full_path.exists() {
                            let _ = std::fs::write(&full_path, b"");
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

                    let meta_path = support_dir.join(format!(
                        "{}.onionforge.meta",
                        support_key_for_path(&entry.path)
                    ));
                    let size_str = entry
                        .size_bytes
                        .map(|s: u64| s.to_string())
                        .unwrap_or_else(|| "0".to_string());
                    let meta_content = format!(
                        "url={}\nsize={}\ntype=file\noriginal_path={}\n",
                        entry.raw_url, size_str, entry.path
                    );
                    let _ = std::fs::write(&meta_path, meta_content.as_bytes());
                }
            }

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
        Ok(())
    })
    .await?;

    manifest.insert_str(
        manifest
            .find('\n')
            .map(|idx| idx + 1)
            .unwrap_or(manifest.len()),
        &format!("# Total Entries: {}\n\n", total_entries),
    );

    if !batch_files.is_empty() {
        let batch_count = batch_files.len();
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

    let manifest_path = support_dir.join("_onionforge_manifest.txt");
    tokio::fs::write(&manifest_path, manifest.as_bytes()).await?;

    Ok(written_final)
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
    let support_dir = base.join("temp_onionforge_forger");
    tokio::fs::create_dir_all(&support_dir).await?;

    let mut written_final: u32 = 0;
    let mut batch_lookup: BTreeMap<String, BatchFileEntry> = BTreeMap::new();
    let mut manifest = String::new();
    let mut total_bytes_hint = 0u64;
    let mut unknown_size_files = 0usize;

    manifest.push_str("# OnionForge Download Manifest\n");
    manifest.push_str(&format!("# Generated: {}\n", chrono_stub()));
    manifest.push_str(&format!("# Total Entries: {}\n\n", entries.len()));

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
                        BatchFileEntry {
                            url: entry.raw_url.clone(),
                            path: safe_target,
                            size_hint: entry.size_bytes,
                            jwt_exp: entry.jwt_exp,
                        },
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

                let meta_path = support_dir.join(format!(
                    "{}.onionforge.meta",
                    support_key_for_path(&entry.path)
                ));
                let size_str = entry
                    .size_bytes
                    .map(|s: u64| s.to_string())
                    .unwrap_or_else(|| "0".to_string());
                let meta_content = format!(
                    "url={}\nsize={}\ntype=file\noriginal_path={}\n",
                    entry.raw_url, size_str, entry.path
                );
                let _ = tokio::fs::write(&meta_path, meta_content.as_bytes()).await;
            }
        }

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

    let batch_files: Vec<BatchFileEntry> = ordered_entries
        .iter()
        .filter_map(|entry| batch_lookup.remove(&entry.path))
        .collect();

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

    let manifest_path = support_dir.join("_onionforge_manifest.txt");
    tokio::fs::write(&manifest_path, manifest.as_bytes()).await?;

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
    options: frontier::CrawlOptions,
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
            let is_onion = url.contains(".onion");
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
    let force_tor = entries.iter().any(|entry| entry.raw_url.contains(".onion"));
    let circuits = connections.unwrap_or(120).max(1);
    scaffold_download(&entries, &output_root, &app, circuits, force_tor)
        .await
        .map_err(|e| e.to_string())
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
    let support_dir = base.join("temp_onionforge_forger");
    tokio::fs::create_dir_all(&support_dir).await?;

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
                    batch_files.push(BatchFileEntry {
                        url: entry.raw_url.clone(),
                        path: safe_target,
                        size_hint: entry.size_bytes,
                        jwt_exp: entry.jwt_exp,
                    });
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

                let meta_path = support_dir.join(format!(
                    "{}.onionforge.meta",
                    support_key_for_path(&entry.path)
                ));
                let size_str = entry
                    .size_bytes
                    .map(|s: u64| s.to_string())
                    .unwrap_or_else(|| "0".to_string());
                let meta_content = format!(
                    "url={}\nsize={}\ntype=file\noriginal_path={}\n",
                    entry.raw_url, size_str, entry.path
                );
                let _ = tokio::fs::write(&meta_path, meta_content.as_bytes()).await;
            }
        }
    }

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

    // Write a manifest index at the root
    let manifest_path = support_dir.join("_onionforge_manifest.txt");
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
    tokio::fs::write(&manifest_path, manifest.as_bytes()).await?;

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

    if let Some(frontier) = active_frontier {
        frontier.cancel();
        let visited = frontier.visited_count();
        let processed = frontier.processed_count();
        let queued = visited.saturating_sub(processed);
        let progress =
            estimate_progress_percent(visited.max(1), processed, queued, frontier.active_workers());
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
        let authoritative_entries =
            target_state::load_entries_snapshot(&target_paths.best_snapshot_path)
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
                target_url.contains(".onion"),
            )
            .await
            .map_err(|e| e.to_string());
        }
    }

    let entries = state.vfs.iter_entries().await.map_err(|e| e.to_string())?;
    let force_tor = entries.iter().any(|entry| entry.raw_url.contains(".onion"));
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

#[tauri::command]
async fn initiate_download(app: tauri::AppHandle, args: DownloadArgs) -> Result<(), String> {
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

    app.emit("log", format!("Initiating extraction for: {url}"))
        .ok();
    app.emit(
        "log",
        format!("[PATH] Output root: {}", output_root.display()),
    )
    .ok();
    let support_dir = output_root.join("temp_onionforge_forger");
    app.emit(
        "log",
        format!("[PATH] Support artifact dir: {}", support_dir.display()),
    )
    .ok();
    app.emit(
        "log",
        format!("[PATH] Resolved target path: {}", safe_target_str),
    )
    .ok();

    let app_clone = app.clone();
    let fail_url = url.clone();
    let fail_path = safe_target_str.clone();

    tokio::spawn(async move {
        let jwt_cache: Arc<tokio::sync::RwLock<std::collections::HashMap<String, String>>> =
            Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()));
        let dummy_entry = aria_downloader::BatchFileEntry {
            url,
            path: safe_target_str,
            size_hint: None,
            jwt_exp: None,
        };
        let result = aria_downloader::start_download(
            app_clone.clone(),
            dummy_entry,
            connections,
            force_tor,
            Some(output_root.to_string_lossy().to_string()),
            control,
            jwt_cache,
        )
        .await;

        aria_downloader::clear_download_control();

        if let Err(err) = result {
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

#[tauri::command]
async fn pre_resolve_onion(url: String, app: tauri::AppHandle) -> Result<(), String> {
    if !url.contains(".onion") {
        return Ok(());
    }
    let state = app.state::<AppState>();
    if let Some(guard_arc) = state.swarm_guard.lock().await.clone() {
        tauri::async_runtime::spawn(async move {
            let guard = guard_arc.lock().await;
            if let Some(client) = guard.get_arti_clients().first() {
                let tor_client = if tokio::runtime::Handle::try_current().is_ok() {
                    tokio::task::block_in_place(|| client.blocking_read().clone())
                } else {
                    client.blocking_read().clone()
                };
                let token = ::arti_client::IsolationToken::new();
                let arti = arti_client::ArtiClient::new((*tor_client).clone(), Some(token));
                let _ = tokio::time::timeout(
                    std::time::Duration::from_secs(10),
                    arti.head(&url).send(),
                )
                .await;
            }
        });
    }
    Ok(())
}

#[tauri::command]
async fn get_subtree_heatmap(target_key: String) -> Result<serde_json::Value, String> {
    let target_dir = crate::canonical_output_root("")
        .unwrap_or_default()
        .join("targets")
        .join(&target_key);
    let heatmap_path = target_dir
        .join("temp_onionforge_forger")
        .join("subtree_heatmap.json");

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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Flush any zombie Tor daemons from previous sessions on startup
    tor::cleanup_stale_tor_daemons();

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
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let state = handle.state::<AppState>();
                if let Ok((guard, _)) = tor::bootstrap_tor_cluster(handle.clone(), 4).await {
                    *state.swarm_guard.lock().await =
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
            detect_input_mode,
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
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
pub mod arti_client;
pub mod arti_connector;
