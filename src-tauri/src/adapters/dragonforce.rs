use crate::adapters::{CrawlerAdapter, EntryType, FileEntry, SiteFingerprint};
use crate::frontier::CrawlerFrontier;
use scraper::{Html, Selector};
use std::sync::Arc;
use tauri::{AppHandle, Manager};
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

/// Recursively search __NEXT_DATA__ JSON for a JWT access token.
/// DragonForce commonly nests the token inside props.pageProps or similar paths.
fn extract_token_from_next_data(json: &serde_json::Value) -> Option<String> {
    match json {
        serde_json::Value::Object(map) => {
            // Check common key names for tokens
            for key in &["token", "accessToken", "access_token", "jwt", "authToken"] {
                if let Some(serde_json::Value::String(val)) = map.get(*key) {
                    // Looks like a JWT? (has dots separating three parts)
                    if val.matches('.').count() >= 2 && val.len() > 20 {
                        return Some(val.clone());
                    }
                    // Could also be a plain token
                    if val.len() > 10 {
                        return Some(val.clone());
                    }
                }
            }
            // Recurse into child values
            for (_k, v) in map {
                if let Some(found) = extract_token_from_next_data(v) {
                    return Some(found);
                }
            }
            None
        }
        serde_json::Value::Array(arr) => {
            for v in arr {
                if let Some(found) = extract_token_from_next_data(v) {
                    return Some(found);
                }
            }
            None
        }
        _ => None,
    }
}

/// Recursively search __NEXT_DATA__ JSON for a fsguest / .onion iframe URL.
/// Returns the first URL string containing "fsguest" or ".onion" with "token=" or "path=".
fn extract_iframe_url_from_next_data(json: &serde_json::Value) -> Option<String> {
    match json {
        serde_json::Value::String(s) => {
            if (s.contains("fsguest") || s.contains(".onion"))
                && (s.contains("token=") || s.contains("path="))
                && s.starts_with("http")
            {
                return Some(s.clone());
            }
            None
        }
        serde_json::Value::Object(map) => {
            // Prefer keys named "url", "src", "iframeSrc", "iframe"
            for key in &["url", "src", "iframeSrc", "iframe", "iframeUrl", "link"] {
                if let Some(serde_json::Value::String(val)) = map.get(*key) {
                    if (val.contains("fsguest") || val.contains(".onion"))
                        && (val.contains("token=") || val.contains("path="))
                        && val.starts_with("http")
                    {
                        return Some(val.clone());
                    }
                }
            }
            for (_k, v) in map {
                if let Some(found) = extract_iframe_url_from_next_data(v) {
                    return Some(found);
                }
            }
            None
        }
        serde_json::Value::Array(arr) => {
            for v in arr {
                if let Some(found) = extract_iframe_url_from_next_data(v) {
                    return Some(found);
                }
            }
            None
        }
        _ => None,
    }
}

pub fn parse_dragonforce_fsguest(html: &str, host: &str, current_url: &str) -> Vec<FileEntry> {
    let mut entries = Vec::new();

    // ── Phase 1: Iframe detection (DragonForce wraps fsguest in an iframe) ──
    if html.contains("<iframe") {
        // Try JWT-bearing iframe first
        let re = regex::Regex::new(
            r#"src=["']([^"']+token=[A-Za-z0-9\-_]+\.[A-Za-z0-9\-_]+\.[A-Za-z0-9\-_]+[^"']*)["']"#,
        )
        .unwrap();
        let iframe_src = if let Some(caps) = re.captures(html) {
            caps.get(1).map(|m| m.as_str().to_string())
        } else {
            // Fallback: look for any iframe src pointing to an onion or containing token/path
            let fallback_re = regex::Regex::new(
                r#"<iframe[^>]+src=["']([^"']*(?:\.onion|token=|path=)[^"']*)["']"#,
            )
            .unwrap();
            fallback_re
                .captures(html)
                .and_then(|caps| caps.get(1).map(|m| m.as_str().to_string()))
        };

        if let Some(src) = iframe_src {
            if !src.is_empty() {
                let absolute_url = if src.starts_with("http") {
                    src
                } else {
                    format!("http://{}/{}", host, src.trim_start_matches('/'))
                };

                println!(
                    "[DRAGONFORCE] Detected iframe bridge → {}",
                    &absolute_url[..absolute_url.len().min(120)]
                );

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

    // ── Phase 2: __NEXT_DATA__ extraction ──
    let next_data_selector = Selector::parse(r#"script#__NEXT_DATA__"#).unwrap();
    if let Some(next_data) = document.select(&next_data_selector).next() {
        let payload = next_data.text().collect::<Vec<_>>().join("");
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&payload) {
            // Try token from URL first, then from __NEXT_DATA__ JSON
            let url_token = current_url
                .split("token=")
                .nth(1)
                .and_then(|value| value.split('&').next())
                .unwrap_or("")
                .to_string();

            let json_token = if url_token.is_empty() {
                // Search common DragonForce __NEXT_DATA__ paths for the access token
                extract_token_from_next_data(&json).unwrap_or_default()
            } else {
                String::new()
            };

            let token = if !url_token.is_empty() {
                url_token
            } else {
                json_token
            };

            // If __NEXT_DATA__ contains an iframe URL / fsguest URL, extract it as bridge
            if let Some(iframe_url) = extract_iframe_url_from_next_data(&json) {
                println!(
                    "[DRAGONFORCE] __NEXT_DATA__ contains fsguest URL → {}",
                    &iframe_url[..iframe_url.len().min(120)]
                );
                entries.push(FileEntry {
                    jwt_exp: None,
                    path: "/_bridge".to_string(),
                    size_bytes: None,
                    entry_type: EntryType::Folder,
                    raw_url: iframe_url,
                });
                return entries;
            }

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

            if !token.is_empty() {
                recursive_extract_json(&json, &mut entries, current_path, host, &token);
                if !entries.is_empty() {
                    return entries;
                }
            } else {
                println!(
                    "[DRAGONFORCE] __NEXT_DATA__ found but no token available — skipping JSON extraction"
                );
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
        // Body marker checks
        if fingerprint.body.contains("fsguest") || fingerprint.body.contains("token=") {
            return true;
        }
        // Domain shortcut — the known dragonforce portal domain
        if fingerprint.url.contains("dragonfor") {
            return true;
        }
        // Iframe-based embedding (DragonForce wraps fsguest in an iframe)
        if fingerprint.body.contains("<iframe") && fingerprint.body.contains("src=") {
            return true;
        }
        false
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

        let html = match resp.bytes().await {
            Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
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

        let queue = Arc::new(crate::spillover::SpilloverQueue::new());
        let all_discovered_entries = Arc::new(crate::spillover::SpilloverList::new());

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
        let telemetry = app
            .try_state::<crate::AppState>()
            .map(|s| s.telemetry.clone());
        let seeded_clients = if let Some(guard_arc) = &frontier.swarm_guard {
            let guard = guard_arc.lock().await;
            let shared_clients = guard.get_arti_clients();
            crate::multi_client_pool::snapshot_seed_clients(&shared_clients, multi_clients)
        } else {
            Vec::new()
        };
        let multi_pool = Arc::new(
            crate::multi_client_pool::MultiClientPool::new_seeded(
                multi_clients,
                seeded_clients,
                telemetry,
            )
            .await
            .unwrap(),
        );

        if multi_pool.borrowed_client_count() > 0 {
            let message = format!(
                "[DragonForce] Seeded MultiClientPool with {}/{} hot Arti clients from the active swarm.",
                multi_pool.borrowed_client_count(),
                multi_clients
            );
            timer.emit_log(&message);
            let _ = app.emit("log", message);
        }

        timer.emit_log("[DragonForce] Concurrent Pre-heating of MultiClientPool circuits to cache HS descriptors...");
        let _ = app.emit("log", "[DragonForce] Concurrent Pre-heating of MultiClientPool circuits to cache HS descriptors...".to_string());
        let mut preheats = Vec::new();
        let preheat_count = multi_clients.min(8); // Only need a few clients to cache HS descriptors
        for i in 0..preheat_count {
            let multi_pool_clone = multi_pool.clone();
            let target_heat_url = current_url.to_string();
            preheats.push(tokio::spawn(async move {
                let tor_arc = multi_pool_clone.get_client(i).await;
                let preheat_client = crate::arti_client::ArtiClient::new((*tor_arc).clone(), None);
                let _ = tokio::time::timeout(
                    std::time::Duration::from_secs(55),
                    preheat_client.head(&target_heat_url).send(),
                )
                .await;
            }));
        }
        // Phase 118: Always wait for at least 1 preheat to succeed.
        // Even with seeded clients, the HS descriptor for the target domain
        // may not be cached (especially when adapter hint skips warmup).
        if !preheats.is_empty() {
            let (result, _index, remaining) = futures::future::select_all(preheats).await;
            let _ = result;
            // Let remaining preheats finish in background — they cache HS descriptors
            // for faster subsequent fetches by other workers
            for handle in remaining {
                tokio::spawn(async move {
                    let _ = handle.await;
                });
            }
        }
        timer.emit_log("[DragonForce] Pre-heating complete. Unleashing workers.");
        let _ = app.emit(
            "log",
            "[DragonForce] Pre-heating complete. Unleashing workers.".to_string(),
        );

        let max_concurrent = frontier.recommended_listing_workers();

        // ═══════════════════════════════════════════════════════════
        // Phase 119: Inverted Retry Queue + Adaptive Workers + Connection Reuse
        // ═══════════════════════════════════════════════════════════
        //
        // Strategy 1 — Inverted Retry Queue: Failed URLs pushed to retry_queue
        //   with an unlock timestamp. Workers never block on retries.
        // Strategy 2 — Adaptive Workers: Start with min(12, max) workers.
        // Strategy 3 — Connection Reuse: Each worker builds ArtiClient ONCE
        //   per TorClient, reuses HTTP keep-alive across fetches.
        // Strategy 4 — Failure Classification: timeout→re-queue, 403→skip,
        //   429→backoff, 404→dead.
        // ═══════════════════════════════════════════════════════════

        struct RetryPayload {
            url: String,
            attempt: u8,
            unlock_at: std::time::Instant,
        }
        let retry_queue: Arc<crossbeam_queue::SegQueue<RetryPayload>> =
            Arc::new(crossbeam_queue::SegQueue::new());

        // ═══════════════════════════════════════════════════════════
        // Phase 121: Multi-Probe Seed Bootstrap
        // ═══════════════════════════════════════════════════════════
        // Problem: With 1 seed URL and 12 workers, only 1 worker gets
        // the URL. If that circuit is bad, we wait 10-45s per retry.
        // Solution: Race N concurrent probes across different circuits.
        // First success wins; losers cancel. This cuts bootstrap from
        // ~40s (serial retries) to ~12s (parallel probes).
        // ═══════════════════════════════════════════════════════════
        {
            use tauri::Emitter;
            let seed_url = queue.pop().unwrap(); // We know exactly 1 item is queued
            let probe_count = multi_pool.clients_count().min(4); // Race on 4 circuits
            let _ = app.emit("log", format!(
                "[DragonForce] Multi-probe bootstrap: racing {} concurrent probes on seed URL",
                probe_count
            ));
            println!(
                "[DragonForce] Multi-probe bootstrap: racing {} probes for seed URL",
                probe_count
            );

            let (probe_tx, mut probe_rx) = mpsc::channel::<(String, Vec<FileEntry>)>(1);
            let cancel_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));

            let mut probe_handles = Vec::new();
            for probe_idx in 0..probe_count {
                let pool_ref = multi_pool.clone();
                let url = seed_url.clone();
                let ptx = probe_tx.clone();
                let cf = cancel_flag.clone();

                probe_handles.push(tokio::spawn(async move {
                    if cf.load(std::sync::atomic::Ordering::Relaxed) { return; }
                    let tor_arc = pool_ref.get_client(probe_idx).await;
                    let probe_client = crate::arti_client::ArtiClient::new((*tor_arc).clone(), None);

                    // Use 30s timeout for initial HS probe (HS descriptor + circuit)
                    let result = tokio::time::timeout(
                        std::time::Duration::from_secs(30),
                        probe_client.get(&url).header("Connection", "keep-alive").send()
                    ).await;

                    match result {
                        Ok(Ok(resp)) if resp.status().is_success() => {
                            // Phase 122 P2→124 P2: bytes() + size-gated decode
                            if let Ok(Ok(body_bytes)) = tokio::time::timeout(
                                std::time::Duration::from_secs(20),
                                resp.bytes(),
                            ).await {
                                let url_clone = url.clone();
                                // Phase 124 P2: inline decode for <4KB, spawn_blocking for ≥4KB
                                let entries = if body_bytes.len() < 4096 {
                                    let body = String::from_utf8_lossy(&body_bytes);
                                    let parsed_url = url::Url::parse(&url_clone).ok();
                                    let host = parsed_url.as_ref()
                                        .and_then(|u| u.host_str())
                                        .unwrap_or("")
                                        .to_string();
                                    parse_dragonforce_fsguest(&body, &host, &url_clone)
                                } else {
                                    tokio::task::spawn_blocking(move || {
                                        let body = String::from_utf8_lossy(&body_bytes);
                                        let parsed_url = url::Url::parse(&url_clone).ok();
                                        let host = parsed_url.as_ref()
                                            .and_then(|u| u.host_str())
                                            .unwrap_or("")
                                            .to_string();
                                        parse_dragonforce_fsguest(&body, &host, &url_clone)
                                    }).await.unwrap_or_default()
                                };
                                let _ = ptx.send((url, entries)).await;
                            }
                        }
                        Ok(Ok(resp)) => {
                            println!(
                                "[DragonForce Probe {}] Non-success: {}",
                                probe_idx, resp.status().as_u16()
                            );
                        }
                        Ok(Err(e)) => {
                            println!(
                                "[DragonForce Probe {}] Error: {}",
                                probe_idx, e
                            );
                        }
                        Err(_) => {
                            println!(
                                "[DragonForce Probe {}] Timeout (30s)",
                                probe_idx
                            );
                        }
                    }
                }));
            }
            drop(probe_tx); // Close sender so recv() can complete

            // Wait for first successful probe (or all fail)
            let mut seed_parsed = false;
            if let Some((_, entries)) = probe_rx.recv().await {
                cancel_flag.store(true, std::sync::atomic::Ordering::Relaxed); // Signal remaining probes
                let _ = app.emit("log", format!(
                    "[DragonForce] Seed probe succeeded! {} entries from initial page",
                    entries.len()
                ));
                println!(
                    "[DragonForce] Seed probe won! {} entries parsed",
                    entries.len()
                );

                for doc in &entries {
                    if doc.entry_type == EntryType::Folder && frontier.mark_visited(&doc.raw_url) {
                        pending.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        queue.push(doc.raw_url.clone());
                        if doc.path == "/_bridge" {
                            let _ = app.emit("log", format!(
                                "[DragonForce] Bridge redirect detected → {}",
                                &doc.raw_url[..doc.raw_url.len().min(120)]
                            ));
                        }
                    }
                    let _ = ui_tx.send(doc.clone()).await;
                }
                if !entries.is_empty() {
                    for nf in entries {
                        all_discovered_entries.push(nf);
                    }
                    seed_parsed = true;
                }
            }

            // Ensure all probe tasks complete — with hard timeout to handle
            // Arti futures that aren't fully cancellation-safe
            let probe_cleanup_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(35);
            for h in probe_handles {
                if tokio::time::timeout_at(probe_cleanup_deadline, h).await.is_err() {
                    println!("[DragonForce] Seed probe cleanup timed out — aborting remaining");
                    break;
                }
            }

            if !seed_parsed {
                // All probes failed — push seed back for workers to retry
                let _ = app.emit("log", format!(
                    "[DragonForce] All {} seed probes failed. Queuing for worker retry.",
                    probe_count
                ));
                queue.push(seed_url);
            }
        }

        // ═══════════════════════════════════════════════════════════
        // Phase 123: ?search= API Probe for Tree Flattening
        // ═══════════════════════════════════════════════════════════
        // DragonForce uses Next.js SPAs that occasionally expose
        // search/list APIs. A single wildcard search can flatten the
        // entire directory tree without recursive BFS crawling.
        // Endpoints probed (priority order):
        //   1. /api/search?query=*&token=TOKEN
        //   2. /api/files?path=/&recursive=true&token=TOKEN
        //   3. /api/list?path=/&search=*&token=TOKEN
        // If any returns a JSON array with file entries, we inject
        // them directly into the discovered set and skip BFS for
        // already-covered paths.
        // ═══════════════════════════════════════════════════════════
        {
            use tauri::Emitter;
            // Extract token from the seed URL for API authentication
            let search_token = current_url
                .split("token=")
                .nth(1)
                .and_then(|v| v.split('&').next())
                .unwrap_or("")
                .to_string();

            let parsed_base = url::Url::parse(current_url).ok();
            let search_host = parsed_base
                .as_ref()
                .and_then(|u| u.host_str())
                .unwrap_or("")
                .to_string();

            if !search_token.is_empty() && !search_host.is_empty() {
                let search_endpoints = vec![
                    format!(
                        "http://{}/api/search?query=*&token={}",
                        search_host, search_token
                    ),
                    format!(
                        "http://{}/api/files?path=/&recursive=true&token={}",
                        search_host, search_token
                    ),
                    format!(
                        "http://{}/api/list?path=/&search=*&token={}",
                        search_host, search_token
                    ),
                ];

                let _ = app.emit(
                    "log",
                    format!(
                        "[DragonForce] Phase 123: Probing {} search endpoints for tree flattening...",
                        search_endpoints.len()
                    ),
                );
                println!(
                    "[DragonForce Phase 123] Probing {} search endpoints for tree flattening",
                    search_endpoints.len()
                );

                let probe_pool = multi_pool.clone();
                let probe_host = search_host.clone();
                let probe_token = search_token.clone();

                // Race all search probes concurrently with 20s timeout each
                let mut search_joinset = tokio::task::JoinSet::new();
                for (ep_idx, endpoint) in search_endpoints.into_iter().enumerate() {
                    let pool_ref = probe_pool.clone();
                    let host_ref = probe_host.clone();
                    let token_ref = probe_token.clone();
                    search_joinset.spawn(async move {
                        let slot = ep_idx % pool_ref.clients_count().max(1);
                        let tor_arc = pool_ref.get_client(slot).await;
                        let search_client =
                            crate::arti_client::ArtiClient::new((*tor_arc).clone(), None);
                        let result = tokio::time::timeout(
                            std::time::Duration::from_secs(20),
                            search_client
                                .get(&endpoint)
                                .header("Connection", "keep-alive")
                                .header("Accept", "application/json")
                                .send(),
                        )
                        .await;
                        match result {
                            Ok(Ok(resp)) if resp.status().is_success() => {
                                if let Ok(body_bytes) = resp.bytes().await {
                                    let body =
                                        String::from_utf8_lossy(&body_bytes).into_owned();
                                    // Must be JSON array or object
                                    let trimmed = body.trim();
                                    if trimmed.starts_with('[') || trimmed.starts_with('{') {
                                        if let Ok(json) =
                                            serde_json::from_str::<serde_json::Value>(&body)
                                        {
                                            let mut flat_entries = Vec::new();
                                            recursive_extract_json(
                                                &json,
                                                &mut flat_entries,
                                                String::new(),
                                                &host_ref,
                                                &token_ref,
                                            );
                                            if !flat_entries.is_empty() {
                                                return Some((ep_idx, endpoint, flat_entries));
                                            }
                                        }
                                    }
                                }
                            }
                            Ok(Ok(resp)) => {
                                println!(
                                    "[DragonForce Phase 123] Endpoint {} returned status {}",
                                    ep_idx,
                                    resp.status().as_u16()
                                );
                            }
                            Ok(Err(e)) => {
                                println!(
                                    "[DragonForce Phase 123] Endpoint {} error: {}",
                                    ep_idx, e
                                );
                            }
                            Err(_) => {
                                println!(
                                    "[DragonForce Phase 123] Endpoint {} timed out",
                                    ep_idx
                                );
                            }
                        }
                        None
                    });
                }

                // Collect first winner — with a hard outer timeout to prevent
                // indefinite blocking when Arti's `.send()` future isn't fully
                // cancellation-safe (the inner 20s timeout fires but the task stays alive).
                let mut search_flattened = false;
                let search_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
                loop {
                    let next = tokio::time::timeout_at(
                        search_deadline,
                        search_joinset.join_next(),
                    ).await;
                    match next {
                        Ok(Some(Ok(Some((ep_idx, endpoint, flat_entries))))) => {
                            search_joinset.abort_all(); // Cancel remaining probes
                            let _ = app.emit(
                                "log",
                                format!(
                                    "[DragonForce] Phase 123: ✅ Tree flattening SUCCESS via endpoint #{} — {} entries discovered in single request!",
                                    ep_idx,
                                    flat_entries.len()
                                ),
                            );
                            println!(
                                "[DragonForce Phase 123] ✅ Tree flattening via {} — {} entries",
                                endpoint, flat_entries.len()
                            );

                            // Inject flattened entries into discovered set and UI
                            for entry in &flat_entries {
                                let _ = ui_tx.send(entry.clone()).await;
                                // Don't queue folders for BFS — we already have everything
                            }
                            for entry in flat_entries {
                                all_discovered_entries.push(entry);
                            }
                            search_flattened = true;
                            break;
                        }
                        Ok(Some(_)) => {
                            // Task returned None or JoinError — continue to next
                            continue;
                        }
                        Ok(None) => {
                            // All tasks completed without a winner
                            break;
                        }
                        Err(_) => {
                            // Hard outer timeout — Arti futures hung past inner timeout
                            println!(
                                "[DragonForce Phase 123] Search probe collection timed out (30s hard limit) — aborting remaining tasks"
                            );
                            search_joinset.abort_all();
                            break;
                        }
                    }
                }

                if search_flattened {
                    let _ = app.emit(
                        "log",
                        "[DragonForce] Phase 123: Tree flattening complete. Skipping recursive BFS."
                            .to_string(),
                    );
                    // Return immediately — no need for BFS workers
                    drop(ui_tx);
                    return Ok(all_discovered_entries.drain_all());
                }

                // No search endpoint hit — fall through to normal BFS
                let _ = app.emit(
                    "log",
                    "[DragonForce] Phase 123: No search API found. Proceeding with normal BFS crawl."
                        .to_string(),
                );
                println!("[DragonForce Phase 123] No search API found — continuing normal BFS");
            }
        }

        // ═══════════════════════════════════════════════════════════
        // Phase 126: Shared CircuitHealth module (extracted from Phase 122→124)
        // ═══════════════════════════════════════════════════════════
        // Uses crate::circuit_health::CircuitHealth — 12 bytes/circuit,
        // lock-free CAS atomics, EWMA + CUSUM + latency tracking.
        // See circuit_health.rs for full documentation and unit tests.
        use crate::circuit_health::CircuitHealth;

        let circuit_health: Arc<Vec<CircuitHealth>> = Arc::new(
            (0..multi_pool.clients_count().max(1))
                .map(|_| CircuitHealth::new())
                .collect()
        );

        // ═══════════════════════════════════════════════════════════
        // Phase 122: Pre-built ArtiClient Pool (P0 — connection reuse)
        // ═══════════════════════════════════════════════════════════
        // Pre-build one ArtiClient per pool slot so workers swap
        // index on rotation instead of destroying the hyper pool.
        // This preserves keep-alive connections across circuit switches.
        // ═══════════════════════════════════════════════════════════
        let pool_size = multi_pool.clients_count().max(1);
        let mut prebuilt_clients = Vec::with_capacity(pool_size);
        for slot in 0..pool_size {
            let tor_arc = multi_pool.get_client(slot).await;
            prebuilt_clients.push(crate::arti_client::ArtiClient::new((*tor_arc).clone(), None));
        }
        let prebuilt_clients = Arc::new(prebuilt_clients);

        // Start with min(12, max) workers to avoid idle spin;
        // more workers don't help when queue is small.
        let initial_workers = max_concurrent.min(12);
        let mut workers = tokio::task::JoinSet::new();

        for worker_idx in 0..initial_workers {
            let f = frontier.clone();
            let pool_clone = multi_pool.clone();
            let q_clone = queue.clone();
            let retry_q = retry_queue.clone();
            let ui_tx_clone = ui_tx.clone();
            let discovered_ref = all_discovered_entries.clone();
            let pending_clone = pending.clone();
            let app_clone = app.clone();
            let health_ref = circuit_health.clone();
            let clients_ref = prebuilt_clients.clone();

            workers.spawn(async move {
                let pool_size = pool_clone.clients_count().max(1);
                // Pick initial circuit: prefer healthiest EWMA score
                let mut client_slot = {
                    let mut best_slot = worker_idx % pool_size;
                    let mut best_score: f32 = -1.0;
                    for i in 0..pool_size {
                        let idx = (worker_idx + i) % pool_size;
                        let sc = health_ref[idx].score();
                        if sc > best_score {
                            best_score = sc;
                            best_slot = idx;
                        }
                    }
                    best_slot
                };
                // Phase 122 P0: Use pre-built ArtiClient — no pool destruction
                let mut reusable_client = clients_ref[client_slot].clone();

                let mut request_count: u32 = 0;
                // Phase 126: CUSUM graduated backoff counter.
                // When ALL slots are dead, back off exponentially (2s, 4s, 8s, 16s)
                // to avoid wasting timeout budget during prolonged HS degradation.
                let mut consecutive_all_dead: u32 = 0;

                loop {
                    if f.is_cancelled() {
                        break;
                    }

                    // ── Pop from primary queue first, then check retry queue ──
                    let (next_url, is_retry, retry_attempt) = if let Some(url) = q_clone.pop() {
                        (url, false, 0u8)
                    } else if let Some(payload) = retry_q.pop() {
                        if std::time::Instant::now() >= payload.unlock_at {
                            (payload.url, true, payload.attempt)
                        } else {
                            // Not ready yet → push back and wait briefly
                            retry_q.push(payload);
                            if pending_clone.load(std::sync::atomic::Ordering::SeqCst) == 0
                                && retry_q.is_empty()
                            {
                                break;
                            }
                            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                            continue;
                        }
                    } else {
                        if pending_clone.load(std::sync::atomic::Ordering::SeqCst) == 0 {
                            break;
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                        continue;
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
                    // Only count as pending if from primary queue (retry items
                    // were already counted when they were first discovered)
                    let _guard = if !is_retry {
                        Some(TaskGuard {
                            counter: pending_clone.clone(),
                        })
                    } else {
                        None
                    };

                    let dynamic_host = if let Ok(u) = url::Url::parse(&next_url) {
                        u.host_str().unwrap_or("").to_string()
                    } else {
                        String::new()
                    };

                    // ── Strategy 4: JWT expiry check ──
                    if let Some(exp) = extract_jwt_expiry(&next_url) {
                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs();
                        if exp <= now + 30 {
                            continue; // JWT expired — skip silently
                        }
                    }

                    let _permit = f.politeness_semaphore.acquire().await.ok();
                    let (cid, _) = f.get_client();

                    let start_time = std::time::Instant::now();
                    request_count += 1;

                    // ── Phase 120→124: Adaptive timeout with circuit-latency awareness ──
                    // First 2 requests: 45s (HS descriptor lookup). Then:
                    // Phase 124 P1: Use adaptive TTFB from EWMA latency tracking.
                    // max(3 × ewma_latency_ms, 5000ms) capped at 25000ms for warm circuits.
                    let fetch_timeout_secs = if request_count <= 2 {
                        45
                    } else {
                        // Adaptive TTFB: converts ms → seconds, rounds up
                        let adaptive_ms = health_ref[client_slot].adaptive_ttfb_ms();
                        let timeout = ((adaptive_ms + 999) / 1000).max(5) as u64;
                        // Phase 125: Log adaptive TTFB convergence for validation
                        if request_count % 10 == 0 {
                            println!(
                                "[DragonForce W{} TTFB] slot={} ewma={}ms timeout={}s",
                                worker_idx, client_slot, adaptive_ms, timeout
                            );
                        }
                        timeout
                    };

                    // ── Phase 124→126: CUSUM + Periodic + Graduated Backoff ──
                    // CUSUM fires IMMEDIATELY on sudden degradation (~3 consecutive failures).
                    // Periodic check still runs every 15 requests as a safety net.
                    // Phase 126: When ALL slots are dead, graduated backoff prevents
                    //   wasting timeout budget hammering unresponsive circuits.
                    let cusum_fired = health_ref[client_slot].cusum_triggered();
                    let periodic_check = request_count > 0 && request_count % 15 == 0;
                    let should_repin = cusum_fired || periodic_check;

                    if should_repin {
                        let current_score = health_ref[client_slot].score();
                        // Phase 125: Log CUSUM/periodic trigger
                        if cusum_fired {
                            println!(
                                "[DragonForce W{} CUSUM] TRIGGERED on slot={} score={:.2} — evaluating repin",
                                worker_idx, client_slot, current_score
                            );
                        }

                        // Phase 126: Check if ALL slots are dead
                        if crate::circuit_health::all_slots_dead(&health_ref, 0.05) {
                            consecutive_all_dead += 1;
                            let backoff_secs = 2u64.pow(consecutive_all_dead.min(4)); // 2, 4, 8, 16s
                            println!(
                                "[DragonForce W{} BACKOFF] All slots dead — sleeping {}s (cycle {})",
                                worker_idx, backoff_secs, consecutive_all_dead
                            );
                            tokio::time::sleep(std::time::Duration::from_secs(backoff_secs)).await;
                            // Still try to rotate to a different slot for diversity
                            client_slot = (client_slot + 1) % pool_size;
                            reusable_client = clients_ref[client_slot].clone();
                        } else {
                            // Normal repin logic: find best slot
                            consecutive_all_dead = 0;
                            let (best_slot_idx, best_score) = crate::circuit_health::best_slot(&health_ref);
                            // Repin if current circuit score is poor (<0.3) or much worse than best
                            if best_slot_idx != client_slot && (current_score < 0.3 || best_score > current_score * 1.5) {
                                println!(
                                    "[DragonForce W{} REPIN] slot {} (score={:.2}) → slot {} (score={:.2}) trigger={}",
                                    worker_idx, client_slot, current_score, best_slot_idx, best_score,
                                    if cusum_fired { "CUSUM" } else { "periodic" }
                                );
                                client_slot = best_slot_idx;
                                reusable_client = clients_ref[client_slot].clone();
                                // Reset CUSUM on the NEW circuit — fresh slate
                                health_ref[best_slot_idx].reset_cusum();
                            }
                        }
                    } else {
                        // Healthy request — reset backoff counter
                        consecutive_all_dead = 0;
                    }

                    // ── Phase 122 P3: Connection keep-alive on all worker requests ──
                    let req = reusable_client.get(&next_url)
                        .header("Connection", "keep-alive")
                        .send();
                    let fetch_result =
                        tokio::time::timeout(std::time::Duration::from_secs(fetch_timeout_secs), req).await;

                    match fetch_result {
                        Ok(Ok(resp)) => {
                            let status = resp.status().as_u16();
                            let elapsed = start_time.elapsed();
                            println!(
                                "[DragonForce W{}] GET {} → {} ({:.1}s)",
                                worker_idx, &next_url[..next_url.len().min(100)], status, elapsed.as_secs_f64()
                            );

                            // ── Strategy 4: Failure Classification ──
                            if status == 404 {
                                // Dead path — skip permanently
                                continue;
                            }
                            if status == 403 || status == 401 {
                                // JWT expired or access denied — no point retrying
                                f.record_failure(cid);
                                continue;
                            }
                            if status == 429 || status == 503 {
                                // Server overloaded → re-queue with backoff
                                f.record_failure(cid);
                                retry_q.push(RetryPayload {
                                    url: next_url,
                                    attempt: 1,
                                    unlock_at: std::time::Instant::now()
                                        + std::time::Duration::from_secs(5),
                                });
                                continue;
                            }

                            if resp.status().is_success() {
                                // Phase 122 P2: Use bytes() instead of text() — move UTF-8
                                // decode into spawn_blocking to free the async executor.
                                if let Ok(Ok(body_bytes)) = tokio::time::timeout(
                                    std::time::Duration::from_secs(20),
                                    resp.bytes(),
                                )
                                .await
                                {
                                    let bytes_downloaded = body_bytes.len() as u64;
                                    let elapsed_ms = start_time.elapsed().as_millis() as u64;
                                    f.record_success(cid, bytes_downloaded, elapsed_ms);

                                    // Phase 122 P1: EWMA circuit health update
                                    health_ref[client_slot].record_success();
                                    // Phase 124 P1: Record latency for adaptive TTFB
                                    health_ref[client_slot].record_latency(elapsed_ms as f32);

                                    // ── Phase 124 P2: Size-gated parse ──
                                    // Bodies <4KB: inline decode (saves ~5µs spawn overhead).
                                    // Bodies ≥4KB: spawn_blocking (prevents executor stall).
                                    let dyn_host_clone = dynamic_host.clone();
                                    let curr_url_clone = next_url.clone();
                                    let new_files = if body_bytes.len() < 4096 {
                                        // Inline path — trivial decode cost for small responses
                                        let body = String::from_utf8_lossy(&body_bytes);
                                        parse_dragonforce_fsguest(
                                            &body,
                                            &dyn_host_clone,
                                            &curr_url_clone,
                                        )
                                    } else {
                                        // Blocking path — offload large decode + parse
                                        tokio::task::spawn_blocking(move || {
                                            let body = String::from_utf8_lossy(&body_bytes);
                                            parse_dragonforce_fsguest(
                                                &body,
                                                &dyn_host_clone,
                                                &curr_url_clone,
                                            )
                                        })
                                        .await
                                        .unwrap_or_default()
                                    };

                                    if new_files.is_empty() {
                                        println!(
                                            "[DragonForce W{}] Parser returned 0 entries from {} bytes of HTML",
                                            worker_idx, bytes_downloaded
                                        );
                                    } else {
                                        println!(
                                            "[DragonForce W{}] Parsed {} entries (files/folders) from {} bytes",
                                            worker_idx, new_files.len(), bytes_downloaded
                                        );
                                    }

                                    for doc in &new_files {
                                        if doc.entry_type == EntryType::Folder
                                            && f.mark_visited(&doc.raw_url)
                                        {
                                            pending_clone
                                                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                                            q_clone.push(doc.raw_url.clone());
                                            if doc.path == "/_bridge" {
                                                use tauri::Emitter;
                                                let _ = app_clone.emit("log", format!(
                                                    "[DragonForce] Bridge redirect detected → {}",
                                                    &doc.raw_url[..doc.raw_url.len().min(120)]
                                                ));
                                                println!(
                                                    "[DragonForce W{}] Bridge redirect queued → {}",
                                                    worker_idx, &doc.raw_url[..doc.raw_url.len().min(120)]
                                                );
                                            }
                                        }
                                        let _ = ui_tx_clone.send(doc.clone()).await;
                                    }

                                    if !new_files.is_empty() {
                                        for nf in new_files {
                                            discovered_ref.push(nf);
                                        }
                                    }
                                } else {
                                    // Body read timeout → re-queue
                                    f.record_failure(cid);
                                    retry_q.push(RetryPayload {
                                        url: next_url,
                                        attempt: 1,
                                        unlock_at: std::time::Instant::now()
                                            + std::time::Duration::from_secs(2),
                                    });
                                }
                            } else {
                                // Other non-success status
                                f.record_failure(cid);

                            }
                        }
                        Ok(Err(e)) => {
                            // HTTP client error (connection refused, DNS, etc.)
                            let elapsed = start_time.elapsed();
                            println!(
                                "[DragonForce W{}] Request error after {:.1}s: {}",
                                worker_idx, elapsed.as_secs_f64(), e
                            );
                            {
                                use tauri::Emitter;
                                let err_str = e.to_string();
                                let truncated = if err_str.len() > 60 { &err_str[..60] } else { &err_str };
                                let _ = app_clone.emit("log", format!(
                                    "[DragonForce W{}] Connect error ({:.1}s, attempt {}/8): {}",
                                    worker_idx, elapsed.as_secs_f64(), retry_attempt + 1, truncated
                                ));
                            }
                            f.record_failure(cid);
                            let next_attempt = retry_attempt + 1;
                            // Phase 120: Aggressive retry (8 attempts) across different circuits
                            // for onion HS that may be slow but not dead
                            if next_attempt < 8 {
                                retry_q.push(RetryPayload {
                                    url: next_url,
                                    attempt: next_attempt,
                                    unlock_at: std::time::Instant::now()
                                        + std::time::Duration::from_secs(2),
                                });
                            } else {
                                use tauri::Emitter;
                                let _ = app_clone.emit("log", format!(
                                    "[DragonForce] Exhausted all 8 retry attempts for URL"
                                ));
                            }
                            // Phase 122 P1: EWMA failure + P0: swap pre-built client
                            health_ref[client_slot].record_failure();
                            client_slot = {
                                let mut best = (client_slot + 1) % pool_size;
                                let mut best_sc: f32 = -1.0;
                                for i in 1..=pool_size {
                                    let idx = (client_slot + i) % pool_size;
                                    let sc = health_ref[idx].score();
                                    if sc > best_sc {
                                        best_sc = sc;
                                        best = idx;
                                    }
                                }
                                best
                            };
                            // Phase 122 P0: Swap to pre-built client (preserves hyper pool)
                            reusable_client = clients_ref[client_slot].clone();

                        }
                        Err(_) => {
                            // ── Strategy 1: Inverted Retry — timeout → re-queue ──
                            let elapsed = start_time.elapsed();
                            println!(
                                "[DragonForce W{}] TIMEOUT after {:.1}s on {}",
                                worker_idx, elapsed.as_secs_f64(), &next_url[..next_url.len().min(100)]
                            );
                            {
                                use tauri::Emitter;
                                let _ = app_clone.emit("log", format!(
                                    "[DragonForce W{}] Timeout ({:.0}s, attempt {}/8). Rotating circuit...",
                                    worker_idx, elapsed.as_secs_f64(), retry_attempt + 1
                                ));
                            }
                            f.record_failure(cid);
                            let next_attempt = retry_attempt + 1;
                            // Phase 120: Allow up to 8 retry attempts for timeouts
                            if next_attempt < 8 {
                                let backoff_secs: u64 = match next_attempt {
                                    1 => 2,
                                    2 => 3,
                                    _ => 5,
                                };
                                retry_q.push(RetryPayload {
                                    url: next_url,
                                    attempt: next_attempt,
                                    unlock_at: std::time::Instant::now()
                                        + std::time::Duration::from_secs(backoff_secs),
                                });
                            } else {
                                use tauri::Emitter;
                                let _ = app_clone.emit("log", format!(
                                    "[DragonForce] Exhausted all 8 retry attempts for URL"
                                ));
                            }
                            // Phase 122 P1: EWMA failure + P0: swap pre-built client
                            health_ref[client_slot].record_failure();
                            client_slot = {
                                let mut best = (client_slot + 1) % pool_size;
                                let mut best_sc: f32 = -1.0;
                                for i in 1..=pool_size {
                                    let idx = (client_slot + i) % pool_size;
                                    let sc = health_ref[idx].score();
                                    if sc > best_sc {
                                        best_sc = sc;
                                        best = idx;
                                    }
                                }
                                best
                            };
                            // Phase 122 P0: Swap to pre-built client (preserves hyper pool)
                            reusable_client = clients_ref[client_slot].clone();

                        }
                    }
                }
            });
        }

        while workers.join_next().await.is_some() {}

        drop(ui_tx);
        let final_results = all_discovered_entries.drain_all();
        Ok(final_results)
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
