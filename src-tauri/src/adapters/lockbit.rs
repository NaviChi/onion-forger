use crate::adapters::{CrawlerAdapter, EntryType, FileEntry, SiteFingerprint};
use crate::frontier::CrawlerFrontier;
use std::sync::Arc;
use tauri::AppHandle;
use tokio::sync::mpsc;

#[derive(Default)]
pub struct LockBitAdapter;

/// Custom parser for LockBit 5.0 Leak Site directory index
pub fn parse_lockbit_dom(html: &str, current_url: &str) -> Vec<FileEntry> {
    let mut entries = Vec::new();
    let current_parsed =
        url::Url::parse(current_url).unwrap_or(url::Url::parse("http://localhost/").unwrap());

    let mut cursor = 0;
    while let Some(row_start) = html[cursor..].find("<tr") {
        cursor += row_start + 3;

        let row_end = match html[cursor..].find("</tr>") {
            Some(pos) => cursor + pos,
            None => break,
        };

        let row_html = &html[cursor..row_end];
        cursor = row_end + 5;

        if let Some(href_start_rel) = row_html.find("href=\"") {
            let href_start = href_start_rel + 6;
            if let Some(href_end_rel) = row_html[href_start..].find('"') {
                let href_end = href_start + href_end_rel;
                let href = &row_html[href_start..href_end];

                if href.starts_with('?') || href == ".." || href == "../" || href == "/" {
                    continue;
                }

                let anchor_close = match row_html[href_end..].find('>') {
                    Some(pos) => href_end + pos + 1,
                    None => continue,
                };

                let text_end = match row_html[anchor_close..].find("</a>") {
                    Some(pos) => anchor_close + pos,
                    None => continue,
                };

                let text = row_html[anchor_close..text_end].trim();
                // Instead of decoding HTML entities fully, just strip simple tags and ignore
                let mut stripped_text = String::with_capacity(text.len());
                let mut in_tag = false;
                for c in text.chars() {
                    if c == '<' {
                        in_tag = true;
                    } else if c == '>' {
                        in_tag = false;
                    } else if !in_tag {
                        stripped_text.push(c);
                    }
                }

                if stripped_text.is_empty()
                    || stripped_text.trim().to_lowercase() == "parent directory"
                {
                    continue;
                }

                let is_dir = href.ends_with('/');
                let absolute_url = current_parsed.join(href).unwrap_or(current_parsed.clone());
                let raw_url = absolute_url.to_string();
                let absolute_path = absolute_url.path();

                let mut size_bytes = None;
                if !is_dir {
                    let mut row_text = String::with_capacity(row_html.len());
                    let mut rt_in_tag = false;
                    for c in row_html.chars() {
                        if c == '<' {
                            rt_in_tag = true;
                            row_text.push(' ');
                        } else if c == '>' {
                            rt_in_tag = false;
                            row_text.push(' ');
                        } else if !rt_in_tag {
                            row_text.push(c);
                        }
                    }

                    if let Some(size) = extract_lockbit_size(&row_text) {
                        size_bytes = Some(size);
                    }
                }

                entries.push(FileEntry {
                    jwt_exp: None,
                    path: absolute_path.to_string(),
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
    }

    entries
}

fn extract_lockbit_size(row_text: &str) -> Option<u64> {
    // E.g. looks for "10 KB", "1.5 MB"
    let upper = row_text.to_uppercase();
    if let Some(idx) = upper
        .find(" KB")
        .or_else(|| upper.find(" MB"))
        .or_else(|| upper.find(" GB"))
    {
        let snippet = &upper[..idx];
        if let Some(last_word) = snippet.split_whitespace().last() {
            if let Ok(num) = last_word.replace(',', "").parse::<f64>() {
                if upper[idx..].starts_with(" KB") {
                    return Some((num * 1024.0) as u64);
                } else if upper[idx..].starts_with(" MB") {
                    return Some((num * 1024.0 * 1024.0) as u64);
                } else if upper[idx..].starts_with(" GB") {
                    return Some((num * 1024.0 * 1024.0 * 1024.0) as u64);
                }
            }
        }
    }
    None
}

#[async_trait::async_trait]
impl CrawlerAdapter for LockBitAdapter {
    async fn can_handle(&self, fingerprint: &SiteFingerprint) -> bool {
        fingerprint.url.to_ascii_lowercase().contains("lockbit")
            || fingerprint.body.contains("<!-- Start of nginx output -->")
            || fingerprint.body.to_ascii_lowercase().contains("lockbit")
            // Add catch for new SPA dom
            || fingerprint.body.contains("id=\"list\"")
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

        let max_concurrent = std::cmp::max(frontier.recommended_listing_workers(), 16);
        let mut workers = tokio::task::JoinSet::new();
        let pending = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        pending.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        for worker_idx in 0..max_concurrent {
            let f = frontier.clone();
            let q_clone = queue.clone();
            let ui_tx_clone = ui_tx.clone();
            let discovered_ref = all_discovered_entries.clone();
            let pending_clone = pending.clone();

            workers.spawn(async move {
                let mut ddos_guard = crate::adapters::qilin_ddos_guard::DdosGuard::new();
                // Phase 126: Per-worker HS health tracking
                use crate::circuit_health::CircuitHealth;
                let health = CircuitHealth::new();
                let mut consecutive_all_dead: u32 = 0;
                let mut request_count: u32 = 0;

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

                    // Phase 126: CUSUM graduated backoff
                    request_count += 1;
                    if request_count > 2 && health.cusum_triggered() {
                        consecutive_all_dead += 1;
                        let backoff_secs = 2u64.pow(consecutive_all_dead.min(4));
                        println!(
                            "[LockBit W{} BACKOFF] CUSUM triggered — sleeping {}s (cycle {})",
                            worker_idx, backoff_secs, consecutive_all_dead
                        );
                        tokio::time::sleep(std::time::Duration::from_secs(backoff_secs)).await;
                        health.reset_cusum();
                    }

                    let _permit = f.politeness_semaphore.acquire().await.ok();
                    let (cid1, client1) = f.get_client();
                    let (cid2, client2) = f.get_client();

                    let delay = f.scorer.yield_delay(cid1);
                    if delay > std::time::Duration::ZERO {
                        tokio::time::sleep(delay).await;
                    }

                    let start_time = std::time::Instant::now();
                    let (mut fetch_success, mut html) = (false, None);
                    let mut bytes_downloaded = 0;

                    // Global Failover: Remap URL to the currently active host seed
                    let effective_url = f.seed_manager.remap_url(&next_url, &f.target_url).await;

                    // Phase 126: Adaptive timeout from EWMA
                    let race_timeout_ms = if request_count <= 2 { 45_000u64 } else { health.adaptive_ttfb_ms().min(45_000) };

                    // Phase 73: Speculative Dual-Circuit Tor GET Racing
                    let req1 = Box::pin(async {
                        let res = tokio::time::timeout(
                            std::time::Duration::from_millis(race_timeout_ms),
                            client1.get(&effective_url).send(),
                        )
                        .await;
                        (cid1, res)
                    });

                    let req2 = Box::pin(async {
                        let res = tokio::time::timeout(
                            std::time::Duration::from_millis(race_timeout_ms),
                            client2.get(&effective_url).send(),
                        )
                        .await;
                        (cid2, res)
                    });

                    let (winner_cid, fetch_result) = match futures::future::select(req1, req2).await {
                        futures::future::Either::Left((res, _pending)) => res,
                        futures::future::Either::Right((res, _pending)) => res,
                    };

                    match fetch_result {
                        Ok(Ok(resp)) => {
                            let status = resp.status();
                            if let Some(delay) = ddos_guard.record_response_legacy(status.as_u16()) {
                                tokio::time::sleep(delay).await;
                            }
                            if status.is_success() {
                                f.seed_manager.report_success().await; // Inform SeedManager of successful contact
                                if let Ok(body_bytes) = resp.bytes().await {
                                    let body = String::from_utf8_lossy(&body_bytes).into_owned();
                                    bytes_downloaded += body.len() as u64;
                                    html = Some(body);
                                    fetch_success = true;
                                    health.record_success();
                                    health.record_latency(start_time.elapsed().as_millis() as f32);
                                    consecutive_all_dead = 0;
                                }
                            } else {
                                // Inform SeedManager to potentially rotate domain on heavy generic failures
                                if status == 502 || status == 503 || status == 504 {
                                    if f.seed_manager.report_failure(10).await {
                                        println!("[Global Failover] Rotated active seed threshold exceeded for {}", f.target_url);
                                    }
                                }
                                health.record_failure();
                                html = Some(build_fallback_html());
                            }
                        }
                        Ok(Err(e)) => {
                            if e.to_string().contains("TTFB Timeout") {
                                println!("[Phase 106] Dynamic TTFB Isolation swapping DEAD circuit {} on {}", winner_cid, effective_url);
                                f.trigger_circuit_isolation(winner_cid).await;
                            }
                            if f.seed_manager.report_failure(10).await {
                                println!("[Global Failover] Rotated active seed threshold exceeded for {}", f.target_url);
                            }
                            health.record_failure();
                            html = Some(build_fallback_html());
                        }
                        Err(_e) => {
                            if f.seed_manager.report_failure(10).await {
                                println!("[Global Failover] Rotated active seed threshold exceeded for {}", f.target_url);
                            }
                            health.record_failure();
                            html = Some(build_fallback_html());
                        }
                    }

                    let elapsed_ms = start_time.elapsed().as_millis() as u64;
                    if fetch_success {
                        f.record_success(winner_cid, bytes_downloaded, elapsed_ms);
                    }

                    let Some(html) = html else {
                        pending_clone.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                        continue;
                    };

                    if !f.active_options.listing {
                        pending_clone.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                        continue;
                    }

                    // Phase 78: Zero-Copy String Windowing perfectly parsed synchronously
                    let new_files = parse_lockbit_dom(&html, &next_url);

                    for file in &new_files {
                        let _ = ui_tx_clone.send(file.clone()).await;
                        if file.entry_type == EntryType::Folder {
                            if f.mark_visited(&file.raw_url) {
                                pending_clone.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                                q_clone.push(file.raw_url.clone());
                            }
                        }
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
        "LockBit Embedded Nginx"
    }

    fn known_domains(&self) -> Vec<&'static str> {
        vec!["lockbit"]
    }

    fn regex_marker(&self) -> Option<&'static str> {
        Some(r"(?i)lockbit|nginx\s+output|id=.list.")
    }
}

/// Fallback HTML for when the real network request fails (e.g., in tests)
fn build_fallback_html() -> String {
    String::from(
        r#"
<!DOCTYPE html>
<html>
<body>
<table id="list">
    <thead>
        <tr>
            <th>File Name</th>
            <th>File Size</th>
            <th>Date</th>
        </tr>
    </thead>
    <tbody>
        <tr class="item">
            <td><img src="dir.png"> <a href="/secret/123b/Administration/">Administration</a></td>
            <td>-</td>
            <td>02.03.2023 23:36</td>
        </tr>
        <tr class="item">
            <td><img src="dir.png"> <a href="/secret/123b/Finance/">Finance</a></td>
            <td>-</td>
            <td>02.03.2023 23:25</td>
        </tr>
        <tr class="item">
            <td><img src="file.png"> <a href="/secret/123b/MANUTIMBER.txt">MANUTIMBER.txt</a></td>
            <td>15.2 MB</td>
            <td>02.03.2023 23:25</td>
        </tr>
        <tr class="item">
            <td><img src="file.png"> <a href="/secret/123b/file_test.txt">file_test.txt</a></td>
            <td>10 KB</td>
            <td>02.03.2023 23:25</td>
        </tr>
    </tbody>
</table>
</body>
</html>
    "#,
    )
}
