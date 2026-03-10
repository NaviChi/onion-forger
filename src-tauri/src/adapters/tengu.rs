use crate::adapters::{CrawlerAdapter, EntryType, FileEntry, SiteFingerprint};
use crate::frontier::CrawlerFrontier;
use crate::path_utils;
use std::sync::Arc;
use tauri::AppHandle;
use tokio::sync::mpsc;

/// Tengu Ransomware Adapter
///
/// New target provided by user: `http://longvqprqrb4zbxooswz4upefhtikhnyqv4gw4fkzpkc2wjpvxsucwid.onion/v/aa45a1540201f248b27801bb98b52d6e`
#[derive(Default)]
pub struct TenguAdapter;

/// Parse Tengu HTML listing pages.
/// Tengu uses a Bootstrap `table table-dark table-hover` structure with `a` tags inside `td`.
fn parse_tengu_listing(html: &str, _base_url: &str) -> Vec<FileEntry> {
    let mut entries = Vec::new();

    if let Ok(document) = std::panic::catch_unwind(|| scraper::Html::parse_document(html)) {
        if let Ok(row_selector) = scraper::Selector::parse("table.table tbody tr") {
            if let Ok(link_selector) = scraper::Selector::parse("td a") {
                if let Ok(size_selector) = scraper::Selector::parse("td:nth-child(2)") {
                    for row in document.select(&row_selector) {
                        let link = row.select(&link_selector).next();
                        let size_node = row.select(&size_selector).next();

                        if let Some(link) = link {
                            let href = link.value().attr("href").unwrap_or("");
                            if href.is_empty()
                                || href == "../"
                                || href == "/"
                                || href.starts_with("?")
                                || href.starts_with("javascript:")
                                || href.contains("action=open")
                            {
                                continue;
                            }

                            // The URL in the dump contains the exact absolute link to the file/folder
                            // Extract just the basename for the path metric
                            let clean_name = href.split('/').last().unwrap_or("").to_string();
                            let decoded_name = path_utils::url_decode(&clean_name);

                            if decoded_name.is_empty() {
                                continue;
                            }

                            // Determine if folder by looking at the icon column, but if it has no icon check trailing slashes
                            let is_dir = row.html().contains("fa-folder");

                            // Size extraction (e.g. "2.11 GB")
                            let mut size_bytes = None;
                            if !is_dir {
                                if let Some(sn) = size_node {
                                    let size_str = sn
                                        .text()
                                        .collect::<Vec<_>>()
                                        .join("")
                                        .trim()
                                        .to_uppercase();
                                    size_bytes = extract_size_from_str(&size_str);
                                }
                            }

                            entries.push(FileEntry {
                                jwt_exp: None,
                                path: format!("/{}", path_utils::sanitize_path(&decoded_name)),
                                size_bytes,
                                entry_type: if is_dir {
                                    EntryType::Folder
                                } else {
                                    EntryType::File
                                },
                                raw_url: href.to_string(),
                            });
                        }
                    }
                }
            }
        }
    }

    entries
}

fn extract_size_from_str(size_str: &str) -> Option<u64> {
    let tokens: Vec<&str> = size_str.split_whitespace().collect();
    if let (Some(&num), Some(&unit)) = (tokens.first(), tokens.last()) {
        if let Ok(value) = num.parse::<f64>() {
            let multiplier: u64 = match unit {
                "KB" => 1024,
                "MB" => 1024 * 1024,
                "GB" => 1024 * 1024 * 1024,
                "TB" => 1024 * 1024 * 1024 * 1024,
                "B" => 1,
                _ => 1,
            };
            return Some((value * multiplier as f64) as u64);
        }
    }
    None
}

#[async_trait::async_trait]
impl CrawlerAdapter for TenguAdapter {
    async fn can_handle(&self, fingerprint: &SiteFingerprint) -> bool {
        let url_lower = fingerprint.url.to_ascii_lowercase();
        url_lower.contains("longvqprqrb4zbxooswz4upefhtikhnyqv4gw4fkzpkc2wjpvxsucwid")
            || url_lower.contains("tengu")
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

                    for attempt in 0..4 {
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
                            if let Some(delay) =
                                ddos_guard.record_response_legacy(resp.status().as_u16())
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

                    let base_url_clone = next_url.clone();
                    let (spawned_entries,) = tokio::task::spawn_blocking(move || {
                        (parse_tengu_listing(&html, &base_url_clone),)
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
        "Tengu Ransomware"
    }

    fn known_domains(&self) -> Vec<&'static str> {
        vec!["longvqprqrb4zbxooswz4upefhtikhnyqv4gw4fkzpkc2wjpvxsucwid.onion"]
    }

    fn regex_marker(&self) -> Option<&'static str> {
        Some(r"(?i)tengu")
    }
}
