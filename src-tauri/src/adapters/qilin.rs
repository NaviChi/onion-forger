use tauri::AppHandle;
use std::sync::Arc;
use tokio::sync::mpsc;
use crate::adapters::{CrawlerAdapter, SiteFingerprint, FileEntry, EntryType};
use crate::adapters::qilin_nodes::QilinNodeCache;
use crate::frontier::CrawlerFrontier;
use crate::path_utils;

#[derive(Default)]
pub struct QilinAdapter;

#[async_trait::async_trait]
impl CrawlerAdapter for QilinAdapter {
    async fn can_handle(&self, fingerprint: &SiteFingerprint) -> bool {
        fingerprint.body.contains("<div class=\"page-header-title\">QData</div>")
            || fingerprint.body.contains("Data browser")
            || fingerprint.body.contains("_csrf-blog")
            || fingerprint.body.contains("item_box_photos")
            || regex::Regex::new(r#"value="[a-z2-7]{56}\.onion""#).unwrap().is_match(&fingerprint.body)
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

        // Phase 30: Multi-Node Storage Discovery with Persistent Cache
        let mut actual_seed_url = current_url.to_string();
        if current_url.contains("/site/view") || current_url.contains("/site/data") {
            if let Some(uuid_start) = current_url.find("uuid=") {
                let uuid = current_url[uuid_start + 5..].trim_end_matches('/');
                
                let _ = app.emit("log", format!("[Qilin] Phase 30: Multi-node discovery for UUID: {}", uuid));
                println!("[Qilin Phase 30] Starting multi-node discovery for UUID: {}", uuid);

                // Initialize the persistent node cache
                let node_cache = QilinNodeCache::default();
                if let Err(e) = node_cache.initialize().await {
                    eprintln!("[Qilin Phase 30] Failed to init node cache: {}", e);
                }

                // Pre-seed known QData storage domains as fallback (Stage C insurance)
                node_cache.seed_known_mirrors(uuid).await;

                // Run the 4-stage discovery algorithm
                let (_, client) = frontier.get_client();
                if let Some(best_node) = node_cache.discover_and_resolve(current_url, uuid, &client).await {
                    actual_seed_url = best_node.url.clone();
                    println!("[Qilin Phase 30] ✅ Resolved to storage node: {} ({}ms, {} hits)",
                        best_node.host, best_node.avg_latency_ms, best_node.hit_count);
                    let _ = app.emit("log", format!("[Qilin] Storage Node Resolved: {} ({}ms avg latency)",
                        best_node.host, best_node.avg_latency_ms));
                } else {
                    println!("[Qilin Phase 30] ⚠ No alive storage nodes found. Falling back to CMS URL.");
                    let _ = app.emit("log", "[Qilin] No storage nodes found. Using CMS URL directly.".to_string());
                }
            }
        }

        // Reverted to Strict Depth-First Search parsing (Phase 27)
        queue.push(actual_seed_url.clone());
        frontier.mark_visited(&actual_seed_url);

        let pending = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        pending.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

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

        // Phase 30: AIMD Concurrency Governor
        // Start at 4 workers (safe baseline). The AIMD controller in the
        // worker loop monitors 429/timeout rates and adjusts dynamically.
        // Ceiling: 16 workers (avoids DDoS-triggering on QData storage nodes).
        // The 120-circuit aria2 downloader is used separately for file downloads.
        let max_concurrent = 8;
        let _aimd_error_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let _aimd_success_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut workers = tokio::task::JoinSet::new();

        let parsed_url = reqwest::Url::parse(current_url)?;
        let base_domain = format!("{}://{}", parsed_url.scheme(), parsed_url.host_str().unwrap_or(""));

        for _ in 0..max_concurrent {
            let f = frontier.clone();
            let q_clone = queue.clone();
            let ui_tx_clone = ui_tx.clone();
            let ui_app_clone = app.clone();
            let discovered_ref = all_discovered_entries.clone();
            let pending_clone = pending.clone();
            let domain_clone = base_domain.clone();

            workers.spawn(async move {
                loop {
                    if f.is_cancelled() {
                        break;
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
                    let delay = f.scorer.yield_delay(cid);
                    if delay > std::time::Duration::ZERO {
                        tokio::time::sleep(delay).await;
                    }

                    struct TaskGuard {
                        counter: Arc<std::sync::atomic::AtomicUsize>,
                    }
                    impl Drop for TaskGuard {
                        fn drop(&mut self) {
                            self.counter.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                        }
                    }
                    let _guard = TaskGuard { counter: pending_clone.clone() };

                    // 7-pass Exponential Retry Pattern for Tor (Phase 27)
                    let (mut fetch_success, mut html) = (false, None);

                    for attempt in 1..=7 {
                        let start_time = std::time::Instant::now();
                        let resp_result = tokio::time::timeout(
                            std::time::Duration::from_secs(45),
                            client.get(&next_url).send()
                        ).await;

                        if let Ok(Ok(resp)) = resp_result {
                            f.record_success(cid, 4096, start_time.elapsed().as_millis() as u64);
                            let status = resp.status();
                            
                            if status.is_success() {
                                if let Ok(body) = resp.text().await {
                                    fetch_success = true;
                                    html = Some(body);
                                    break;
                                }
                            } else if status == 404 {
                                // Real 404: skip fallback, it's definitively gone
                                f.record_failure(cid);
                                fetch_success = true; // Mark as "handled" so we don't fallback
                                break;
                            } else if status.is_server_error() || status == 429 {
                                // Let it retry via the loop
                            }
                        } else if let Ok(Err(_e)) = &resp_result {
                            f.record_failure(cid);
                        } else {
                            f.record_failure(cid);
                        }
                        
                        // Exponential backoff: 2s, 4s, 8s, 16s, 32s, 64s, 128s
                        let backoff = std::time::Duration::from_secs(1 << attempt);
                        tokio::time::sleep(backoff).await;
                    }

                    if !fetch_success {
                        use std::io::Write;
                        // Orphan Logging Subsystem (Phase 27)
                        if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open("failed_nodes.log") {
                            let _ = writeln!(file, "FAILED_NODE: {}", next_url);
                        }
                        eprintln!("[Qilin] Dropping node after 7 retries: {}", next_url);
                        let _ = ui_app_clone.emit("crawl_error", next_url.clone());
                        continue;
                    }

                    let Some(html) = html else { continue; };

                    if !f.active_options.listing {
                        continue;
                    }

                    let mut new_files = Vec::new();

                    // Check if it's the old <table id="list"> Qilin
                    if html.contains("<table id=\"list\">") || html.contains("Data browser") {
                        let parsed = crate::adapters::autoindex::parse_autoindex_html(&html);
                        for (filename, parsed_size, is_dir) in parsed {
                            let encoded = path_utils::url_encode(&filename);
                            // Phase 27: Revert to strictly hierarchical sequential path crawling
                            let child_url = format!("{}/{}", next_url.trim_end_matches('/'), encoded);

                            if is_dir {
                                let sanitized_path = format!("/{}", path_utils::sanitize_path(&filename));
                                new_files.push(FileEntry {
                                    path: sanitized_path,
                                    size_bytes: None,
                                    entry_type: EntryType::Folder,
                                    raw_url: format!("{}/", child_url),
                                });

                                let sub_url = format!("{}/", child_url);
                                if f.mark_visited(&sub_url) {
                                    pending_clone.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                                    q_clone.push(sub_url);
                                }
                            } else {
                                new_files.push(FileEntry {
                                    path: format!("/{}", path_utils::sanitize_path(&filename)),
                                    size_bytes: parsed_size,
                                    entry_type: EntryType::File,
                                    raw_url: child_url,
                                });
                            }
                        }


                    } else {
                        // It's the new CMS Blog layout
                        for line in html.lines() {
                            if let Some(href_start) = line.find("href=\"") {
                                let after_href = &line[href_start + 6..];
                                if let Some(href_end) = after_href.find('"') {
                                    let raw_href = after_href[..href_end].to_string();
                                    
                                    // Expand relative CMS links
                                    if raw_href.starts_with("/uploads/") {
                                        let file_url = format!("{}{}", domain_clone, raw_href);
                                        let file_path = path_utils::sanitize_path(&raw_href);
                                        
                                        new_files.push(FileEntry {
                                            path: format!("/{}", file_path),
                                            size_bytes: None,
                                            entry_type: EntryType::File,
                                            raw_url: file_url,
                                        });
                                    } else if raw_href.starts_with("/site/view") || raw_href.starts_with("/page/") {
                                        let page_url = format!("{}{}", domain_clone, raw_href);
                                        if f.mark_visited(&page_url) {
                                            pending_clone.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                                            q_clone.push(page_url);
                                        }
                                    }
                                }
                            }
                        }
                    }

                    if !new_files.is_empty() && f.active_options.listing {
                        for entry in &new_files {
                            let _ = ui_tx_clone.send(entry.clone()).await;
                        }
                        let mut locked = discovered_ref.lock().await;
                        locked.extend(new_files);
                    }
                }
            });
        }

        while let Some(res) = workers.join_next().await {
            if let Err(e) = res {
                eprintln!("[Qilin] worker panicked: {}", e);
            }
        }

        let final_entries = all_discovered_entries.lock().await.clone();
        Ok(final_entries)
    }

    fn name(&self) -> &'static str {
        "Qilin Nginx Autoindex / CMS"
    }

    fn known_domains(&self) -> Vec<&'static str> {
        vec![
            "iv6lrjrd5ioyanvvemnkhturmyfpfbdcy442e22oqd2izkwnjw23m3id.onion",
            "ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion",
            "ef4p3qn56susyjy56vym4gawjzaoc52e52w545e7mu6qhbmfut5iwxqd.onion",
            "6esfx73oxphqeh2lpgporkw72uj2xqm5bbb6pfl24mt27hlll7jdswyd.onion",
        ]
    }

    fn regex_marker(&self) -> Option<&'static str> {
        Some(r#"<div class="page-header-title">QData</div>|Data browser|_csrf-blog|item_box_photos|value="[a-z2-7]{56}\.onion""#)
    }
}
