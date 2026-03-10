use std::sync::Arc;
use tauri::AppHandle;
use tokio::sync::mpsc;

#[derive(Default)]
pub struct WorldLeaksAdapter;

#[async_trait::async_trait]
impl super::CrawlerAdapter for WorldLeaksAdapter {
    async fn can_handle(&self, fingerprint: &super::SiteFingerprint) -> bool {
        fingerprint.body.contains("worldleaks")
            || fingerprint.body.contains("<app-root></app-root>")
    }

    async fn crawl(
        &self,
        current_url: &str,
        frontier: Arc<crate::frontier::CrawlerFrontier>,
        app: AppHandle,
    ) -> anyhow::Result<Vec<super::FileEntry>> {
        use tauri::Emitter;

        // 1. Setup channels
        let queue = Arc::new(crossbeam_queue::SegQueue::new());

        let all_discovered_entries = Arc::new(tokio::sync::Mutex::new(Vec::new()));

        // Pending counter to fix BFS race conditions
        let pending = Arc::new(std::sync::atomic::AtomicUsize::new(0));

        // 2. Initial seed
        queue.push(current_url.to_string());
        frontier.mark_visited(current_url);
        pending.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        // 3. Batched UI Backpressure Task
        let (ui_tx, mut ui_rx) = mpsc::channel::<super::FileEntry>(20000);
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
                    else => break, // Channel closed
                }
            }
        });

        // 4. Thread Pool config
        let max_concurrent = frontier.recommended_listing_workers();
        let mut workers = tokio::task::JoinSet::new();

        for _ in 0..max_concurrent {
            let f = frontier.clone();
            let q_clone = queue.clone();
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

                    // Advanced Politeness Throttle
                    let _permit = f.politeness_semaphore.acquire().await.ok();
                    let (cid, _client) = f.get_client();
                    let start_time = std::time::Instant::now();

                    let mut fetch_success = false;
                    let mut bytes_downloaded = 0;
                    let mut html = String::new();
                    let mut active_cid = cid;
                    let mut ddos_guard = crate::adapters::qilin_ddos_guard::DdosGuard::new();

                    for _ in 0..4 {
                        let (current_cid, current_client) = f.get_client();
                        active_cid = current_cid;

                        let req = current_client.get(&next_url).send();
                        if let Ok(Ok(resp)) =
                            tokio::time::timeout(std::time::Duration::from_secs(45), req).await
                        {
                            if let Some(delay) =
                                ddos_guard.record_response_legacy(resp.status().as_u16())
                            {
                                tokio::time::sleep(delay).await;
                            }

                            println!(
                                "[DEBUG WORLDLEAKS] Fetch status for {}: {}",
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
                                "[DEBUG WORLDLEAKS] Timeout or client error connecting to {}",
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
                        let html_len = html.len();
                        let start_idx = html_len.saturating_sub(4000);
                        println!(
                            "[DEBUG WORLDLEAKS PARSER] Raw HTML end ({} bytes): {}",
                            html_len,
                            &html[start_idx..]
                        );

                        // We will add the actual HTML parsing here later
                    }
                }
            });
        }

        while workers.join_next().await.is_some() {}

        drop(ui_tx); // Signals the UI batcher to flush and shutdown
        let mut final_results = all_discovered_entries.lock().await;
        Ok(final_results.drain(..).collect())
    }

    fn name(&self) -> &'static str {
        "WorldLeaks SPA"
    }
}
