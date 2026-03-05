pub mod adapters;
pub mod aria_downloader;
pub mod bbr;
pub mod bft_quorum;
pub mod db;
pub mod ghost_browser;
pub mod frontier;
pub mod io_vanguard;
pub mod kalman;
pub mod path_utils;
pub mod scorer;
pub mod tor;

use std::sync::Arc;
use tauri::Emitter;
use tauri::Manager;
use tokio::sync::Mutex;

/// Global shared frontier for cancellation support
struct AppState {
    active_frontier: Mutex<Option<Arc<frontier::CrawlerFrontier>>>,
    vfs: db::SledVfs,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CrawlStatusUpdate {
    phase: String,
    progress_percent: f64,
    visited_nodes: usize,
    processed_nodes: usize,
    queued_nodes: usize,
    active_workers: usize,
    worker_target: usize,
    eta_seconds: Option<u64>,
    estimation: String,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct DownloadBatchStartedEvent {
    total_files: usize,
    total_bytes_hint: u64,
    unknown_size_files: usize,
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
            let _ = app.emit("crawl_status_update", payload);

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

async fn start_aria_single_download(
    app: &tauri::AppHandle,
    url: String,
    safe_target: String,
    circuits: usize,
    force_tor: bool,
) -> Result<(), String> {
    let control = aria_downloader::activate_download_control()
        .ok_or_else(|| "Auto-download skipped: another download is already active.".to_string())?;

    let result = aria_downloader::start_download(
        app.clone(),
        url,
        safe_target.clone(),
        circuits.max(1),
        force_tor,
        std::path::Path::new(&safe_target)
            .parent()
            .map(|p| p.to_string_lossy().to_string()),
        control,
    )
    .await;
    aria_downloader::clear_download_control();
    result.map_err(|e| e.to_string())
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
) -> Result<Vec<adapters::FileEntry>, String> {
    let is_onion = url.contains(".onion");
    let output_root = canonical_output_root(&output_dir)?;
    let output_root_str = output_root.to_string_lossy().to_string();
    let support_dir = output_root.join("temp_onionforge_forger");
    tokio::fs::create_dir_all(&support_dir)
        .await
        .map_err(|e| format!("Failed to create support directory: {e}"))?;

    let mut swarm_guard = None;
    let mut active_ports = Vec::new();
    let auto_download = options.download;

    if is_onion {
        // Flush any zombie Tor daemons from previous sessions before starting new ones
        tor::cleanup_stale_tor_daemons();

        app.emit(
            "crawl_log",
            format!("[System] Bootstrapping Target: {}", url),
        )
        .unwrap();

        let target_daemons = options.daemons.unwrap_or(if cfg!(target_os = "windows") { 8 } else { 12 }).max(1);
        match tor::bootstrap_tor_cluster(app.clone(), target_daemons).await {
            Ok((guard, ports)) => {
                swarm_guard = Some(guard);
                active_ports = ports;
            }
            Err(e) => return Err(format!("Failed to start Tor Swarm: {}", e)),
        }
    }

    let daemon_count = if is_onion {
        active_ports.len().max(1)
    } else {
        options.daemons.unwrap_or(if cfg!(target_os = "windows") { 8 } else { 12 }).max(1)
    };

    let mut frontier = frontier::CrawlerFrontier::new(
        Some(app.clone()),
        url.clone(),
        daemon_count,
        is_onion,
        active_ports,
        options.clone(),
    );
    frontier.swarm_guard = swarm_guard;

    // Initialize VFS Database for this crawl session
    let state = app.state::<AppState>();
    let vfs_path = support_dir.join(".crawli_vtdb");
    let vfs_path_str = vfs_path.to_string_lossy().to_string();
    let _ = state.vfs.initialize(&vfs_path_str).await;
    if !options.resume {
        let _ = state.vfs.clear().await;
    }

    let client = frontier.get_client().1;
    app.emit(
        "crawl_log",
        "[System] Generating Site Fingerprint...".to_string(),
    )
    .unwrap();

    let resp = match client.get(&url).send().await {
        Ok(r) => r,
        Err(e) => return Err(format!("OFFLINE_SYNC_ERROR: The site might be down. Please manually check it to verify if it is actually functional and active. ({})", e)),
    };

    let status = resp.status().as_u16();
    let headers = resp.headers().clone();

    // Safety Net: Prevent reqwest from trying to UTF-8 decode a massive 20GB .7z payload
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
        url: url.clone(),
        status,
        headers,
        body,
    };

    if let Some(name) = looks_like_direct_artifact(&url, is_binary) {
        // Resolve adapter identity for direct artifact mode so UI can surface it reliably.
        let registry = adapters::AdapterRegistry::new();
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

        // Direct file URL interception happens before adapter resolution so detection-only
        // adapters never short-circuit raw artifact downloads.
        let size = fingerprint
            .headers
            .get("content-length")
            .and_then(|h| h.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok());
        let entry = adapters::FileEntry {
            path: name.clone(),
            size_bytes: size,
            entry_type: adapters::EntryType::File,
            raw_url: url.clone(),
        };
        app.emit(
            "crawl_log",
            format!(
                "[System] Raw File Target Intercepted: {}. Enqueueing directly to Aria Forge.",
                name
            ),
        )
        .unwrap();
        let fallback_entries = vec![entry.clone()];
        let _ = app.emit("crawl_progress", fallback_entries.clone());

        if auto_download {
            let circuits = options.circuits.unwrap_or(120).max(1);
            let safe_target = path_utils::resolve_download_target_within_root(&output_root, &name)
                .map_err(|e| format!("Direct artifact path rejected: {e}"))?;
            let safe_target_str = safe_target.to_string_lossy().to_string();
            app.emit(
                "crawl_log",
                format!(
                    "[OPSEC] Auto-Mirror engaged for direct artifact via Aria Forge. Target: {} | Circuits: {}",
                    safe_target_str, circuits
                ),
            )
            .unwrap_or_default();

            match start_aria_single_download(&app, url.clone(), safe_target_str, circuits, is_onion)
                .await
            {
                Ok(_) => {
                    app.emit(
                        "crawl_log",
                        "[OPSEC] Direct artifact download task finished.".to_string(),
                    )
                    .unwrap_or_default();
                }
                Err(e) => {
                    app.emit(
                        "crawl_log",
                        format!("[ERROR] Direct artifact download failed: {}", e),
                    )
                    .unwrap_or_default();
                }
            }
        }

        let arc_frontier = std::sync::Arc::new(frontier);
        {
            let state = app.state::<AppState>();
            let mut lock = state.active_frontier.lock().await;
            *lock = Some(arc_frontier.clone());
        }
        let _ = app.emit(
            "crawl_status_update",
            crawl_status_snapshot(arc_frontier.as_ref(), "complete", 100.0, Some(0)),
        );
        return Ok(fallback_entries);
    }

    let registry = adapters::AdapterRegistry::new();
    if let Some(adapter) = registry.determine_adapter(&fingerprint).await {
        app.emit(
            "crawl_log",
            format!("[Adapter] Match found: {}", adapter.name()),
        )
        .unwrap();

        let arc_frontier = std::sync::Arc::new(frontier);

        // Store frontier in app state for cancel support
        {
            let state = app.state::<AppState>();
            let mut lock = state.active_frontier.lock().await;
            *lock = Some(arc_frontier.clone());
        }

        let _ = app.emit(
            "crawl_status_update",
            crawl_status_snapshot(arc_frontier.as_ref(), "probing", 0.0, None),
        );
        let status_emitter = spawn_crawl_status_emitter(app.clone(), arc_frontier.clone());

        let crawl_result = adapter.crawl(&url, arc_frontier.clone(), app.clone()).await;

        status_emitter.abort();
        let _ = status_emitter.await;

        let final_payload = if arc_frontier.is_cancelled() {
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
        let _ = app.emit("crawl_status_update", final_payload);

        match crawl_result {
            Ok(files) => {
                // We keep the active_frontier alive here so that the UI can manually trigger
                // "Download All" later and still have access to the warm Tor circuits!

                // Auto-download if enabled
                if auto_download {
                    if files.is_empty() {
                        app.emit(
                            "crawl_log",
                            "[OPSEC] Auto-Mirror skipped: adapter returned 0 downloadable entries."
                                .to_string(),
                        )
                        .unwrap_or_default();
                    } else {
                        let circuits = options.circuits.unwrap_or(120).max(1);
                        app.emit("crawl_log", format!("[OPSEC] Auto-Mirror engaged. Routing {} entries through Aria mirror pipeline to {}", files.len(), output_root_str)).unwrap();
                        match scaffold_download(&files, &output_root, &app, circuits, is_onion)
                            .await
                        {
                            Ok(count) => {
                                app.emit(
                                    "crawl_log",
                                    format!(
                                        "[OPSEC] Mirror complete. {} items written to disk.",
                                        count
                                    ),
                                )
                                .unwrap();
                            }
                            Err(e) => {
                                app.emit("crawl_log", format!("[ERROR] Mirror failed: {}", e))
                                    .unwrap();
                            }
                        }
                    }
                }
                Ok(files)
            }
            Err(e) => {
                // Clear active frontier on error too
                let state = app.state::<AppState>();
                let mut lock = state.active_frontier.lock().await;
                *lock = None;
                Err(e.to_string())
            }
        }
    } else {
        Err("No known adapter matched this sites architecture.".to_string())
    }
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
                if entry.raw_url.starts_with("http://") || entry.raw_url.starts_with("https://") {
                    batch_files.push(BatchFileEntry {
                        url: entry.raw_url.clone(),
                        path: safe_target,
                        size_hint: entry.size_bytes,
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
    app: tauri::AppHandle,
) -> Result<u32, String> {
    use tauri::Emitter;
    let _ = app.emit(
        "crawl_log",
        "[System] Querying Sled VFS for full mirroring operation...",
    );
    let state = app.state::<AppState>();
    let entries = state.vfs.iter_entries().await.map_err(|e| e.to_string())?;
    let output_root = canonical_output_root(&output_dir)?;
    let force_tor = entries.iter().any(|entry| entry.raw_url.contains(".onion"));
    let circuits = connections.unwrap_or(120).max(1);
    scaffold_download(&entries, &output_root, &app, circuits, force_tor)
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
        let result = aria_downloader::start_download(
            app_clone.clone(),
            url,
            safe_target_str,
            connections,
            force_tor,
            Some(output_root.to_string_lossy().to_string()),
            control,
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Flush any zombie Tor daemons from previous sessions on startup
    tor::cleanup_stale_tor_daemons();

    tauri::Builder::default()
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState {
            active_frontier: Mutex::new(None),
            vfs: db::SledVfs::default(),
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
            ingest_vfs_entries
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
