use tauri::AppHandle;
use std::sync::Arc;
use tokio::sync::mpsc;
use crate::adapters::{CrawlerAdapter, SiteFingerprint, FileEntry, EntryType};
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

        queue.push(current_url.to_string());
        frontier.mark_visited(current_url);

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

        let max_concurrent = 120;
        let mut workers = tokio::task::JoinSet::new();

        let parsed_url = reqwest::Url::parse(current_url)?;
        let base_domain = format!("{}://{}", parsed_url.scheme(), parsed_url.host_str().unwrap_or(""));

        for _ in 0..max_concurrent {
            let f = frontier.clone();
            let q_clone = queue.clone();
            let ui_tx_clone = ui_tx.clone();
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

                    // 5-pass Retry Pattern for Tor
                    let (mut fetch_success, mut html) = (false, None);

                    for attempt in 1..=5 {
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
                            } else if status.is_server_error() {
                                // Let it retry via the loop
                            }
                        } else if let Ok(Err(_e)) = &resp_result {
                            f.record_failure(cid);
                        } else {
                            f.record_failure(cid);
                        }
                        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                    }

                    if !fetch_success {
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
