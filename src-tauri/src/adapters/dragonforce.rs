use crate::adapters::{CrawlerAdapter, EntryType, FileEntry, SiteFingerprint};
use crate::frontier::CrawlerFrontier;
use scraper::{Html, Selector};
use std::sync::Arc;
use tauri::AppHandle;
use tokio::sync::mpsc;

#[derive(Default)]
pub struct DragonForceAdapter;

impl DragonForceAdapter {
    pub fn new() -> Self {
        Self
    }
}

#[allow(dead_code)]
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
                    // DragonForce's Next.js SPA usually provides the absolute path in `map.get("path")`
                    let path = if let Some(p) = map.get("path").and_then(|v| v.as_str()) {
                        p.to_string()
                    } else if current_path.is_empty() {
                        format!("/{}", name_val)
                    } else if current_path.ends_with('/') {
                        format!("{}{}", current_path, name_val)
                    } else {
                        format!("{}/{}", current_path, name_val)
                    };

                    let size_bytes = map.get("size").and_then(|v| v.as_u64());

                    // Segregate NextJS HTML routes (/?path=) from Backend API downloads (/download?path=)
                    let api_endpoint = if is_dir { "" } else { "download" };
                    let raw_url = format!(
                        "http://{}/{}?path={}&token={}",
                        host,
                        api_endpoint,
                        urlencoding::encode(path.trim_start_matches('/')),
                        token
                    );

                    entries.push(FileEntry {
                        jwt_exp: None,
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

fn extract_jwt_expiry(url: &str) -> Option<u64> {
    use base64::Engine;
    if let Some(token_start) = url.find("token=") {
        let token_str = &url[token_start + 6..];
        let jwt = token_str.split('&').next().unwrap_or("");
        let parts: Vec<&str> = jwt.split('.').collect();
        if parts.len() == 3 {
            let payload = parts[1];
            let mut padded_payload = payload.to_string();
            if padded_payload.len() % 4 != 0 {
                padded_payload.push_str(&"=".repeat(4 - padded_payload.len() % 4));
            }
            if let Ok(decoded) = base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(payload)
                .or_else(|_| base64::engine::general_purpose::STANDARD.decode(&padded_payload))
            {
                if let Ok(json_str) = String::from_utf8(decoded) {
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&json_str) {
                        return value.get("exp").and_then(|v| v.as_u64());
                    }
                }
            }
        }
    }
    None
}

pub fn parse_dragonforce_fsguest(html: &str, host: &str, current_url: &str) -> Vec<FileEntry> {
    let html_len = html.len();
    let start_idx = html_len.saturating_sub(4000);
    println!(
        "[DEBUG DRAGONFORCE PARSER] Raw HTML end ({} bytes): {}",
        html_len,
        &html[start_idx..]
    );
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
                    jwt_exp: None,
                    path: "/_bridge".to_string(),
                    size_bytes: None,
                    entry_type: EntryType::Folder,
                    raw_url: absolute_url,
                });
                return entries;
            }
        }
    }

    let document = Html::parse_document(html);

    let next_data_selector = Selector::parse(r#"script#__NEXT_DATA__"#).unwrap();
    if let Some(next_data) = document.select(&next_data_selector).next() {
        let payload = next_data.text().collect::<Vec<_>>().join("");
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&payload) {
            let token = current_url
                .split("token=")
                .nth(1)
                .and_then(|value| value.split('&').next())
                .unwrap_or("");
            let current_path = current_url
                .split("path=")
                .nth(1)
                .and_then(|value| value.split('&').next())
                .map(|value| {
                    urlencoding::decode(value)
                        .unwrap_or(std::borrow::Cow::Borrowed(value))
                        .into_owned()
                })
                .unwrap_or_default();

            recursive_extract_json(&json, &mut entries, current_path, host, token);
            if !entries.is_empty() {
                return entries;
            }
        }
    }

    // Modern static DragonForce UI
    let item_selector = Selector::parse(".item").unwrap();
    let link_selector = Selector::parse("a.text-pointer-animations").unwrap();
    let size_selector = Selector::parse("div.size").unwrap();

    for item in document.select(&item_selector) {
        if let Some(link) = item.select(&link_selector).next() {
            let is_dir = link.value().classes().any(|c| c == "dir");
            let href = link.value().attr("href").unwrap_or("");

            if href.starts_with("javascript:") {
                continue;
            }

            let raw_url = format!("http://{}{}", host, href);

            // Extract the path from the href
            // href format: /?path=RJZ-APP1/G/01%20RJZ&token=...
            // or /download?path=RJZ-APP1/...&token=...
            let mut extracted_path = String::new();
            if let Some(path_start) = href.find("path=") {
                let after_path = &href[path_start + 5..];
                if let Some(path_end) = after_path.find("&token=") {
                    let encoded_path = &after_path[..path_end];
                    extracted_path = urlencoding::decode(encoded_path)
                        .unwrap_or(std::borrow::Cow::Borrowed(encoded_path))
                        .to_string();
                } else {
                    extracted_path = urlencoding::decode(after_path)
                        .unwrap_or(std::borrow::Cow::Borrowed(after_path))
                        .to_string();
                }
            }

            if extracted_path.is_empty() {
                continue;
            }

            let path = format!("/{}", extracted_path.trim_start_matches('/'));

            let mut size_bytes = None;
            if !is_dir {
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

            let jwt_exp = extract_jwt_expiry(&raw_url);

            entries.push(FileEntry {
                jwt_exp,
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

    async fn refresh_jwt(
        &self,
        entry: &FileEntry,
        client: &crate::arti_client::ArtiClient,
    ) -> anyhow::Result<Option<FileEntry>> {
        // Find the `path=` param.
        let path_start = match entry.raw_url.find("path=") {
            Some(idx) => idx,
            None => return Ok(None),
        };
        let after_path = &entry.raw_url[path_start + 5..];

        let mut encoded_path = after_path;
        if let Some(token_idx) = after_path.find("&token=") {
            encoded_path = &after_path[..token_idx];
        }

        let decoded = urlencoding::decode(encoded_path)
            .unwrap_or(std::borrow::Cow::Borrowed(encoded_path))
            .to_string();

        // Extract parent directory.
        let mut parent_path = "/".to_string();
        if let Some(last_slash) = decoded.rfind('/') {
            if last_slash > 0 {
                parent_path = decoded[..last_slash].to_string();
            }
        }

        // Construct parent URL.
        let host = if let Ok(u) = url::Url::parse(&entry.raw_url) {
            u.host_str().unwrap_or("").to_string()
        } else {
            return Ok(None);
        };

        // We do a fresh GET to the root `/?path=/parent` to force a NextNode regeneration.
        // E.g., http://fsguest...onion/?path=/parent/directory
        let fetch_url = format!(
            "http://{}/?path={}",
            host,
            urlencoding::encode(&parent_path)
        );

        println!(
            "[DEBUG DRAGONFORCE] Refreshing JWT Token for {}. Upstream parent: {}",
            entry.path, fetch_url
        );

        let resp = match tokio::time::timeout(
            std::time::Duration::from_secs(30),
            client.get(&fetch_url).send(),
        )
        .await
        {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => {
                println!("[CRIT DRAGONFORCE REFRESH] HTTP client error: {}", e);
                return Ok(None);
            }
            Err(_) => {
                println!("[CRIT DRAGONFORCE REFRESH] Artifact path discovery timeout.");
                return Ok(None);
            }
        };

        if !resp.status().is_success() {
            println!(
                "[CRIT DRAGONFORCE REFRESH] Non-success status: {}",
                resp.status()
            );
            return Ok(None);
        }

        let html = match resp.text().await {
            Ok(txt) => txt,
            Err(_) => return Ok(None),
        };

        // Parse children of the parent. We only want our specific file/folder back.
        let newly_parsed = parse_dragonforce_fsguest(&html, &host, &fetch_url);

        for mut child in newly_parsed {
            if child.path == entry.path {
                println!(
                    "[DEBUG DRAGONFORCE] Successfully refreshed Token for: {}",
                    entry.path
                );
                child.size_bytes = entry.size_bytes; // Preserve original explicit size if parsed.
                return Ok(Some(child));
            }
        }

        println!(
            "[CRIT DRAGONFORCE REFRESH] Could not find {} inside the upstream parent html dump.",
            entry.path
        );
        Ok(None)
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
        let multi_clients = frontier.active_options.circuits.unwrap_or_else(|| {
            std::env::var("CRAWLI_MULTI_CLIENTS")
                .ok()
                .and_then(|v| v.trim().parse::<usize>().ok())
                .unwrap_or(4)
        });
        let timer = crate::timer::CrawlTimer::new(app.clone());
        timer.emit_log(&format!(
            "[DragonForce] Bootstrapping MultiClientPool with {} independent TorClients...",
            multi_clients
        ));
        let _ = app.emit(
            "log",
            format!(
                "[DragonForce] Bootstrapping MultiClientPool with {} independent TorClients...",
                multi_clients
            ),
        );
        let multi_pool = Arc::new(
            crate::multi_client_pool::MultiClientPool::new(multi_clients)
                .await
                .unwrap(),
        );

        timer.emit_log("[DragonForce] Concurrent Pre-heating of MultiClientPool circuits to cache HS descriptors...");
        let _ = app.emit("log", "[DragonForce] Concurrent Pre-heating of MultiClientPool circuits to cache HS descriptors...".to_string());
        let mut preheats = Vec::new();
        for i in 0..multi_clients {
            let tor_arc = multi_pool.get_client(i).await;
            let preheat_client = crate::arti_client::ArtiClient::new((*tor_arc).clone(), None);
            let target_heat_url = current_url.to_string();
            preheats.push(tokio::spawn(async move {
                let _ = tokio::time::timeout(
                    std::time::Duration::from_secs(55),
                    preheat_client.get(&target_heat_url).send(),
                )
                .await;
            }));
        }
        futures::future::join_all(preheats).await;
        timer.emit_log("[DragonForce] Pre-heating complete. Unleashing workers.");
        let _ = app.emit(
            "log",
            "[DragonForce] Pre-heating complete. Unleashing workers.".to_string(),
        );

        let max_concurrent = frontier.recommended_listing_workers();
        let mut workers = tokio::task::JoinSet::new();

        for worker_idx in 0..max_concurrent {
            let f = frontier.clone();
            let pool_clone = multi_pool.clone();
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
                            self.counter
                                .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                        }
                    }
                    let _guard = TaskGuard {
                        counter: pending_clone.clone(),
                    };

                    let dynamic_host = if let Ok(u) = url::Url::parse(&next_url) {
                        u.host_str().unwrap_or("").to_string()
                    } else {
                        String::new()
                    };

                    if let Some(exp) = extract_jwt_expiry(&next_url) {
                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs();
                        if exp <= now + 30 {
                            println!(
                                "[JWT EXPIRED] Skipping {}. Token exp: {}, Now: {}",
                                next_url, exp, now
                            );
                            continue;
                        }
                    }

                    let _permit = f.politeness_semaphore.acquire().await.ok();
                    let (cid, _) = f.get_client(); // Kept to acquire a cid for politeness delays

                    let delay = f.scorer.yield_delay(cid);
                    if delay > std::time::Duration::ZERO {
                        tokio::time::sleep(delay).await;
                    }

                    let start_time = std::time::Instant::now();
                    let mut fetch_success = false;
                    let mut bytes_downloaded = 0;
                    let mut html = String::new();
                    let mut active_cid = cid;
                    let mut ddos_guard = crate::adapters::qilin_ddos_guard::DdosGuard::new();

                    for _ in 0..4 {
                        let (current_cid, _) = f.get_client();
                        active_cid = current_cid;

                        let is_fsguest = next_url.contains("fsguest");
                        let tor_arc = if is_fsguest {
                            pool_clone.get_client(worker_idx).await
                        } else {
                            pool_clone.get_client(0).await
                        };
                        let current_client =
                            crate::arti_client::ArtiClient::new((*tor_arc).clone(), None);

                        let req = current_client.get(&next_url).send();
                        if let Ok(Ok(resp)) =
                            tokio::time::timeout(std::time::Duration::from_secs(45), req).await
                        {
                            if let Some(delay) = ddos_guard.record_response(resp.status().as_u16())
                            {
                                tokio::time::sleep(delay).await;
                            }

                            println!(
                                "[DEBUG DRAGONFORCE] Fetch status for {}: {}",
                                next_url,
                                resp.status()
                            );

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
                        } else {
                            println!(
                                "[DEBUG DRAGONFORCE] Timeout or client error connecting to {}",
                                next_url
                            );
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
