use crate::adapters::{CrawlerAdapter, EntryType, FileEntry, SiteFingerprint};
use crate::frontier::CrawlerFrontier;
use crate::path_utils;
use std::sync::Arc;
use tauri::AppHandle;
use tokio::sync::mpsc;

#[derive(Default)]
pub struct AutoindexAdapter;

/// Parse an Apache/nginx "Index of /" HTML autoindex page.
/// Extracts all <a href="..."> links and optionally file sizes from the listing.
/// This is the shared parser used by both AutoindexAdapter and PlayAdapter.
pub fn parse_autoindex_html(html: &str) -> Vec<(String, Option<u64>, bool)> {
    let mut results = Vec::new();

    for line in html.lines() {
        if let Some(href_start) = line.find("href=\"") {
            let after_href = &line[href_start + 6..];
            if let Some(href_end) = after_href.find('"') {
                let raw_href = &after_href[..href_end];

                // Skip parent directory link
                if raw_href == "../"
                    || raw_href == ".."
                    || raw_href == "/"
                    || raw_href.starts_with("?")
                {
                    continue;
                }

                // URL-decode the href to get the real filename
                let decoded_name = path_utils::url_decode(raw_href);
                let is_dir = raw_href.ends_with('/');
                let clean_name = decoded_name.trim_end_matches('/').to_string();

                if clean_name.is_empty() {
                    continue;
                }

                // Try to extract size from the line text after the closing </a>
                let size = extract_size_from_line(line);

                results.push((clean_name, size, is_dir));
            }
        }
    }

    results
}

/// Extract file size from an autoindex line.
fn extract_size_from_line(line: &str) -> Option<u64> {
    if let Some(after_tag) = line.split("</a>").nth(1) {
        let tokens: Vec<&str> = after_tag.split_whitespace().collect();
        if let Some(last) = tokens.last() {
            let last_upper = last.trim().to_uppercase();

            if let Ok(size) = last_upper.parse::<u64>() {
                return Some(size);
            }

            // Handle human readable K, M, G representations
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
            } else if num_str.ends_with('T') {
                num_str.pop();
                multiplier = 1024 * 1024 * 1024 * 1024;
            }

            if let Ok(num) = num_str.parse::<f64>() {
                return Some((num * multiplier as f64) as u64);
            }
        }
    }
    None
}

#[async_trait::async_trait]
impl CrawlerAdapter for AutoindexAdapter {
    async fn can_handle(&self, fingerprint: &SiteFingerprint) -> bool {
        // Generic index fallback — matches any Apache/nginx autoindex page
        fingerprint.body.contains("Index of /")
    }

    async fn crawl(
        &self,
        current_url: &str,
        frontier: Arc<CrawlerFrontier>,
        app: AppHandle,
    ) -> anyhow::Result<Vec<FileEntry>> {
        use tauri::Emitter;

        let queue = Arc::new(crossbeam_queue::SegQueue::new());
        let all_discovered_entries = Arc::new(tokio::sync::Mutex::new(Vec::new()));

        queue.push(current_url.to_string());
        frontier.mark_visited(current_url);

        let pending = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        pending.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        // Batched UI Backpressure Task
        let (ui_tx, mut ui_rx) = mpsc::channel::<FileEntry>(500000);
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

        let max_concurrent = 120; // Massive worker-stealer parallel pool
        let mut workers = tokio::task::JoinSet::new();

        let _base_url = current_url.trim_end_matches('/').to_string();

        for _ in 0..max_concurrent {
            let f = frontier.clone();
            let q_clone = queue.clone();
            let ui_tx_clone = ui_tx.clone();
            let discovered_ref = all_discovered_entries.clone();
            let pending_clone = pending.clone();

            workers.spawn(async move {
                loop {
                    // Check cancellation before doing any work
                    if f.is_cancelled() {
                        return;
                    }

                    let next_url = match q_clone.pop() {
                        Some(url) => url,
                        None => {
                            if pending_clone.load(std::sync::atomic::Ordering::SeqCst) == 0 {
                                break;
                            }
                            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                            continue;
                        }
                    };

                    let _permit = f.politeness_semaphore.acquire().await.ok();
                    let (cid, client) = f.get_client();

                    // Enforce predictive yield delay from CircuitScorer
                    let delay = f.scorer.yield_delay(cid);
                    if delay > std::time::Duration::ZERO {
                        tokio::time::sleep(delay).await;
                    }

                    let start_time = std::time::Instant::now();
                    let mut bytes_downloaded = 0;

                    // Fetch the HTML page
                    let (mut fetch_success, mut html) = (false, None);
                    if let Ok(Ok(resp)) = tokio::time::timeout(
                        std::time::Duration::from_secs(45),
                        client.get(&next_url).send(),
                    )
                    .await
                    {
                        if resp.status().is_success() {
                            if let Ok(body) = resp.text().await {
                                bytes_downloaded += body.len() as u64;
                                fetch_success = true;
                                html = Some(body);
                            }
                        } else if resp.status() == 404 {
                            f.record_failure(cid);
                            pending_clone.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                            continue;
                        }
                    }

                    // Report to AIMD and CircuitScorer
                    let elapsed_ms = start_time.elapsed().as_millis() as u64;
                    if fetch_success {
                        f.record_success(cid, bytes_downloaded, elapsed_ms);
                    } else {
                        f.record_failure(cid);
                        pending_clone.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                        continue; // Move to next URL without aborting worker
                    }

                    let Some(html) = html else {
                        pending_clone.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                        continue;
                    };

                    if !f.active_options.listing {
                        pending_clone.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                        continue;
                    }

                    // Parse all entries from the autoindex page
                    let parsed = parse_autoindex_html(&html);
                    let mut new_files = Vec::new();

                    for (filename, parsed_size, is_dir) in parsed {
                        let encoded = path_utils::url_encode(&filename);
                        let child_url = format!("{}/{}", next_url.trim_end_matches('/'), encoded);

                        if is_dir {
                            // Emit folder entry
                            let sanitized_path =
                                format!("/{}", path_utils::sanitize_path(&filename));
                            new_files.push(FileEntry {
                                path: sanitized_path.clone(),
                                size_bytes: None,
                                entry_type: EntryType::Folder,
                                raw_url: format!("{}/", child_url),
                            });

                            // Enqueue subdirectory for recursive crawling
                            let sub_url = format!("{}/", child_url);
                            if f.mark_visited(&sub_url) {
                                pending_clone.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                                q_clone.push(sub_url);
                            }
                        } else {
                            // File entry
                            let size = if f.active_options.sizes {
                                if let Some(s) = parsed_size {
                                    Some(s)
                                } else {
                                    // Try HEAD request for Content-Length
                                    match client.head(&child_url).send().await {
                                        Ok(head_resp) => head_resp
                                            .headers()
                                            .get("content-length")
                                            .and_then(|v| v.to_str().ok())
                                            .and_then(|s| s.parse::<u64>().ok()),
                                        Err(_) => None,
                                    }
                                }
                            } else {
                                None
                            };

                            let sanitized_path =
                                format!("/{}", path_utils::sanitize_path(&filename));
                            new_files.push(FileEntry {
                                path: sanitized_path,
                                size_bytes: size,
                                entry_type: EntryType::File,
                                raw_url: child_url,
                            });
                        }
                    }

                    // Flush to IPC batcher
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
        "Generic Autoindex"
    }
}
