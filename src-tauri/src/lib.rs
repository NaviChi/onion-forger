pub mod adapters;
pub mod tor;
pub mod frontier;
pub mod aimd;
pub mod scorer;
pub mod path_utils;
pub mod aria_downloader;
pub mod io_vanguard;
pub mod db;
pub mod kalman;
pub mod bft_quorum;

use tauri::Emitter;
use tauri::Manager;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Global shared frontier for cancellation support
struct AppState {
    active_frontier: Mutex<Option<Arc<frontier::CrawlerFrontier>>>,
    vfs: db::SledVfs,
}

#[tauri::command]
async fn get_vfs_children(parent_path: String, app: tauri::AppHandle) -> Result<Vec<adapters::FileEntry>, String> {
    let state = app.state::<AppState>();
    state.vfs.get_children(&parent_path).await.map_err(|e| e.to_string())
}

// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
#[tauri::command]
async fn start_crawl(url: String, options: frontier::CrawlOptions, output_dir: String, app: tauri::AppHandle) -> Result<Vec<adapters::FileEntry>, String> {
    let is_onion = url.contains(".onion");
    
    let mut swarm_guard = None;
    let mut active_ports = Vec::new();
    let auto_download = options.download;

    if is_onion {
        // Flush any zombie Tor daemons from previous sessions before starting new ones
        tor::cleanup_stale_tor_daemons();
        
        app.emit("crawl_log", format!("[System] Bootstrapping Target: {}", url)).unwrap();
        match tor::bootstrap_tor_cluster(app.clone(), 4).await {
            Ok((guard, ports)) => {
                swarm_guard = Some(guard);
                active_ports = ports;
            },
            Err(e) => return Err(format!("Failed to start Tor Swarm: {}", e)),
        }
    }

    let mut frontier = frontier::CrawlerFrontier::new(Some(app.clone()), url.clone(), 4, is_onion, active_ports, options.clone());
    frontier.swarm_guard = swarm_guard;

    // Initialize VFS Database for this crawl session
    let state = app.state::<AppState>();
    let vfs_path = format!("{}/.crawli_vtdb", output_dir); // VFS temporary DB
    let _ = state.vfs.initialize(&vfs_path).await;
    let _ = state.vfs.clear().await;

    let client = frontier.get_client().1;
    app.emit("crawl_log", "[System] Generating Site Fingerprint...".to_string()).unwrap();

    let resp = match client.get(&url).send().await {
        Ok(r) => r,
        Err(e) => return Err(format!("OFFLINE_SYNC_ERROR: The site might be down. Please manually check it to verify if it is actually functional and active. ({})", e)),
    };
    
    let status = resp.status().as_u16();
    let headers = resp.headers().clone();
    
    // Safety Net: Prevent reqwest from trying to UTF-8 decode a massive 20GB .7z payload
    let content_type = headers.get("content-type").and_then(|h| h.to_str().ok()).unwrap_or("");
    let is_binary = !content_type.starts_with("text/") && !content_type.contains("json") && !content_type.contains("xml") && (content_type.contains("application/") || url.ends_with(".7z") || url.ends_with(".zip") || url.ends_with(".rar"));
    
    let body = if is_binary {
        "[BINARY_OR_ARCHIVE_DATA]".to_string()
    } else {
        resp.text().await.unwrap_or_else(|_| "[DECODE_ERROR]".to_string())
    };

    let fingerprint = adapters::SiteFingerprint {
        url: url.clone(),
        status,
        headers,
        body,
    };

    let registry = adapters::AdapterRegistry::new();
    if let Some(adapter) = registry.determine_adapter(&fingerprint).await {
        app.emit("crawl_log", format!("[Adapter] Match found: {}", adapter.name())).unwrap();
        
        let arc_frontier = std::sync::Arc::new(frontier);
        
        // Store frontier in app state for cancel support
        {
            let state = app.state::<AppState>();
            let mut lock = state.active_frontier.lock().await;
            *lock = Some(arc_frontier.clone());
        }
        
        match adapter.crawl(&url, arc_frontier.clone(), app.clone()).await {
            Ok(files) => {
                // We keep the active_frontier alive here so that the UI can manually trigger 
                // "Download All" later and still have access to the warm Tor circuits!
                
                // Auto-download if enabled
                if auto_download && !output_dir.is_empty() {
                    app.emit("crawl_log", format!("[OPSEC] Auto-Mirror engaged. Scaffolding {} nodes to {}", files.len(), output_dir)).unwrap();
                    match scaffold_download(&files, &output_dir, &app).await {
                        Ok(count) => {
                            app.emit("crawl_log", format!("[OPSEC] Mirror complete. {} items written to disk.", count)).unwrap();
                        },
                        Err(e) => {
                            app.emit("crawl_log", format!("[ERROR] Mirror failed: {}", e)).unwrap();
                        }
                    }
                }
                Ok(files)
            },
            Err(e) => {
                // Clear active frontier on error too
                let state = app.state::<AppState>();
                let mut lock = state.active_frontier.lock().await;
                *lock = None;
                Err(e.to_string())
            },
        }
    } else {
        // Fallback: If it's a direct artifact file link instead of a root directory mapping, 
        // silently intercept the target and spawn it into the VFS directly for Aria extraction
        let name = url.split('/').last().unwrap_or("artifact").to_string();
        if is_binary || name.contains('.') { 
            let size = fingerprint.headers.get("content-length").and_then(|h| h.to_str().ok()).and_then(|s| s.parse::<u64>().ok());
            let entry = adapters::FileEntry {
                path: name.clone(),
                size_bytes: size,
                entry_type: adapters::EntryType::File,
                raw_url: url.clone(),
            };
            app.emit("crawl_log", format!("[System] Raw File Target Intercepted: {}. Enqueueing directly to Swarm Engine.", name)).unwrap();
            
            let arc_frontier = std::sync::Arc::new(frontier);
            {
                let state = app.state::<AppState>();
                let mut lock = state.active_frontier.lock().await;
                *lock = Some(arc_frontier);
            }
            return Ok(vec![entry]);
        }

        Err("No known adapter matched this sites architecture.".to_string())
    }
}

/// Sled IPC Receiver: Persist incoming entries silently in the background
#[tauri::command]
async fn ingest_vfs_entries(entries: Vec<adapters::FileEntry>, app: tauri::AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    state.vfs.insert_entries(&entries).await.map_err(|e| e.to_string())
}

/// Downloads / scaffolds all discovered FileEntries onto disk.
/// - Folders: creates the directory tree
/// - Files: creates empty placeholder files preserving the full path hierarchy
///          (actual content download via Tor would happen here in production)
/// - Handles 0-byte files, missing sizes, and deeply nested paths gracefully
#[tauri::command]
async fn download_files(entries: Vec<adapters::FileEntry>, output_dir: String, app: tauri::AppHandle) -> Result<u32, String> {
    scaffold_download(&entries, &output_dir, &app).await.map_err(|e| e.to_string())
}

#[derive(Clone, serde::Serialize)]
struct DownloadProgressEvent {
    path: String,
    bytes_downloaded: u64,
    total_bytes: Option<u64>,
    speed_bps: u64,
    active_circuits: usize,
}

async fn scaffold_download(entries: &[adapters::FileEntry], output_dir: &str, app: &tauri::AppHandle) -> anyhow::Result<u32> {
    use std::path::PathBuf;
    use tauri::Manager;
    use tokio::io::AsyncWriteExt;
    use futures::StreamExt;
    
    let base = PathBuf::from(output_dir);
    tokio::fs::create_dir_all(&base).await?;
    
    let active_arc = {
        let state = app.state::<AppState>();
        let lock = state.active_frontier.lock().await;
        lock.clone()
    };
    
    let written = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    let total = entries.len();
    
    // Process bulk downloads with a balanced concurrency of 8 to avoid Tor circuit collision
    // (Aria chunking is used for single massive files, while this balances mass-node discovery)
    futures::stream::iter(entries.iter().enumerate()).for_each_concurrent(8, |(_idx, entry)| {
        let app_clone = app.clone();
        let active_clone = active_arc.clone();
        let base_clone = base.clone();
        let written_clone = written.clone();
        
        async move {
            let sanitized = path_utils::sanitize_path(&entry.path);
            if sanitized.is_empty() { return; }
            
            let full_path = base_clone.join(&sanitized);
            let http_client = if let Some(f) = active_clone {
                f.get_client().1
            } else {
                reqwest::Client::new()
            };
            
            let write_to_disk = |path: &PathBuf, data: &[u8]| -> anyhow::Result<()> {
                use std::io::Write;
                let mut file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).open(path)?;
                file.write_all(data)?;
                Ok(())
            };
            
            match entry.entry_type {
                adapters::EntryType::Folder => {
                    let _ = tokio::fs::create_dir_all(&full_path).await;
                    let gitkeep = full_path.join(".gitkeep");
                    if !gitkeep.exists() {
                        let _ = tokio::fs::write(&gitkeep, b"").await;
                    }
                },
                adapters::EntryType::File => {
                    if let Some(parent) = full_path.parent() {
                        let _ = tokio::fs::create_dir_all(parent).await;
                    }
                    
                    if !full_path.exists() {
                        match tokio::time::timeout(std::time::Duration::from_secs(45), http_client.get(&entry.raw_url).send()).await {
                            Ok(Ok(mut resp)) => {
                                if resp.status().is_success() {
                                    if let Ok(mut file) = tokio::fs::File::create(&full_path).await {
                                        let mut downloaded = 0;
                                        let start_time = std::time::Instant::now();
                                        let mut last_emit = std::time::Instant::now();
                                        
                                        while let Ok(Some(chunk)) = tokio::time::timeout(std::time::Duration::from_secs(15), resp.chunk()).await.unwrap_or(Ok(None)) {
                                            if chunk.is_empty() { continue; }
                                            let _ = file.write_all(&chunk).await;
                                            downloaded += chunk.len() as u64;
                                            
                                            if last_emit.elapsed().as_millis() > 150 {
                                                last_emit = std::time::Instant::now();
                                                let elapsed = start_time.elapsed().as_secs_f64().max(0.001);
                                                let speed = (downloaded as f64 / elapsed) as u64;
                                                
                                                let _ = app_clone.emit("download_progress_update", DownloadProgressEvent {
                                                    path: entry.path.clone(),
                                                    bytes_downloaded: downloaded,
                                                    total_bytes: entry.size_bytes.or_else(|| match resp.content_length() {
                                                        Some(l) => Some(l),
                                                        None => None,
                                                    }),
                                                    speed_bps: speed,
                                                    active_circuits: 1, // Single native connection
                                                });
                                            }
                                        }
                                        
                                        let elapsed = start_time.elapsed().as_secs_f64().max(0.001);
                                        let speed = (downloaded as f64 / elapsed) as u64;
                                        let content_len = entry.size_bytes.or(resp.content_length());
                                        let _ = app_clone.emit("download_progress_update", DownloadProgressEvent {
                                            path: entry.path.clone(),
                                            bytes_downloaded: downloaded,
                                            total_bytes: Some(content_len.unwrap_or(downloaded).max(downloaded)),
                                            speed_bps: speed,
                                            active_circuits: 1, // Single native connection
                                        });
                                    }
                                } else {
                                    let _ = write_to_disk(&full_path, b"");
                                }
                            }
                            _ => {
                                let _ = write_to_disk(&full_path, b"");
                            }
                        }
                    }
                    
                    let meta_path = PathBuf::from(format!("{}.onionforge.meta", full_path.display()));
                    let size_str = entry.size_bytes.map(|s: u64| s.to_string()).unwrap_or_else(|| "0".to_string());
                    let meta_content = format!("url={}\nsize={}\ntype=file\noriginal_path={}\n", entry.raw_url, size_str, entry.path);
                    let _ = tokio::fs::write(&meta_path, meta_content.as_bytes()).await;
                }
            }
            
            let w = written_clone.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            if w % 50 == 0 || w == total as u32 {
                let _ = app_clone.emit("crawl_log", format!("[MIRROR] {}/{} items scaffolded", w, total));
            }
        }
    }).await;
    
    let written_final = written.load(std::sync::atomic::Ordering::Relaxed);
    
    // Write a manifest index at the root
    let manifest_path = base.join("_onionforge_manifest.txt");
    let mut manifest = String::new();
    manifest.push_str(&format!("# OnionForge Download Manifest\n"));
    manifest.push_str(&format!("# Generated: {}\n", chrono_stub()));
    manifest.push_str(&format!("# Total Entries: {}\n\n", entries.len()));
    for entry in entries {
        let type_tag = match entry.entry_type {
            adapters::EntryType::Folder => "DIR ",
            adapters::EntryType::File => "FILE",
        };
        let size_tag = entry.size_bytes.map(|s| format_bytes(s)).unwrap_or_else(|| "0 B".to_string());
        let decoded_path = path_utils::url_decode(&entry.path);
        manifest.push_str(&format!("{} {:>12}  {}  {}\n", type_tag, size_tag, decoded_path, entry.raw_url));
    }
    tokio::fs::write(&manifest_path, manifest.as_bytes()).await?;
    
    Ok(written_final)
}

fn chrono_stub() -> String {
    // Simple timestamp without adding chrono dependency
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
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
    let lock: tokio::sync::MutexGuard<'_, Option<Arc<frontier::CrawlerFrontier>>> = state.active_frontier.lock().await;
    
    // Attempt to stop any active single file downloads utilizing the Aria engine lock
    let _ = stop_active_download(app.clone());

    if let Some(ref frontier) = *lock {
        frontier.cancel();
        app.emit("crawl_log", "[System] ⚠ Cancellation signal sent to all workers.").unwrap_or_default();
        Ok("Cancellation signal sent.".to_string())
    } else {
        Ok("Crawl cancelled and downloads halted.".to_string())
    }
}

#[tauri::command]
async fn export_json(output_path: String, app: tauri::AppHandle) -> Result<String, String> {
    let state = app.state::<AppState>();
    let entries = state.vfs.iter_entries().await.map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(&entries).map_err(|e| e.to_string())?;
    tokio::fs::write(&output_path, json.as_bytes()).await.map_err(|e| e.to_string())?;
    Ok(format!("Exported {} entries to {}", entries.len(), output_path))
}

#[tauri::command]
async fn download_all(output_dir: String, app: tauri::AppHandle) -> Result<u32, String> {
    use tauri::Emitter;
    let _ = app.emit("crawl_log", "[System] Querying Sled VFS for full mirroring operation...");
    let state = app.state::<AppState>();
    let entries = state.vfs.iter_entries().await.map_err(|e| e.to_string())?;
    scaffold_download(&entries, &output_dir, &app).await.map_err(|e| e.to_string())
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct DownloadArgs {
    url: String,
    path: String,
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
        connections,
        force_tor,
    } = args;

    app.emit("log", format!("Initiating extraction for: {url}")).ok();

    let app_clone = app.clone();
    let fail_url = url.clone();
    let fail_path = path.clone();

    tokio::spawn(async move {
        let result = aria_downloader::start_download(
            app_clone.clone(),
            url,
            path,
            connections,
            force_tor,
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
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(AppState {
            active_frontier: Mutex::new(None),
            vfs: db::SledVfs::default(),
        })
        .invoke_handler(tauri::generate_handler![start_crawl, download_files, download_all, cancel_crawl, export_json, initiate_download, pause_active_download, stop_active_download, get_vfs_children, ingest_vfs_entries])
        .on_window_event(|_window, event| {
            if let tauri::WindowEvent::Destroyed = event {
                // Kill any remaining Tor processes on window close
                tor::cleanup_stale_tor_daemons();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
