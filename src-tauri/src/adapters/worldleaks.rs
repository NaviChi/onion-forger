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
        let queue = Arc::new(crate::spillover::SpilloverQueue::new());

        let all_discovered_entries = Arc::new(crate::spillover::SpilloverList::new());

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

        // Phase 119: Work-stealing + reduced timeout/retries
        let retry_q = Arc::new(crate::work_stealing::new_retry_queue());
        let max_concurrent = frontier.recommended_listing_workers().min(12);
        let mut workers = tokio::task::JoinSet::new();

        for worker_idx in 0..max_concurrent {
            let f = frontier.clone();
            let q_clone = queue.clone();
            let pending_clone = pending.clone();
            let rq = retry_q.clone();

            workers.spawn(async move {
                // Phase 126: Per-worker HS health tracking via shared CircuitHealth
                use crate::circuit_health::CircuitHealth;
                let health = CircuitHealth::new();
                let mut consecutive_all_dead: u32 = 0;
                let mut request_count: u32 = 0;

                loop {
                    if f.is_cancelled() {
                        break;
                    }

                    // Work-stealing: primary queue first, then retry queue
                    let (next_url, retry_attempt) = if let Some(url) = q_clone.pop() {
                        (url, 0u8)
                    } else if let Some((url, attempt)) = crate::work_stealing::try_pop_retry(&rq) {
                        (url, attempt)
                    } else {
                        if pending_clone.load(std::sync::atomic::Ordering::SeqCst) == 0
                            && rq.is_empty()
                        {
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
                    let _guard = if retry_attempt == 0 {
                        Some(TaskGuard {
                            counter: pending_clone.clone(),
                        })
                    } else {
                        None
                    };

                    // Phase 126: CUSUM graduated backoff
                    request_count += 1;
                    if request_count > 2 && health.cusum_triggered() {
                        consecutive_all_dead += 1;
                        let backoff_secs = 2u64.pow(consecutive_all_dead.min(4));
                        println!(
                            "[WorldLeaks W{} BACKOFF] CUSUM triggered — sleeping {}s (cycle {})",
                            worker_idx, backoff_secs, consecutive_all_dead
                        );
                        tokio::time::sleep(std::time::Duration::from_secs(backoff_secs)).await;
                        health.reset_cusum();
                    }

                    let _permit = f.politeness_semaphore.acquire().await.ok();
                    let (cid, client) = f.get_client();
                    let start_time = std::time::Instant::now();

                    // Phase 126: Adaptive timeout from EWMA latency
                    let timeout_ms = if request_count <= 2 { 20_000u64 } else { health.adaptive_ttfb_ms().min(20_000) };
                    let req = client.get(&next_url).send();
                    match tokio::time::timeout(std::time::Duration::from_millis(timeout_ms), req).await {
                        Ok(Ok(resp)) => {
                            let status = resp.status().as_u16();
                            let action = crate::work_stealing::classify_http_status(status);

                            if action != crate::work_stealing::FailureAction::Skip
                                && !resp.status().is_success()
                            {
                                f.record_failure(cid);
                                health.record_failure();
                                crate::work_stealing::handle_failure(
                                    &rq,
                                    next_url,
                                    retry_attempt,
                                    action,
                                );
                                continue;
                            }

                            if resp.status().is_success() {
                                if let Ok(Ok(body_bytes)) = tokio::time::timeout(
                                    std::time::Duration::from_secs(20),
                                    resp.bytes(),
                                )
                                .await
                                {
                                    let body = String::from_utf8_lossy(&body_bytes).into_owned();
                                    let bytes_downloaded = body.len() as u64;
                                    let elapsed_ms = start_time.elapsed().as_millis() as u64;
                                    f.record_success(cid, bytes_downloaded, elapsed_ms);
                                    health.record_success();
                                    health.record_latency(elapsed_ms as f32);
                                    consecutive_all_dead = 0; // Reset backoff on success

                                    if !body.is_empty() {
                                        let html_len = body.len();
                                        let start_idx = html_len.saturating_sub(4000);
                                        println!(
                                            "[DEBUG WORLDLEAKS PARSER] Raw HTML end ({} bytes): {}",
                                            html_len,
                                            &body[start_idx..]
                                        );
                                    }
                                } else {
                                    f.record_failure(cid);
                                    health.record_failure();
                                    crate::work_stealing::requeue_with_backoff(
                                        &rq, next_url, retry_attempt, 3,
                                    );
                                }
                            }
                        }
                        Ok(Err(e)) => {
                            if e.to_string().contains("TTFB Timeout") {
                                f.trigger_circuit_isolation(cid).await;
                            }
                            f.record_failure(cid);
                            health.record_failure();
                            crate::work_stealing::requeue_with_backoff(
                                &rq, next_url, retry_attempt, 3,
                            );
                        }
                        Err(_) => {
                            f.record_failure(cid);
                            health.record_failure();
                            crate::work_stealing::requeue_with_backoff(
                                &rq, next_url, retry_attempt, 3,
                            );
                        }
                    }
                }
            });
        }

        while workers.join_next().await.is_some() {}

        drop(ui_tx); // Signals the UI batcher to flush and shutdown
        let final_results = all_discovered_entries.drain_all();
        Ok(final_results)
    }

    fn name(&self) -> &'static str {
        "WorldLeaks SPA"
    }
}
