use crate::adapters::{CrawlerAdapter, FileEntry, SiteFingerprint};
use crate::frontier::CrawlerFrontier;
use std::sync::Arc;
use tauri::AppHandle;
use tokio::sync::mpsc;

#[derive(Default)]
pub struct PearAdapter;

#[async_trait::async_trait]
impl CrawlerAdapter for PearAdapter {
    async fn can_handle(&self, fingerprint: &SiteFingerprint) -> bool {
        fingerprint
            .url
            .contains("m3wwhkus4dxbnxbtihexlyd2cv63qrvex6jiebc4vqe22kg2z3udebid.onion")
            || fingerprint.body.to_lowercase().contains("pear ransomware")
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

        let max_concurrent = 120; // Massive worker-stealer parallel pool
        let mut workers = tokio::task::JoinSet::new();

        let _base_url = current_url.to_string();

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

                    let _permit = f.politeness_semaphore.acquire().await.ok();
                    let (_cid, _client) = f.get_client();

                    let mut new_files = Vec::new();

                    let start_time = std::time::Instant::now();
                    let mut html = String::new();
                    // active_cid is just used inside the loop

                    for _attempt in 0..4 {
                        let (current_cid, current_client) = f.get_client();
                        html = match tokio::time::timeout(
                            std::time::Duration::from_secs(45),
                            current_client.get(&next_url).send(),
                        )
                        .await
                        {
                            Ok(Ok(resp)) if resp.status().is_success() => {
                                resp.text().await.unwrap_or_default()
                            }
                            Ok(Ok(resp)) => {
                                if resp.status() == 404 {
                                    break;
                                }
                                String::new()
                            }
                            _ => String::new(),
                        };

                        if !html.is_empty() {
                            f.record_success(
                                current_cid,
                                html.len() as u64,
                                start_time.elapsed().as_millis() as u64,
                            );
                            break;
                        } else {
                            f.record_failure(current_cid);
                        }
                    }

                    if !html.is_empty() {
                        let parsed_files = crate::adapters::autoindex::parse_autoindex_html(&html);

                        for doc in &parsed_files {
                            let base_clean = next_url.trim_end_matches('/');
                            let absolute_url = if doc.0.starts_with("http") {
                                doc.0.clone()
                            } else if doc.0.starts_with('/') {
                                if let Ok(u) = url::Url::parse(base_clean) {
                                    format!(
                                        "{}://{}{}",
                                        u.scheme(),
                                        u.host_str().unwrap_or(""),
                                        doc.0
                                    )
                                } else {
                                    format!("{}{}", base_clean, doc.0)
                                }
                            } else {
                                format!("{}/{}", base_clean, doc.0)
                            };

                            let file_entry = crate::adapters::FileEntry {
                                path: format!("/{}", doc.0),
                                size_bytes: doc.1,
                                entry_type: if doc.2 {
                                    crate::adapters::EntryType::Folder
                                } else {
                                    crate::adapters::EntryType::File
                                },
                                raw_url: absolute_url.clone(),
                            };

                            new_files.push(file_entry);

                            if doc.2
                                && absolute_url.matches('/').count() < 12
                                && f.mark_visited(&absolute_url)
                            {
                                pending_clone.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                                q_clone.push(absolute_url);
                            }
                        }
                    }

                    for file in &new_files {
                        let _ = ui_tx_clone.send(file.clone()).await;
                    }

                    if !new_files.is_empty() {
                        let mut lock = discovered_ref.lock().await;
                        lock.extend(new_files);
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
        "Pear Ransomware Crawler"
    }

    fn known_domains(&self) -> Vec<&'static str> {
        vec!["m3wwhkus4dxbnxbtihexlyd2cv63qrvex6jiebc4vqe22kg2z3udebid.onion"]
    }
}
