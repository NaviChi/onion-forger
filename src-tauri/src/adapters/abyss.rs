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
    ".rar", ".zip", ".7z", ".tar", ".gz", ".bz2", ".xz", ".tar.gz", ".tar.bz2", ".tar.xz", ".tgz",
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
                let child_url =
                    if raw_href.starts_with("http://") || raw_href.starts_with("https://") {
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
                    jwt_exp: None,
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
            let _ = app.emit(
                "log",
                format!("[Abyss] Direct archive detected: {}", filename),
            );

            // Probe for file size via GET Range request
            let (cid, client) = frontier.get_client();
            let start = std::time::Instant::now();
            let mut size_bytes = None;

            if let Ok(Ok(size_resp)) = tokio::time::timeout(
                std::time::Duration::from_secs(30),
                client.get(current_url).header("Range", "bytes=0-0").send(),
            )
            .await
            {
                size_bytes = size_resp
                    .headers()
                    .get("content-range")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.split('/').last())
                    .and_then(|s| s.parse::<u64>().ok())
                    .or_else(|| {
                        size_resp
                            .headers()
                            .get("content-length")
                            .and_then(|v| v.to_str().ok())
                            .and_then(|s| s.parse::<u64>().ok())
                    });
                frontier.record_success(cid, 0, start.elapsed().as_millis() as u64);
            } else {
                frontier.record_failure(cid);
            }

            let entry = FileEntry {
                jwt_exp: None,
                path: format!("/{}", path_utils::sanitize_path(&filename)),
                size_bytes,
                entry_type: EntryType::File,
                raw_url: current_url.to_string(),
            };

            let _ = app.emit("crawl_progress", vec![entry.clone()]);
            return Ok(vec![entry]);
        }

        // CASE 2: Directory listing / recursive traversal
        let queue = Arc::new(crate::spillover::SpilloverQueue::new());
        let all_discovered_entries = Arc::new(crate::spillover::SpilloverList::new());

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

        // Phase 119: work-stealing with inverted retry queue
        let retry_q = Arc::new(crate::work_stealing::new_retry_queue());
        let max_concurrent = frontier.recommended_listing_workers().min(12);

        let mut workers = tokio::task::JoinSet::new();

        for worker_idx in 0..max_concurrent {
            let f = frontier.clone();
            let q_clone = queue.clone();
            let ui_tx_clone = ui_tx.clone();
            let discovered_ref = all_discovered_entries.clone();
            let pending_clone = pending.clone();
            let rq = retry_q.clone();

            workers.spawn(async move {
                // Phase 126: Per-worker HS health tracking
                use crate::circuit_health::CircuitHealth;
                let health = CircuitHealth::new();
                let mut consecutive_all_dead: u32 = 0;
                let mut request_count: u32 = 0;

                loop {
                    if f.is_cancelled() {
                        return;
                    }

                    // Work-stealing: primary queue first, then retry queue
                    let (next_url, retry_attempt) = if let Some(url) = q_clone.pop() {
                        (url, 0u8)
                    } else if let Some((url, attempt)) =
                        crate::work_stealing::try_pop_retry(&rq)
                    {
                        (url, attempt)
                    } else {
                        if pending_clone.load(std::sync::atomic::Ordering::SeqCst) == 0
                            && rq.is_empty()
                        {
                            break;
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                        continue;
                    };

                    let is_primary = retry_attempt == 0;

                    // Phase 126: CUSUM graduated backoff
                    request_count += 1;
                    if request_count > 2 && health.cusum_triggered() {
                        consecutive_all_dead += 1;
                        let backoff_secs = 2u64.pow(consecutive_all_dead.min(4));
                        println!(
                            "[Abyss W{} BACKOFF] CUSUM triggered — sleeping {}s (cycle {})",
                            worker_idx, backoff_secs, consecutive_all_dead
                        );
                        tokio::time::sleep(std::time::Duration::from_secs(backoff_secs)).await;
                        health.reset_cusum();
                    }

                    let _permit = f.politeness_semaphore.acquire().await.ok();
                    let (cid, client) = f.get_client();
                    let start_time = std::time::Instant::now();

                    // Phase 126: Adaptive timeout from EWMA latency
                    let timeout_ms = if request_count <= 2 { 20_000u64 } else { health.adaptive_ttfb_ms().min(20_000) };
                    let req = client.get(&next_url).send();
                    match tokio::time::timeout(std::time::Duration::from_millis(timeout_ms), req).await {
                        Ok(Ok(resp)) => {
                            let status = resp.status().as_u16();

                            if status == 404 {
                                f.record_failure(cid);
                                health.record_failure();
                                if is_primary {
                                    pending_clone
                                        .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                                }
                                continue;
                            }

                            if !resp.status().is_success() {
                                f.record_failure(cid);
                                health.record_failure();
                                let action = crate::work_stealing::classify_http_status(status);
                                crate::work_stealing::handle_failure(
                                    &rq,
                                    next_url,
                                    retry_attempt,
                                    action,
                                );
                                if is_primary {
                                    pending_clone
                                        .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                                }
                                continue;
                            }

                            // Success path
                            if let Ok(body_bytes) = resp.bytes().await {
                                let body = String::from_utf8_lossy(&body_bytes).into_owned();
                                let bytes_downloaded = body.len() as u64;
                                let elapsed_ms = start_time.elapsed().as_millis() as u64;
                                f.record_success(
                                    cid,
                                    bytes_downloaded,
                                    elapsed_ms,
                                );
                                health.record_success();
                                health.record_latency(elapsed_ms as f32);
                                consecutive_all_dead = 0;

                                if f.active_options.listing {
                                    let base_url_clone = next_url.clone();
                                    let parsed_entries =
                                        tokio::task::spawn_blocking(move || {
                                            parse_abyss_listing(&body, &base_url_clone)
                                        })
                                        .await
                                        .unwrap_or_default();

                                    let mut new_files = Vec::new();
                                    for entry in parsed_entries {
                                        if entry.entry_type == EntryType::Folder {
                                            if f.mark_visited(&entry.raw_url) {
                                                pending_clone.fetch_add(
                                                    1,
                                                    std::sync::atomic::Ordering::SeqCst,
                                                );
                                                q_clone.push(entry.raw_url.clone());
                                            }
                                        }
                                        new_files.push(entry);
                                    }

                                    for file in &new_files {
                                        let _ = ui_tx_clone.send(file.clone()).await;
                                    }

                                    if !new_files.is_empty() {
                                        for nf in new_files {
                                            discovered_ref.push(nf);
                                        }
                                    }
                                }
                            } else {
                                f.record_failure(cid);
                                health.record_failure();
                                crate::work_stealing::requeue_with_backoff(
                                    &rq, next_url, retry_attempt, 3,
                                );
                            }
                        }
                        _ => {
                            f.record_failure(cid);
                            health.record_failure();
                            crate::work_stealing::requeue_with_backoff(
                                &rq, next_url, retry_attempt, 3,
                            );
                        }
                    }

                    if is_primary {
                        pending_clone.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                    }
                }
            });
        }

        while workers.join_next().await.is_some() {}

        drop(ui_tx);
        let final_results = all_discovered_entries.drain_all();
        Ok(final_results)
    }

    fn name(&self) -> &'static str {
        "Abyss Ransomware"
    }

    fn known_domains(&self) -> Vec<&'static str> {
        vec!["vmmefm7ktazj2bwtmy46o3wxhk42tctasyyqv6ymuzlivszteyhkkyad.onion"]
    }

    fn regex_marker(&self) -> Option<&'static str> {
        Some(r"(?i)abyss")
    }
}
