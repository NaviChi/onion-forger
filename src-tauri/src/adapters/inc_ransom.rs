use tauri::AppHandle;
use std::sync::Arc;
use tokio::sync::mpsc;
use crate::adapters::{CrawlerAdapter, SiteFingerprint, FileEntry, EntryType};
use crate::frontier::CrawlerFrontier;
use serde::Deserialize;

#[derive(Default)]
pub struct IncRansomAdapter;

#[derive(Deserialize, Debug)]
struct CdnMeta {
    onion: String,
}

#[derive(Deserialize, Debug)]
struct DisclosurePayload {
    _id: String,
    cdn: Option<CdnMeta>,
}

#[derive(Deserialize, Debug)]
struct DisclosureResponse {
    #[serde(default)]
    #[serde(rename="type")]
    success: bool,
    payload: Option<Vec<DisclosurePayload>>,
}

#[derive(Deserialize, Debug)]
struct IncFolderEntry {
    originalname: String,
    path: String,
    size: Option<u64>,
    #[serde(default)]
    #[serde(rename="isFolder")]
    is_folder: bool,
}

#[derive(Deserialize, Debug)]
struct FolderResponse {
    #[serde(default)]
    #[serde(rename="type")]
    success: bool,
    payload: Option<Vec<IncFolderEntry>>,
}

#[async_trait::async_trait]
impl CrawlerAdapter for IncRansomAdapter {
    async fn can_handle(&self, fingerprint: &SiteFingerprint) -> bool {
        fingerprint.url.contains("incblog") || fingerprint.body.contains("INC Ransom")
    }

    async fn crawl(
        &self, 
        current_url: &str, 
        frontier: Arc<CrawlerFrontier>, 
        app: AppHandle
    ) -> anyhow::Result<Vec<FileEntry>> {
        use tauri::Emitter;
        
        let client_singleton = frontier.get_client().1;
        
        let announcement_id = if let Some(pos) = current_url.rfind('/') {
            &current_url[pos + 1..]
        } else {
            return Err(anyhow::anyhow!("Invalid INC Ransom URL structure"));
        };
        
        let api_url = format!("http://incbacg6bfwtrlzwdbqc55gsfl763s3twdtwhp27dzuik6s6rwdcityd.onion/api/v1/blog/get/disclosures/{}", announcement_id);
        app.emit("crawl_log", format!("[System] Fetching INC disclosure for {}...", announcement_id)).unwrap_or_default();
        let resp_result = client_singleton.get(&api_url).send().await;
        
        let mut disc_id = String::new();
        let mut cdn_onion = String::new();
        
        match resp_result {
            Ok(resp) => {
                if let Ok(data) = resp.json::<DisclosureResponse>().await {
                    if data.success {
                        if let Some(payloads) = data.payload {
                            if !payloads.is_empty() {
                                disc_id = payloads[0]._id.clone();
                                if let Some(cdn) = &payloads[0].cdn {
                                    // The INC API returns URL-encoded CDN addresses  
                                    // e.g. "http%3A%2F%2Finc2eoul...onion"
                                    // We must decode them for reqwest to reach the host
                                    cdn_onion = urlencoding::decode(&cdn.onion)
                                        .unwrap_or(std::borrow::Cow::Borrowed(&cdn.onion))
                                        .to_string();
                                } else {
                                    return Err(anyhow::anyhow!("Missing CDN configuration in INC response"));
                                }
                            } else {
                                return Err(anyhow::anyhow!("Empty payloads in INC disclosure"));
                            }
                        }
                    } else {
                        return Err(anyhow::anyhow!("Failed parsing INC Disclosures API payload"));
                    }
                } else {
                    return Err(anyhow::anyhow!("Invalid INC Disclosures Response Format"));
                }
            }
            Err(e) => {
                return Err(anyhow::anyhow!("Failed fetching INC Disclosure info: {}", e));
            }
        }
        
        app.emit("crawl_log", format!("[System] Found CDN: {}, DisclosureID: {}", cdn_onion, disc_id)).unwrap_or_default();

        // 1. Setup multi-producer single-consumer queue
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        let all_discovered_entries = Arc::new(tokio::sync::Mutex::new(Vec::new()));
        
        // Pending counter: tracks items that are either queued OR actively being processed.
        // A worker decrements this only AFTER it has finished processing AND enqueued any
        // child folders. This eliminates the race condition where the loop could terminate
        // between a worker finishing and its newly-discovered paths being consumed.
        let pending = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        
        // 2. Initial seed
        let seed_path = "./".to_string();
        tx.send(seed_path.clone())?;
        frontier.mark_visited(&seed_path);
        pending.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        // 3. Batched UI Backpressure Task (Protects React from 1000s of rapid events)
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

        // 4. Autonomous Worker Pool with pending-counter termination
        let max_concurrent = 120;
        let mut active_tasks: usize = 0;
        let mut workers = tokio::task::JoinSet::new();

        loop {
            if frontier.is_cancelled() {
                app.emit("crawl_log", "[System] Crawl cancelled by user.".to_string()).unwrap_or_default();
                break;
            }
            
            // Spawn workers for all queued paths up to concurrency limit
            while active_tasks < max_concurrent {
                if let Ok(next_path) = rx.try_recv() {
                    let f = frontier.clone();
                    let tx_clone = tx.clone();
                    let ui_tx_clone = ui_tx.clone();
                    let discovered_ref = all_discovered_entries.clone();
                    let disc_id_clone = disc_id.clone();
                    let cdn_onion_clone = cdn_onion.clone();
                    let pending_clone = pending.clone();

                    active_tasks += 1;
                    workers.spawn(async move {
                        if f.is_cancelled() {
                            pending_clone.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                            return;
                        }
                        
                        // Await permission from the global Politeness Semaphore to avoid DOSing the onion
                        let _permit = f.politeness_semaphore.acquire().await.ok();
                        
                        // Grab a Keep-Alive Tor circuit client from the pool
                        let (cid, client) = f.get_client();
                        
                        let mut safe_path = next_path.clone();
                        if !safe_path.starts_with("./") {
                            safe_path = format!("./{}", safe_path.trim_start_matches('/'));
                        }

                        let folder_api_url = format!("{}/api/v1/blog/get/folder", cdn_onion_clone);
                        
                        let body = serde_json::json!({
                            "disclosureId": disc_id_clone,
                            "path": safe_path,
                        });
                        
                        let start_time = std::time::Instant::now();
                        let resp_result = client.post(&folder_api_url)
                            .json(&body)
                            .send()
                            .await;
                            
                        f.record_success(cid, 4096, start_time.elapsed().as_millis() as u64);
                        let mut new_files = Vec::new();
                        
                        if let Ok(resp) = resp_result {
                            if let Ok(folder_res) = resp.json::<FolderResponse>().await {
                                if folder_res.success {
                                    if let Some(entries) = folder_res.payload {
                                        for entry in entries {
                                            let etype = if entry.is_folder { EntryType::Folder } else { EntryType::File };
                                            
                                            let clean_path = entry.path.trim_start_matches("./").to_string();
                                            let mut file_path = if !clean_path.starts_with('/') {
                                                format!("/{}", clean_path)
                                            } else {
                                                clean_path.clone()
                                            };
                                            if file_path == "/" {
                                                file_path = format!("/{}", entry.originalname);
                                            }
                                            
                                            let raw_url = format!("{}/api/v1/blog/download/{}", cdn_onion_clone, clean_path); 
                                            
                                            new_files.push(FileEntry {
                                                path: file_path.clone(),
                                                size_bytes: if f.active_options.sizes { entry.size } else { None },
                                                entry_type: etype.clone(),
                                                raw_url,
                                            });
                                            if entry.is_folder && f.active_options.listing {
                                                let mut sub_path = entry.path.clone();
                                                if !sub_path.ends_with('/') {
                                                    sub_path.push('/');
                                                }
                                                if f.mark_visited(&sub_path) {
                                                    pending_clone.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                                                    tx_clone.send(sub_path).unwrap_or_default();
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        // Flush partial results to IPC batcher
                        if !new_files.is_empty() {
                            let mut locked = discovered_ref.lock().await;
                            for file in new_files {
                                let _ = ui_tx_clone.send(file.clone()).await;
                                locked.push(file);
                            }
                        }
                        
                        // This work unit is complete. Decrement pending counter.
                        // We already incremented for each child folder above, so
                        // the counter correctly reflects remaining work.
                        pending_clone.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                    });
                } else {
                    break;
                }
            }

            // Wait for at least one worker to finish
            if let Some(_res) = workers.join_next().await {
                active_tasks -= 1;
            } else {
                // No active workers — check if truly done via atomic counter
                if pending.load(std::sync::atomic::Ordering::SeqCst) == 0 {
                    break;
                }
                // Items may have just been enqueued; yield and retry
                tokio::task::yield_now().await;
            }
        }
        
        drop(ui_tx);
        let mut final_results = all_discovered_entries.lock().await;
        Ok(final_results.drain(..).collect())
    }

    fn name(&self) -> &'static str {
        "INC Ransom Crawler"
    }
}
