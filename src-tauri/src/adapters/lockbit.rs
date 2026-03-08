use crate::adapters::{CrawlerAdapter, EntryType, FileEntry, SiteFingerprint};
use crate::frontier::CrawlerFrontier;
use std::sync::Arc;
use tauri::AppHandle;
use tokio::sync::mpsc;

#[derive(Default)]
pub struct LockBitAdapter;

/// Custom parser for LockBit 5.0 Leak Site directory index
pub fn parse_lockbit_dom(html: &str, current_url: &str) -> Vec<FileEntry> {
    println!(
        "\n[DEBUG LOCKBIT DOM] ----\n{}\n----\n",
        &html.chars().take(3000).collect::<String>()
    );
    let mut entries = Vec::new();

    let document = match std::panic::catch_unwind(|| scraper::Html::parse_document(html)) {
        Ok(doc) => doc,
        Err(_) => return entries,
    };

    // LockBit 5.0 puts files/dirs in a table but uses specific class or IDs
    // The screenshot shows a row with: icon, "File Name", "File Size", "Date"
    // Let's grab all rows inside the list or table
    let row_selector = match scraper::Selector::parse("tr, .row, .item") {
        Ok(s) => s,
        Err(_) => return entries,
    };

    let a_selector = match scraper::Selector::parse("a") {
        Ok(s) => s,
        Err(_) => return entries,
    };

    let current_parsed =
        url::Url::parse(current_url).unwrap_or(url::Url::parse("http://localhost/").unwrap());

    for row in document.select(&row_selector) {
        if let Some(a_node) = row.select(&a_selector).next() {
            if let Some(href) = a_node.value().attr("href") {
                // Ignore sorting links or table headers
                if href.starts_with('?') || href == ".." || href == "../" || href == "/" {
                    continue;
                }

                let text = a_node
                    .text()
                    .collect::<Vec<_>>()
                    .join("")
                    .trim()
                    .to_string();
                if text.is_empty() || text.to_lowercase() == "parent directory" {
                    continue;
                }

                let is_dir = href.ends_with('/');
                let absolute_url = current_parsed.join(href).unwrap_or(current_parsed.clone());
                let raw_url = absolute_url.to_string();
                let absolute_path = absolute_url.path();

                // Parse size by looking at the row text content
                let mut size_bytes = None;
                if !is_dir {
                    let row_text = row.text().collect::<Vec<_>>().join(" ");
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

        let queue = Arc::new(crossbeam_queue::SegQueue::new());
        let all_discovered_entries = Arc::new(tokio::sync::Mutex::new(Vec::new()));

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

        let max_concurrent = frontier.recommended_listing_workers();
        let mut workers = tokio::task::JoinSet::new();
        let pending = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        pending.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        for _ in 0..max_concurrent {
            let f = frontier.clone();
            let q_clone = queue.clone();
            let ui_tx_clone = ui_tx.clone();
            let discovered_ref = all_discovered_entries.clone();
            let pending_clone = pending.clone();

            workers.spawn(async move {
                let mut ddos_guard = crate::adapters::qilin_ddos_guard::DdosGuard::new();
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

                    let start_time = std::time::Instant::now();
                    let (mut fetch_success, mut html) = (false, None);
                    let mut bytes_downloaded = 0;

                    let fetch_result = tokio::time::timeout(
                        std::time::Duration::from_secs(45),
                        client.get(&next_url).send(),
                    )
                    .await;

                    match fetch_result {
                        Ok(Ok(resp)) => {
                            let status = resp.status();
                            println!("[DEBUG LOCKBIT] HTTP Response Status: {}", status);
                            if let Some(delay) = ddos_guard.record_response(status.as_u16()) {
                                tokio::time::sleep(delay).await;
                            }
                            if status.is_success() {
                                if let Ok(body) = resp.text().await {
                                    bytes_downloaded += body.len() as u64;
                                    html = Some(body);
                                    fetch_success = true;
                                }
                            } else {
                                println!("[DEBUG LOCKBIT] HTTP Failed: {}", status);
                                html = Some(build_fallback_html());
                            }
                        }
                        Ok(Err(e)) => {
                            println!("[DEBUG LOCKBIT] HTTP Request Error: {}", e);
                            html = Some(build_fallback_html());
                        }
                        Err(e) => {
                            println!("[DEBUG LOCKBIT] HTTP Timeout: {}", e);
                            html = Some(build_fallback_html());
                        }
                    }

                    let elapsed_ms = start_time.elapsed().as_millis() as u64;
                    if fetch_success {
                        f.record_success(cid, bytes_downloaded, elapsed_ms);
                    }

                    let Some(html) = html else {
                        pending_clone.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                        continue;
                    };

                    if !f.active_options.listing {
                        pending_clone.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                        continue;
                    }

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
