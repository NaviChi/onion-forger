use tauri::AppHandle;
use std::sync::Arc;
use tokio::sync::mpsc;
use crate::adapters::{CrawlerAdapter, SiteFingerprint, FileEntry, EntryType};
use crate::frontier::CrawlerFrontier;
use scraper::{Html, Selector};

#[derive(Default)]
pub struct DragonForceAdapter;

/// Parses the DragonForce `fsguest` HTML listing to extract files and directories.
/// Directory links: `<a class="text-pointer-animations dir" href="/?path=...&token=...">Dir/</a>`
/// File links: `<a class="text-pointer-animations" href="/download?path=...&token=...">File.ext</a>`
/// File sizes: `<div class="size"><b>...</b> (bytes)</div>`
pub fn parse_dragonforce_fsguest(html: &str, host: &str) -> Vec<FileEntry> {
    let mut entries = Vec::new();
    let document = Html::parse_document(html);
    
    // Select all items in the list
    let item_selector = Selector::parse(".item").unwrap();
    let link_selector = Selector::parse("a.text-pointer-animations").unwrap();
    let size_selector = Selector::parse("div.size").unwrap();

    for item in document.select(&item_selector) {
        if let Some(link) = item.select(&link_selector).next() {
            let is_dir = link.value().classes().any(|c| c == "dir");
            let href = link.value().attr("href").unwrap_or("");
            
            // Skip back link
            if href.starts_with("javascript:") {
                continue;
            }

            let raw_url = format!("http://{}{}", host, href);
            
            // Extract the path from the href
            // href is like: /?path=RJZ-APP1/G/01%20RJZ&token=...
            // or /download?path=RJZ-APP1/...&token=...
            let mut extracted_path = String::new();
            if let Some(path_start) = href.find("path=") {
                let after_path = &href[path_start + 5..];
                if let Some(path_end) = after_path.find("&token=") {
                    let encoded_path = &after_path[..path_end];
                    
                    // Robust URL decode instead of just replace("%20")
                    extracted_path = urlencoding::decode(encoded_path)
                        .unwrap_or(std::borrow::Cow::Borrowed(encoded_path))
                        .to_string();
                }
            }

            if extracted_path.is_empty() {
                continue; // Could not parse path
            }

            let path = format!("/{}", extracted_path.trim_start_matches('/'));

            let mut size_bytes = None;
            if !is_dir {
                // Try to parse the exact byte size from `<b>...</b> (bytes)`
                if let Some(size_div) = item.select(&size_selector).next() {
                    let text = size_div.text().collect::<Vec<_>>().join("");
                    if let Some(start) = text.find('(') {
                        if let Some(end) = text.find(')') {
                            if let Ok(bytes) = text[start + 1..end].parse::<u64>() {
                                size_bytes = Some(bytes);
                            }
                        }
                    }
                }
            }

            entries.push(FileEntry {
                path,
                size_bytes,
                entry_type: if is_dir { EntryType::Folder } else { EntryType::File },
                raw_url,
            });
        }
    }

    entries
}

#[async_trait::async_trait]
impl CrawlerAdapter for DragonForceAdapter {
    async fn can_handle(&self, fingerprint: &SiteFingerprint) -> bool {
        fingerprint.body.contains("fsguest") || fingerprint.body.contains("token=")
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
        
        let pending = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        pending.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

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

        let max_concurrent = 20;
        let mut active_tasks = 0;
        let mut workers = tokio::task::JoinSet::new();

        let host = if let Ok(u) = url::Url::parse(current_url) {
            u.host_str().unwrap_or("").to_string()
        } else {
            String::new()
        };

        loop {
            if frontier.is_cancelled() { break; }

            while active_tasks < max_concurrent {
                if let Ok(next_url) = rx.try_recv() {
                    let f = frontier.clone();
                    let tx_clone = tx.clone();
                    let ui_tx_clone = ui_tx.clone();
                    let discovered_ref = all_discovered_entries.clone();
                    let current_host = host.clone();
                    let pending_clone = pending.clone();

                    active_tasks += 1;
                    workers.spawn(async move {
                        if f.is_cancelled() { 
                            pending_clone.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                            return; 
                        }
                        let _permit = f.politeness_semaphore.acquire().await.ok();
                        let (cid, client) = f.get_client();

                        let delay = f.scorer.yield_delay(cid);
                        if delay > std::time::Duration::ZERO {
                            tokio::time::sleep(delay).await;
                        }

                        let start_time = std::time::Instant::now();
                        let mut fetch_success = true;
                        let mut bytes_downloaded = 0;

                        let html = match client.get(&next_url).send().await {
                            Ok(resp) if resp.status().is_success() => {
                                match resp.text().await {
                                    Ok(body) => {
                                        bytes_downloaded += body.len() as u64;
                                        body
                                    }
                                    Err(_) => { fetch_success = false; String::new() }
                                }
                            }
                            _ => { fetch_success = false; String::new() }
                        };

                        let elapsed_ms = start_time.elapsed().as_millis() as u64;
                        if fetch_success {
                            f.record_success(cid, bytes_downloaded, elapsed_ms);
                        } else {
                            f.record_failure(cid);
                        }

                        if fetch_success && !html.is_empty() {
                            let mut new_files = parse_dragonforce_fsguest(&html, &current_host);
                            
                            for doc in &new_files {
                                if doc.entry_type == EntryType::Folder {
                                    if f.mark_visited(&doc.raw_url) {
                                        pending_clone.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                                        let _ = tx_clone.send(doc.raw_url.clone());
                                    }
                                }
                                let _ = ui_tx_clone.send(doc.clone()).await;
                            }

                            if !new_files.is_empty() {
                                discovered_ref.lock().await.append(&mut new_files);
                            }
                        }
                        
                        pending_clone.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                    });
                } else {
                    break;
                }
            }

            if let Some(_) = workers.join_next().await {
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
        "DragonForce Iframe SPA"
    }

    fn known_domains(&self) -> Vec<&'static str> {
        vec![
            "fsguestuctexqqaoxuahuydfa6ovxuhtng66pgyr5gqcrsi7qgchpkad.onion",
            "dragonforxxbp3awc7mzs5dkswrua3znqyx5roefmi4smjrsdi22xwqd.onion"
        ]
    }
}
