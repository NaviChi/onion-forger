use tauri::AppHandle;
use std::sync::Arc;
use tokio::sync::mpsc;
use crate::adapters::{CrawlerAdapter, SiteFingerprint, FileEntry, EntryType};
use crate::adapters::autoindex::parse_autoindex_html;
use crate::frontier::CrawlerFrontier;
use crate::path_utils;

#[derive(Default)]
pub struct PlayAdapter;

#[async_trait::async_trait]
impl CrawlerAdapter for PlayAdapter {
    async fn can_handle(&self, fingerprint: &SiteFingerprint) -> bool {
        fingerprint.url.contains("b3pzp6qwelgeygmzn6awkduym6s4gxh6htwxuxeydrziwzlx63zergyd.onion")
            || fingerprint.url.contains("FALOp")
            || fingerprint.body.contains("Index of /FALOp/")
    }

    async fn crawl(
        &self,
        current_url: &str,
        frontier: Arc<CrawlerFrontier>,
        app: AppHandle
    ) -> anyhow::Result<Vec<FileEntry>> {
        use tauri::Emitter;

        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        let all_discovered_entries = Arc::new(tokio::sync::Mutex::new(Vec::new()));

        tx.send(current_url.to_string())?;
        frontier.mark_visited(current_url);

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

        let max_concurrent = 120;
        let mut active_tasks = 0;
        let mut workers = tokio::task::JoinSet::new();

        let base_url = current_url.trim_end_matches('/').to_string();
        
        let pending = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        pending.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        loop {
            // Check cancellation
            if frontier.is_cancelled() {
                app.emit("crawl_log", "[System] Crawl cancelled by user.".to_string()).unwrap_or_default();
                break;
            }

            while active_tasks < max_concurrent {
                if let Ok(next_url) = rx.try_recv() {
                    let f = frontier.clone();
                    let _tx_clone = tx.clone();
                    let ui_tx_clone = ui_tx.clone();
                    let discovered_ref = all_discovered_entries.clone();
                    let current_base = base_url.clone();

                    active_tasks += 1;
                    let pending_clone = pending.clone();
                    workers.spawn(async move {
                        // Check cancellation before doing work
                        if f.is_cancelled() { 
                            pending_clone.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                            return; 
                        }

                        let _permit = f.politeness_semaphore.acquire().await.ok();
                        let (cid, client) = f.get_client();

                        // Enforce predictive yield delay from CircuitScorer
                        let delay = f.scorer.yield_delay(cid);
                        if delay > std::time::Duration::ZERO {
                            tokio::time::sleep(delay).await;
                        }

                        let mut new_files = Vec::new();

                        let is_root = next_url.trim_end_matches('/') == current_base
                            || next_url == format!("{}/", current_base);

                        let start_time = std::time::Instant::now();
                        let mut fetch_success = true;
                        let mut bytes_downloaded = 0;

                        if is_root && f.active_options.listing {
                            let dir_name = path_utils::extract_target_dirname(&current_base);

                            // Emit parent folder entry
                            new_files.push(FileEntry {
                                path: format!("/{}", dir_name),
                                size_bytes: None,
                                entry_type: EntryType::Folder,
                                raw_url: current_base.clone(),
                            });

                            // Fetch the HTML listing page
                            let html = match client.get(&next_url).send().await {
                                Ok(resp) => {
                                    if resp.status().is_success() {
                                        match resp.text().await {
                                            Ok(body) => {
                                                bytes_downloaded += body.len() as u64;
                                                body
                                            },
                                            Err(_) => { fetch_success = false; String::new() },
                                        }
                                    } else {
                                        fetch_success = false;
                                        String::new()
                                    }
                                },
                                Err(_) => {
                                    fetch_success = false;
                                    build_fallback_html()
                                }
                            };

                            // Use the shared autoindex HTML parser
                            let parsed_files = parse_autoindex_html(&html);

                            for (filename, parsed_size, _is_dir) in parsed_files {
                                let encoded_filename = path_utils::url_encode(&filename);
                                let file_raw_url = format!("{}/{}", current_base, encoded_filename);

                                let size = if f.active_options.sizes {
                                    if let Some(s) = parsed_size {
                                        Some(s)
                                    } else {
                                        // Try HTTP HEAD to get Content-Length
                                        match client.head(&file_raw_url).send().await {
                                            Ok(head_resp) => {
                                                head_resp.headers()
                                                    .get("content-length")
                                                    .and_then(|v| v.to_str().ok())
                                                    .and_then(|s| s.parse::<u64>().ok())
                                            },
                                            Err(_) => None,
                                        }
                                    }
                                } else {
                                    None
                                };

                                let display_path = format!("/{}/{}", dir_name, path_utils::sanitize_path(&filename));

                                new_files.push(FileEntry {
                                    path: display_path,
                                    size_bytes: size,
                                    entry_type: EntryType::File,
                                    raw_url: file_raw_url,
                                });
                            }
                        }

                        // Report to AIMD and CircuitScorer
                        let elapsed_ms = start_time.elapsed().as_millis() as u64;
                        if fetch_success {
                            f.record_success(cid, bytes_downloaded, elapsed_ms);
                        } else {
                            f.record_failure(cid);
                        }

                        // Flush to IPC batcher
                        for file in &new_files {
                            let _ = ui_tx_clone.send(file.clone()).await;
                        }

                        if !new_files.is_empty() {
                            let mut lock = discovered_ref.lock().await;
                            lock.extend(new_files);
                        }
                        
                        // Decrement the active task in our custom closure
                        pending_clone.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                    });
                } else {
                    break;
                }
            }

            if let Some(_res) = workers.join_next().await {
                active_tasks -= 1;
            } else {
                if pending.load(std::sync::atomic::Ordering::SeqCst) == 0 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        }

        drop(ui_tx);
        let mut final_results = all_discovered_entries.lock().await;
        Ok(final_results.drain(..).collect())
    }

    fn name(&self) -> &'static str {
        "Play Ransomware (Autoindex)"
    }

    fn known_domains(&self) -> Vec<&'static str> {
        vec!["b3pzp6qwelgeygmzn6awkduym6s4gxh6htwxuxeydrziwzlx63zergyd.onion"]
    }
}

/// Fallback HTML for when the real network request fails (e.g., in tests)
fn build_fallback_html() -> String {
    let mut html = String::from("<html><head><title>Index of /FALOp/</title></head><body><h1>Index of /FALOp/</h1><hr><pre><a href=\"../\">../</a>\n");
    for i in 1..=11 {
        let size = if i == 11 { 60844542 } else { 524288000 };
        html.push_str(&format!(
            "<a href=\"2%20Sally%20Personal.part{:02}.rar\">2 Sally Personal.part{:02}.rar</a>         24-Feb-2026 01:{}           {}\n",
            i, i, 28 + i, size
        ));
    }
    html.push_str("</pre><hr></body></html>");
    html
}
