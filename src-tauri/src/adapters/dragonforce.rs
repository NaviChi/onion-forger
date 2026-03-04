use crate::adapters::{CrawlerAdapter, EntryType, FileEntry, SiteFingerprint};
use crate::frontier::CrawlerFrontier;
use scraper::{Html, Selector};
use std::sync::Arc;
use tauri::AppHandle;
use tokio::sync::mpsc;

#[derive(Default)]
pub struct DragonForceAdapter;

fn recursive_extract_json(
    val: &serde_json::Value,
    entries: &mut Vec<FileEntry>,
    current_path: String,
    host: &str,
    token: &str,
) {
    match val {
        serde_json::Value::Object(map) => {
            if let Some(name_val) = map.get("name").and_then(|v| v.as_str()) {
                let is_dir = map
                    .get("isDir")
                    .and_then(|v| v.as_bool())
                    .unwrap_or_else(|| {
                        map.get("type")
                            .and_then(|v| v.as_str())
                            .is_some_and(|s| s == "dir" || s == "directory")
                    });
                let has_size = map.contains_key("size");

                if is_dir || has_size {
                    let path = if current_path.is_empty() {
                        format!("/{}", name_val)
                    } else if current_path.ends_with('/') {
                        format!("{}{}", current_path, name_val)
                    } else {
                        format!("{}/{}", current_path, name_val)
                    };

                    let size_bytes = map.get("size").and_then(|v| v.as_u64());
                    let raw_url = format!(
                        "http://{}/?path={}&token={}",
                        host,
                        urlencoding::encode(path.trim_start_matches('/')),
                        token
                    );

                    entries.push(FileEntry {
                        path: path.clone(),
                        size_bytes,
                        entry_type: if is_dir {
                            EntryType::Folder
                        } else {
                            EntryType::File
                        },
                        raw_url,
                    });
                }
            }

            for (_k, v) in map {
                recursive_extract_json(v, entries, current_path.clone(), host, token);
            }
        }
        serde_json::Value::Array(arr) => {
            for v in arr {
                recursive_extract_json(v, entries, current_path.clone(), host, token);
            }
        }
        _ => {}
    }
}

pub fn parse_dragonforce_fsguest(html: &str, host: &str, current_url: &str) -> Vec<FileEntry> {
    let mut entries = Vec::new();

    if html.contains("<iframe") {
        let re = regex::Regex::new(
            r#"src="([^"]+token=[A-Za-z0-9\-_]+\.[A-Za-z0-9\-_]+\.[A-Za-z0-9\-_]+[^"]*)""#,
        )
        .unwrap();
        if let Some(caps) = re.captures(html) {
            let iframe_src = caps.get(1).map(|m| m.as_str()).unwrap_or("");
            if !iframe_src.is_empty() {
                let absolute_url = if iframe_src.starts_with("http") {
                    iframe_src.to_string()
                } else {
                    format!("http://{}{}", host, iframe_src.trim_start_matches('/'))
                };

                entries.push(FileEntry {
                    path: "/_bridge".to_string(),
                    size_bytes: None,
                    entry_type: EntryType::Folder,
                    raw_url: absolute_url,
                });
                return entries;
            }
        }
    }

    let token = if let Some(t_idx) = current_url.find("token=") {
        current_url[t_idx + 6..].split('&').next().unwrap_or("")
    } else {
        ""
    };

    let document = Html::parse_document(html);

    let script_selector = Selector::parse("script#__NEXT_DATA__").unwrap();
    if let Some(script) = document.select(&script_selector).next() {
        if let Ok(json_val) = serde_json::from_str::<serde_json::Value>(&script.inner_html()) {
            recursive_extract_json(&json_val, &mut entries, String::new(), host, token);
            if !entries.is_empty() {
                return entries;
            }
        }
    }

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
                entry_type: if is_dir {
                    EntryType::Folder
                } else {
                    EntryType::File
                },
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
        app: AppHandle,
    ) -> anyhow::Result<Vec<FileEntry>> {
        use tauri::Emitter;

        let queue = Arc::new(crossbeam_queue::SegQueue::new());
        let all_discovered_entries = Arc::new(tokio::sync::Mutex::new(Vec::new()));

        queue.push(current_url.to_string());
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

        let max_concurrent = 120; // Massive worker-stealer parallel pool
        let mut workers = tokio::task::JoinSet::new();

        for _ in 0..max_concurrent {
            let f = frontier.clone();
            let q_clone = queue.clone();
            let ui_tx_clone = ui_tx.clone();
            let discovered_ref = all_discovered_entries.clone();
            let pending_clone = pending.clone();

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

                    struct TaskGuard {
                        counter: Arc<std::sync::atomic::AtomicUsize>,
                    }
                    impl Drop for TaskGuard {
                        fn drop(&mut self) {
                            self.counter.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                        }
                    }
                    let _guard = TaskGuard { counter: pending_clone.clone() };

                    let dynamic_host = if let Ok(u) = url::Url::parse(&next_url) {
                        u.host_str().unwrap_or("").to_string()
                    } else {
                        String::new()
                    };

                    let _permit = f.politeness_semaphore.acquire().await.ok();
                    let (cid, _client) = f.get_client();

                    let delay = f.scorer.yield_delay(cid);
                    if delay > std::time::Duration::ZERO {
                        tokio::time::sleep(delay).await;
                    }

                    let start_time = std::time::Instant::now();
                    let mut fetch_success = false;
                    let mut bytes_downloaded = 0;
                    let mut html = String::new();
                    let mut active_cid = cid;

                    for _ in 0..4 {
                        let (current_cid, current_client) = f.get_client();
                        active_cid = current_cid;

                        let req = current_client.get(&next_url).send();
                        if let Ok(Ok(resp)) =
                            tokio::time::timeout(std::time::Duration::from_secs(45), req).await
                        {
                            if resp.status().is_success() {
                                if let Ok(Ok(body)) = tokio::time::timeout(
                                    std::time::Duration::from_secs(45),
                                    resp.text(),
                                )
                                .await
                                {
                                    bytes_downloaded += body.len() as u64;
                                    html = body;
                                    fetch_success = true;
                                    break;
                                }
                            } else if resp.status() == 404 {
                                break;
                            }
                        }
                        f.record_failure(active_cid);
                    }

                    let elapsed_ms = start_time.elapsed().as_millis() as u64;
                    if fetch_success {
                        f.record_success(active_cid, bytes_downloaded, elapsed_ms);
                    } else {
                        f.record_failure(active_cid);
                    }

                    if fetch_success && !html.is_empty() {
                        let mut new_files =
                            parse_dragonforce_fsguest(&html, &dynamic_host, &next_url);
                        let _is_nextjs = html.contains("__NEXT_DATA__");

                        for doc in &new_files {
                            if doc.entry_type == EntryType::Folder && f.mark_visited(&doc.raw_url) {
                                pending_clone.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                                q_clone.push(doc.raw_url.clone());
                            }
                            let _ = ui_tx_clone.send(doc.clone()).await;
                        }

                        if !new_files.is_empty() {
                            discovered_ref.lock().await.append(&mut new_files);
                        }
                    }
                }
            });
        }

        while workers.join_next().await.is_some() {}

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
            "dragonforxxbp3awc7mzs5dkswrua3znqyx5roefmi4smjrsdi22xwqd.onion",
        ]
    }
}
