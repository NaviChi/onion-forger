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
    let mut cursor = 0;

    while let Some(pos) = html[cursor..].find("href=\"") {
        let href_start = cursor + pos + 6;
        cursor = href_start;

        let href_end = match html[cursor..].find('"') {
            Some(end_pos) => cursor + end_pos,
            None => break,
        };

        let raw_href = &html[href_start..href_end];
        cursor = href_end + 1;

        if raw_href == "../"
            || raw_href == ".."
            || raw_href == "/"
            || raw_href.starts_with('?')
            || raw_href.starts_with('/')
            || raw_href.starts_with("http://")
            || raw_href.starts_with("https://")
            || raw_href.starts_with("${")
        {
            continue;
        }

        let decoded_name = path_utils::url_decode(raw_href);
        let is_dir = raw_href.ends_with('/');
        let clean_name = decoded_name.trim_end_matches('/').to_string();

        if clean_name.is_empty() {
            continue;
        }

        // To extract size, we find the closing </a> and read the text after it up to the newline or the next tag.
        let mut size: Option<u64> = None;
        if let Some(tag_close) = html[cursor..].find("</a>") {
            let search_start = cursor + tag_close + 4;

            // Find the boundary to stop searching for size (either a newline or another tag)
            let newline_pos = html[search_start..]
                .find('\n')
                .unwrap_or(html.len() - search_start);
            let next_tag_pos = html[search_start..]
                .find('<')
                .unwrap_or(html.len() - search_start);
            let bound_pos = std::cmp::min(newline_pos, next_tag_pos);

            let text_after_tag = html[search_start..search_start + bound_pos].trim();
            if !text_after_tag.is_empty() {
                size = extract_size_from_line(text_after_tag);
            }
        }

        results.push((clean_name, size, is_dir));
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

        let queue = Arc::new(crate::spillover::SpilloverQueue::new());
        let all_discovered_entries = Arc::new(crate::spillover::SpilloverList::new());

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

        let max_concurrent = frontier.recommended_listing_workers();
        let mut workers = tokio::task::JoinSet::new();

        let _base_url = current_url.trim_end_matches('/').to_string();

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
                    // Check cancellation before doing any work
                    if f.is_cancelled() {
                        return;
                    }

                    let next_url = match q_clone.pop() {
                        Some(url) => {
                            println!("[DEBUG AUTOINDEX] Worker grabbed URL from queue: {}", url);
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
                    let (cid1, client1) = f.get_client();
                    let (cid2, client2) = f.get_client();

                    // Enforce predictive yield delay from CircuitScorer
                    let delay = f.scorer.yield_delay(cid1);
                    if delay > std::time::Duration::ZERO {
                        tokio::time::sleep(delay).await;
                    }

                    let start_time = std::time::Instant::now();
                    let mut bytes_downloaded = 0;

                    // Phase 73: Speculative Dual-Circuit Tor GET Racing
                    let req1 = Box::pin(async {
                        let res = tokio::time::timeout(
                            std::time::Duration::from_secs(45),
                            client1.get(&next_url).send(),
                        ).await;
                        (cid1, res)
                    });

                    let req2 = Box::pin(async {
                        let res = tokio::time::timeout(
                            std::time::Duration::from_secs(45),
                            client2.get(&next_url).send(),
                        ).await;
                        (cid2, res)
                    });

                    let (winner_cid, fetch_result) = match futures::future::select(req1, req2).await {
                        futures::future::Either::Left((res, _)) => res,
                        futures::future::Either::Right((res, _)) => res,
                    };

                    let (mut fetch_success, mut html) = (false, None);

                    match fetch_result {
                        Ok(Ok(resp)) => {
                            if let Some(delay) = ddos_guard.record_response_legacy(resp.status().as_u16()) {
                                tokio::time::sleep(delay).await;
                            }
                            if resp.status().is_success() {
                                if let Ok(body_bytes) = resp.bytes().await {
                                    let body = String::from_utf8_lossy(&body_bytes).into_owned();
                                    bytes_downloaded += body.len() as u64;
                                    fetch_success = true;
                                    if next_url.contains("lockbit") {
                                        println!("[DEBUG LOCKBIT] Body prefix: {}", &body.chars().take(4000).collect::<String>());
                                    }
                                    html = Some(body);
                                }
                            } else if resp.status() == 404 {
                                f.record_failure(winner_cid);
                                pending_clone.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                                continue;
                            } else {
                                if next_url.contains("lockbit") {
                                    println!("[DEBUG LOCKBIT] Fetch failed to {}, status: {}", next_url, resp.status());
                                }
                                f.record_failure(winner_cid);
                                pending_clone.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                                continue;
                            }
                        }
                        Ok(Err(e)) => {
                            if next_url.contains("lockbit") {
                                println!("[DEBUG LOCKBIT] Reqwest HTTP error establishing connection to {}: {}", next_url, e);
                            }
                        }
                        Err(e) => {
                            if next_url.contains("lockbit") {
                                println!("[DEBUG LOCKBIT] Tokio Timeout hitting endpoint {}: {}", next_url, e);
                            }
                        }
                    }

                    // Report to AIMD and CircuitScorer
                    let elapsed_ms = start_time.elapsed().as_millis() as u64;
                    if fetch_success {
                        f.record_success(winner_cid, bytes_downloaded, elapsed_ms);
                    } else {
                        f.record_failure(winner_cid);
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

                    // Parse all entries from the autoindex page off-thread
                    let (spawned_files, spawned_folders) = tokio::task::spawn_blocking({
                        let html = html.clone();
                        let next_url = next_url.clone();
                        move || {
                            let mut local_files = Vec::new();
                            let mut local_folders = Vec::new();
                            let parsed = parse_autoindex_html(&html);

                            for (filename, parsed_size, is_dir) in parsed {
                                let encoded = path_utils::url_encode(&filename);
                                let child_url =
                                    format!("{}/{}", next_url.trim_end_matches('/'), encoded);

                                if is_dir {
                                    let sanitized_path =
                                        format!("/{}", path_utils::sanitize_path(&filename));
                                    local_files.push(FileEntry { jwt_exp: None,
                                        path: sanitized_path,
                                        size_bytes: None,
                                        entry_type: EntryType::Folder,
                                        raw_url: format!("{}/", child_url),
                                    });
                                    local_folders.push(format!("{}/", child_url));
                                } else {
                                    let sanitized_path =
                                        format!("/{}", path_utils::sanitize_path(&filename));
                                    local_files.push(FileEntry { jwt_exp: None,
                                        path: sanitized_path,
                                        size_bytes: parsed_size,
                                        entry_type: EntryType::File,
                                        raw_url: child_url,
                                    });
                                }
                            }
                            (local_files, local_folders)
                        }
                    })
                    .await
                    .unwrap_or_default();

                    let mut new_files = spawned_files;
                    for sub_url in spawned_folders {
                        if f.mark_visited(&sub_url) {
                            pending_clone.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                            q_clone.push(sub_url);
                        }
                    }

                    // Async GET Range requests for Content-Length (merges HEAD redundancy)
                    if f.active_options.sizes {
                        for nf in new_files.iter_mut() {
                            if nf.entry_type == EntryType::File && nf.size_bytes.is_none() {
                                if let Ok(Ok(size_resp)) = tokio::time::timeout(
                                    std::time::Duration::from_secs(10),
                                    client1.get(&nf.raw_url).header("Range", "bytes=0-0").send(),
                                )
                                .await
                                {
                                    nf.size_bytes = size_resp
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
                                }
                            }
                        }
                    }

                    // Flush to IPC batcher
                    for file in &new_files {
                        let _ = ui_tx_clone.send(file.clone()).await;
                    }

                    if !new_files.is_empty() {
                        for nf in new_files {
                            discovered_ref.push(nf);
                        }
                    }

                    pending_clone.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                }
            });
        }

        while workers.join_next().await.is_some() {}

        drop(ui_tx);
        let final_results = all_discovered_entries.drain_all();
        Ok(final_results)
    }

    fn name(&self) -> &'static str {
        "Generic Autoindex"
    }
}
