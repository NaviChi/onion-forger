use crate::adapters::{CrawlerAdapter, EntryType, FileEntry, SiteFingerprint};
use crate::frontier::CrawlerFrontier;
use crate::path_utils;
use std::sync::Arc;
use tauri::AppHandle;
use tokio::sync::mpsc;

/// Abyss Ransomware Adapter
///
/// Abyss typically exposes direct .rar / .zip archive download links.
/// The URL pattern is `http://<onion>/<filename>.rar` — a direct artifact.
/// However, they may also have an autoindex-like listing page.
///
/// Strategy:
/// 1. If the URL points to a direct file (.rar, .zip, .7z, etc), treat as single file.
/// 2. If it's a directory listing, parse the HTML for links.
/// 3. Fall back to autoindex for recursive traversal if applicable.
#[derive(Default)]
pub struct AbyssAdapter;

/// Known archive extensions for direct-file detection
const ARCHIVE_EXTENSIONS: &[&str] = &[
    ".rar", ".zip", ".7z", ".tar", ".gz", ".bz2", ".xz",
    ".tar.gz", ".tar.bz2", ".tar.xz", ".tgz",
];

/// Check if a URL points to a direct downloadable archive
fn is_direct_archive_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    ARCHIVE_EXTENSIONS.iter().any(|ext| lower.ends_with(ext))
}

/// Extract filename from a direct archive URL
fn extract_filename_from_url(url: &str) -> String {
    if let Ok(parsed) = url::Url::parse(url) {
        parsed
            .path_segments()
            .and_then(|segments| segments.last())
            .map(|s| path_utils::url_decode(s))
            .unwrap_or_else(|| "unknown_archive".to_string())
    } else {
        url.rsplit('/')
            .find(|s| !s.is_empty())
            .unwrap_or("unknown_archive")
            .to_string()
    }
}

/// Parse Abyss-style HTML listings. Abyss may use a minimal custom HTML layout
/// or a standard autoindex. We handle both.
fn parse_abyss_listing(html: &str, base_url: &str) -> Vec<FileEntry> {
    let mut entries = Vec::new();

    // Try to extract <a href="..."> links
    for line in html.lines() {
        if let Some(href_start) = line.find("href=\"") {
            let after_href = &line[href_start + 6..];
            if let Some(href_end) = after_href.find('"') {
                let raw_href = &after_href[..href_end];

                // Skip navigation/back links
                if raw_href == "../"
                    || raw_href == ".."
                    || raw_href == "/"
                    || raw_href.starts_with("?")
                    || raw_href.starts_with("javascript:")
                    || raw_href.starts_with("#")
                {
                    continue;
                }

                let decoded_name = path_utils::url_decode(raw_href);
                let is_dir = raw_href.ends_with('/');
                let clean_name = decoded_name.trim_end_matches('/').to_string();

                if clean_name.is_empty() {
                    continue;
                }

                // Build absolute URL
                let child_url = if raw_href.starts_with("http://") || raw_href.starts_with("https://") {
                    raw_href.to_string()
                } else {
                    let encoded = path_utils::url_encode(&clean_name);
                    if is_dir {
                        format!("{}/{}/", base_url.trim_end_matches('/'), encoded)
                    } else {
                        format!("{}/{}", base_url.trim_end_matches('/'), encoded)
                    }
                };

                // Try to extract size from the line (after </a>)
                let size = extract_size_from_line(line);

                entries.push(FileEntry {
                    path: format!("/{}", path_utils::sanitize_path(&clean_name)),
                    size_bytes: size,
                    entry_type: if is_dir {
                        EntryType::Folder
                    } else {
                        EntryType::File
                    },
                    raw_url: child_url,
                });
            }
        }
    }

    entries
}

/// Extract file size from an HTML line — supports both raw bytes and human-readable K/M/G.
fn extract_size_from_line(line: &str) -> Option<u64> {
    if let Some(after_tag) = line.split("</a>").nth(1) {
        let tokens: Vec<&str> = after_tag.split_whitespace().collect();
        if let Some(last) = tokens.last() {
            let last_upper = last.trim().to_uppercase();
            if let Ok(size) = last_upper.parse::<u64>() {
                return Some(size);
            }
            let mut num_str = last_upper.clone();
            let mut multiplier: u64 = 1;
            if num_str.ends_with('K') {
                num_str.pop();
                multiplier = 1024;
            } else if num_str.ends_with('M') {
                num_str.pop();
                multiplier = 1024 * 1024;
            } else if num_str.ends_with('G') {
                num_str.pop();
                multiplier = 1024 * 1024 * 1024;
            }
            if let Ok(num) = num_str.parse::<f64>() {
                return Some((num * multiplier as f64) as u64);
            }
        }
    }
    None
}

#[async_trait::async_trait]
impl CrawlerAdapter for AbyssAdapter {
    async fn can_handle(&self, fingerprint: &SiteFingerprint) -> bool {
        // Abyss detection:
        // 1. URL contains 'abyss' or known Abyss onion domain patterns
        // 2. Direct archive link (e.g., .rar download)
        // 3. Body contains 'abyss' markers
        let url_lower = fingerprint.url.to_ascii_lowercase();
        let body_lower = fingerprint.body.to_ascii_lowercase();

        url_lower.contains("abyss")
            || body_lower.contains("abyss")
            || (is_direct_archive_url(&fingerprint.url)
                && url_lower.contains("vmmefm7ktazj2bwtmy46o3wxhk42tctasyyqv6ymuzlivszteyhkkyad"))
    }

    async fn crawl(
        &self,
        current_url: &str,
        frontier: Arc<CrawlerFrontier>,
        app: AppHandle,
    ) -> anyhow::Result<Vec<FileEntry>> {
        use tauri::Emitter;

        // CASE 1: Direct archive download URL
        if is_direct_archive_url(current_url) {
            let filename = extract_filename_from_url(current_url);
            let _ = app.emit("log", format!("[Abyss] Direct archive detected: {}", filename));

            // Probe for file size via HEAD request
            let (cid, client) = frontier.get_client();
            let start = std::time::Instant::now();
            let mut size_bytes = None;

            if let Ok(Ok(resp)) = tokio::time::timeout(
                std::time::Duration::from_secs(30),
                client.head(current_url).send(),
            )
            .await
            {
                size_bytes = resp
                    .headers()
                    .get("content-length")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.parse::<u64>().ok());
                frontier.record_success(cid, 0, start.elapsed().as_millis() as u64);
            } else {
                frontier.record_failure(cid);
            }

            let entry = FileEntry {
                path: format!("/{}", path_utils::sanitize_path(&filename)),
                size_bytes,
                entry_type: EntryType::File,
                raw_url: current_url.to_string(),
            };

            let _ = app.emit("crawl_progress", vec![entry.clone()]);
            return Ok(vec![entry]);
        }

        // CASE 2: Directory listing / recursive traversal
        let queue = Arc::new(crossbeam_queue::SegQueue::new());
        let all_discovered_entries = Arc::new(tokio::sync::Mutex::new(Vec::new()));

        queue.push(current_url.to_string());
        frontier.mark_visited(current_url);

        let pending = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        pending.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        // Batched UI backpressure
        let (ui_tx, mut ui_rx) = mpsc::channel::<FileEntry>(50000);
        let ui_app = app.clone();
        tokio::spawn(async move {
            let mut batch = Vec::new();
            let mut interval = tokio::time::interval(std::time::Duration::from_millis(500));
            loop {
                tokio::select! {
                    Some(entry) = ui_rx.recv() => {
                        batch.push(entry);
                        if batch.len() >= 500 {
                            let _ = ui_app.emit("crawl_progress", batch.clone());
                            batch.clear();
                        }
                    }
                    _ = interval.tick() => {
                        if !batch.is_empty() {
                            let _ = ui_app.emit("crawl_progress", batch.clone());
                            batch.clear();
                        }
                    }
                    else => break,
                }
            }
        });

        let max_concurrent = frontier.recommended_listing_workers();

        let mut workers = tokio::task::JoinSet::new();

        for _ in 0..max_concurrent {
            let f = frontier.clone();
            let q_clone = queue.clone();
            let ui_tx_clone = ui_tx.clone();
            let discovered_ref = all_discovered_entries.clone();
            let pending_clone = pending.clone();

            workers.spawn(async move {
                let mut idle_sleep_ms: u64 = 50;
                let mut ddos_guard = crate::adapters::qilin_ddos_guard::DdosGuard::new();
                loop {
                    if f.is_cancelled() {
                        return;
                    }

                    let next_url = match q_clone.pop() {
                        Some(url) => {
                            idle_sleep_ms = 50;
                            url
                        }
                        None => {
                            if pending_clone.load(std::sync::atomic::Ordering::SeqCst) == 0 {
                                break;
                            }
                            tokio::time::sleep(std::time::Duration::from_millis(idle_sleep_ms))
                                .await;
                            idle_sleep_ms = std::cmp::min(idle_sleep_ms * 2, 800);
                            continue;
                        }
                    };

                    let _permit = f.politeness_semaphore.acquire().await.ok();
                    let (cid, client) = f.get_client();

                    let delay = f.scorer.yield_delay(cid);
                    if delay > std::time::Duration::ZERO {
                        tokio::time::sleep(delay).await;
                    }

                    let start_time = std::time::Instant::now();
                    let mut bytes_downloaded = 0;
                    let (mut fetch_success, mut html) = (false, None);

                    // Try up to 3 retries with circuit rotation
                    for attempt in 0..3 {
                        let (retry_cid, retry_client) = if attempt == 0 {
                            (cid, client.clone())
                        } else {
                            f.get_client()
                        };

                        if let Ok(Ok(resp)) = tokio::time::timeout(
                            std::time::Duration::from_secs(45),
                            retry_client.get(&next_url).send(),
                        )
                        .await
                        {
                            if let Some(delay) = ddos_guard.record_response(resp.status().as_u16()) {
                                tokio::time::sleep(delay).await;
                            }
                            if resp.status().is_success() {
                                if let Ok(body) = resp.text().await {
                                    bytes_downloaded += body.len() as u64;
                                    fetch_success = true;
                                    html = Some(body);
                                    f.record_success(
                                        retry_cid,
                                        bytes_downloaded,
                                        start_time.elapsed().as_millis() as u64,
                                    );
                                    break;
                                }
                            } else if resp.status().as_u16() == 404 {
                                f.record_failure(retry_cid);
                                break;
                            }
                        }
                        f.record_failure(retry_cid);
                    }

                    if !fetch_success {
                        pending_clone.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                        continue;
                    }

                    let Some(html) = html else {
                        pending_clone.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                        continue;
                    };

                    if !f.active_options.listing {
                        pending_clone.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                        continue;
                    }

                    // Parse the page for links
                    let base_url_clone = next_url.clone();
                    let parsed_entries =
                        tokio::task::spawn_blocking(move || parse_abyss_listing(&html, &base_url_clone))
                            .await
                            .unwrap_or_default();

                    let mut new_files = Vec::new();
                    for entry in parsed_entries {
                        if entry.entry_type == EntryType::Folder {
                            if f.mark_visited(&entry.raw_url) {
                                pending_clone.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                                q_clone.push(entry.raw_url.clone());
                            }
                        }
                        new_files.push(entry);
                    }

                    // Send to IPC batcher
                    for file in &new_files {
                        let _ = ui_tx_clone.send(file.clone()).await;
                    }

                    if !new_files.is_empty() {
                        let mut lock = discovered_ref.lock().await;
                        lock.extend(new_files);
                    }

                    pending_clone.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                }
            });
        }

        while workers.join_next().await.is_some() {}

        drop(ui_tx);
        let mut final_results = all_discovered_entries.lock().await;
        Ok(final_results.drain(..).collect())
    }

    fn name(&self) -> &'static str {
        "Abyss Ransomware"
    }

    fn known_domains(&self) -> Vec<&'static str> {
        vec![
            "vmmefm7ktazj2bwtmy46o3wxhk42tctasyyqv6ymuzlivszteyhkkyad.onion",
        ]
    }

    fn regex_marker(&self) -> Option<&'static str> {
        Some(r"(?i)abyss")
    }
}
