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
        let max_concurrent = 120; // Will be scaled by AIMD + Politeness later
        let mut workers = tokio::task::JoinSet::new();

        for _ in 0..max_concurrent {
            let f = frontier.clone();
            let q_clone = queue.clone();
            let ui_tx_clone = ui_tx.clone();
            let discovered_ref = all_discovered_entries.clone();
            let pending_clone = pending.clone();

            workers.spawn(async move {
                loop {
                    if f.is_cancelled() { break; }

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

                        // Advanced Politeness Throttle
                        let _permit = f.politeness_semaphore.acquire().await.ok();

                        // Round Robin Persistent Client
                        let (cid, _client) = f.get_client();

                        let start_time = std::time::Instant::now();

                        // Emulated network fetch
                        tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;

                        f.record_success(cid, 1024, start_time.elapsed().as_millis() as u64);

                        // Emulating parsed discoveries (In reality we client.get(&next_url).await...)
                        let mut new_files = Vec::new();
                        let items_to_find = rand::random::<u8>() % 5;

                        for i in 0..items_to_find {
                            let dummy_url = format!("{next_url}/node_{i}");
                            if f.mark_visited(&dummy_url) {
                                // Add back to queue to recurse inside
                                pending_clone.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                                q_clone.push(dummy_url.clone());

                                let entry = super::FileEntry {
                                    path: format!(
                                        "/Target Server/{}",
                                        dummy_url.replace("http://", "")
                                    ),
                                    size_bytes: Some(1024 * (i as u64 + 1)),
                                    entry_type: super::EntryType::File,
                                    raw_url: dummy_url,
                                };
                                new_files.push(entry);
                            }
                        }

                        // Send to IPC batcher
                        for file in &new_files {
                            let _ = ui_tx_clone.send(file.clone()).await;
                        }

                        // Save internally
                        if !new_files.is_empty() {
                            let mut lock = discovered_ref.lock().await;
                            lock.extend(new_files);
                        }

                        pending_clone.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                }
            });
        }

        while let Some(_) = workers.join_next().await {}

        drop(ui_tx); // Signals the UI batcher to flush and shutdown
        let mut final_results = all_discovered_entries.lock().await;
        Ok(final_results.drain(..).collect())
    }

    fn name(&self) -> &'static str {
        "WorldLeaks SPA"
    }
}
