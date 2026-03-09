use crate::adapters::{CrawlerAdapter, EntryType, FileEntry, SiteFingerprint};
use crate::frontier::CrawlerFrontier;
use crate::path_utils;
use std::sync::Arc;
use tauri::AppHandle;
use tokio::sync::mpsc;

/// AlphaLocker Ransomware Adapter
///
/// AlphaLocker typically exposes a file listing interface. The URL pattern often
/// includes company domains as path segments with a /Files/ suffix.
/// Example: `http://<onion>/gazomet.pl%20&%20cgas.pl/Files/`
///
/// The listing is commonly nginx autoindex-style, but may include custom
/// HTML markup. This adapter handles:
/// 1. URL-encoded path segments (e.g., `%20&%20`)
/// 2. Standard autoindex HTML parsing
/// 3. Custom AlphaLocker listing markup
/// 4. Recursive directory traversal
#[derive(Default)]
pub struct AlphaLockerAdapter;

/// Parse AlphaLocker-style HTML listing pages.
/// AlphaLocker may use autoindex-like layout or a custom table-based listing.
fn parse_alphalocker_listing(html: &str, base_url: &str) -> Vec<FileEntry> {
    let mut entries = Vec::new();

    // Strategy 1: Try standard autoindex-style parsing
    for line in html.lines() {
        if let Some(href_start) = line.find("href=\"") {
            let after_href = &line[href_start + 6..];
            if let Some(href_end) = after_href.find('"') {
                let raw_href = &after_href[..href_end];

                // Skip navigation/irrelevant links
                if raw_href == "../"
                    || raw_href == ".."
                    || raw_href == "/"
                    || raw_href.starts_with("?")
                    || raw_href.starts_with("javascript:")
                    || raw_href.starts_with("#")
                    || raw_href.starts_with("${")
                {
                    continue;
                }

                // Skip absolute external links unless they're to the same host
                if (raw_href.starts_with("http://") || raw_href.starts_with("https://"))
                    && !raw_href.contains(
                        &url::Url::parse(base_url)
                            .ok()
                            .and_then(|u| u.host_str().map(|s| s.to_string()))
                            .unwrap_or_default(),
                    )
                {
                    continue;
                }

                let decoded_name = path_utils::url_decode(raw_href);
                let is_dir = raw_href.ends_with('/');
                let clean_name = decoded_name.trim_end_matches('/').to_string();

                if clean_name.is_empty() {
                    continue;
                }

                // Build child URL
                let child_url =
                    if raw_href.starts_with("http://") || raw_href.starts_with("https://") {
                        raw_href.to_string()
                    } else {
                        let encoded = path_utils::url_encode(&clean_name);
                        let base = base_url.trim_end_matches('/');
                        if is_dir {
                            format!("{}/{}/", base, encoded)
                        } else {
                            format!("{}/{}", base, encoded)
                        }
                    };

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

    // Strategy 2: If no results from autoindex parsing, try table-based extraction
    if entries.is_empty() {
        // Try scraper-based parsing for custom table layouts
        if let Ok(document) = std::panic::catch_unwind(|| scraper::Html::parse_document(html)) {
            if let Ok(link_selector) = scraper::Selector::parse("a[href]") {
                for link in document.select(&link_selector) {
                    let href = link.value().attr("href").unwrap_or("");
                    if href.is_empty()
                        || href == "../"
                        || href == "/"
                        || href.starts_with("?")
                        || href.starts_with("javascript:")
                    {
                        continue;
                    }

                    let link_text = link.text().collect::<Vec<_>>().join("").trim().to_string();
                    if link_text.is_empty() || link_text == ".." || link_text == "Parent Directory"
                    {
                        continue;
                    }

                    let is_dir = href.ends_with('/');
                    let clean_name = link_text.trim_end_matches('/').to_string();

                    let child_url = if href.starts_with("http://") || href.starts_with("https://") {
                        href.to_string()
                    } else {
                        let encoded = path_utils::url_encode(&clean_name);
                        let base = base_url.trim_end_matches('/');
                        if is_dir {
                            format!("{}/{}/", base, encoded)
                        } else {
                            format!("{}/{}", base, encoded)
                        }
                    };

                    entries.push(FileEntry {
                        jwt_exp: None,
                        path: format!("/{}", path_utils::sanitize_path(&clean_name)),
                        size_bytes: None,
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
    }

    entries
}

/// Extract file size from an HTML line
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
impl CrawlerAdapter for AlphaLockerAdapter {
    async fn can_handle(&self, fingerprint: &SiteFingerprint) -> bool {
        let url_lower = fingerprint.url.to_ascii_lowercase();
        let body_lower = fingerprint.body.to_ascii_lowercase();

        url_lower.contains("alphalocker")
            || url_lower.contains("3v4zoso2ghne47usnhyoe4dsezmfqhfv5v5iuep4saic5nnfpc6phrad")
            || body_lower.contains("alphalocker")
            || body_lower.contains("alpha locker")
            // AlphaLocker may have autoindex with custom branding
            || (url_lower.contains("/files/")
                && url_lower.contains("3v4zoso2ghne47usnhyoe4dsezmfqhfv5v5iuep4saic5nnfpc6phrad"))
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

                    // Retry loop with circuit rotation
                    for attempt in 0..4 {
                        let (retry_cid1, retry_client1) = if attempt == 0 {
                            (cid, client.clone())
                        } else {
                            f.get_client()
                        };
                        let (retry_cid2, retry_client2) = f.get_client();
                        
                        let next_url_clone1 = next_url.clone();
                        let next_url_clone2 = next_url.clone();
                        
                        let req1 = Box::pin(async move {
                            let res = tokio::time::timeout(
                                std::time::Duration::from_secs(45),
                                retry_client1.get(&next_url_clone1).send(),
                            ).await;
                            (retry_cid1, res)
                        });
                        
                        let req2 = Box::pin(async move {
                            let res = tokio::time::timeout(
                                std::time::Duration::from_secs(45),
                                retry_client2.get(&next_url_clone2).send(),
                            ).await;
                            (retry_cid2, res)
                        });
                        
                        let (winner_cid, fetch_result) = match futures::future::select(req1, req2).await {
                            futures::future::Either::Left((res, _)) => res,
                            futures::future::Either::Right((res, _)) => res,
                        };

                        if let Ok(Ok(resp)) = fetch_result {
                            if let Some(delay) = ddos_guard.record_response(resp.status().as_u16())
                            {
                                tokio::time::sleep(delay).await;
                            }
                            if resp.status().is_success() {
                                if let Ok(Ok(body)) = tokio::time::timeout(
                                    std::time::Duration::from_secs(45),
                                    resp.text(),
                                )
                                .await
                                {
                                    bytes_downloaded += body.len() as u64;
                                    html = Some(body);
                                    fetch_success = true;
                                    f.record_success(
                                        winner_cid,
                                        bytes_downloaded,
                                        start_time.elapsed().as_millis() as u64,
                                    );
                                    break;
                                }
                            } else if resp.status().as_u16() == 404 {
                                f.record_failure(winner_cid);
                                break;
                            }
                        }
                        f.record_failure(winner_cid);
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

                    // Parse off-thread Phase 73 HFT DOM Preheating
                    let base_url_clone = next_url.clone();
                    let html_clone = html.clone();
                    let spawned_entries = tokio::task::spawn_blocking(move || {
                        parse_alphalocker_listing(&html_clone, &base_url_clone)
                    })
                    .await
                    .unwrap_or_default();

                    let mut new_files = Vec::new();
                    for entry in spawned_entries {
                        if entry.entry_type == EntryType::Folder {
                            if f.mark_visited(&entry.raw_url) {
                                pending_clone.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                                q_clone.push(entry.raw_url.clone());
                            }
                        }
                        new_files.push(entry);
                    }

                    // Merge HEAD Size Probes into First GET (Kill Redundant Requests)
                    if f.active_options.sizes {
                        for nf in new_files.iter_mut() {
                            if nf.entry_type == EntryType::File && nf.size_bytes.is_none() {
                                let (hcid, hclient) = f.get_client();
                                if let Ok(Ok(size_resp)) = tokio::time::timeout(
                                    std::time::Duration::from_secs(10),
                                    hclient.get(&nf.raw_url)
                                        .header("Range", "bytes=0-0")
                                        .send(),
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
                                    f.record_success(hcid, 0, 0);
                                } else {
                                    f.record_failure(hcid);
                                }
                            }
                        }
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
        "AlphaLocker Ransomware"
    }

    fn known_domains(&self) -> Vec<&'static str> {
        vec!["3v4zoso2ghne47usnhyoe4dsezmfqhfv5v5iuep4saic5nnfpc6phrad.onion"]
    }

    fn regex_marker(&self) -> Option<&'static str> {
        Some(r"(?i)alpha\s*locker|alphalocker")
    }
}
