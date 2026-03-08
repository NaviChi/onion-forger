use crate::bbr::BbrController;
use crate::scorer::CircuitScorer;
use crate::tor::TorProcessGuard;
use bloomfilter::Bloom;
use dashmap::{DashMap, DashSet};

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::{BufRead, BufReader};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};
use tokio::sync::Semaphore;

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CrawlOptions {
    pub listing: bool,
    pub sizes: bool,
    pub download: bool,
    pub circuits: Option<usize>,
    pub daemons: Option<usize>,
    #[serde(default)]
    pub agnostic_state: bool,
    #[serde(default)]
    pub resume: bool,
    #[serde(default)]
    pub resume_index: Option<String>,
    /// Optional password for Mega.nz password-protected links (#P! format)
    #[serde(default)]
    pub mega_password: Option<String>,
}

impl Default for CrawlOptions {
    fn default() -> Self {
        Self {
            listing: true,
            sizes: true,
            download: false,
            circuits: Some(120),
            daemons: Some(4),
            agnostic_state: false,
            resume: false,
            resume_index: None,
            mega_password: None,
        }
    }
}

/// The central Brain for the Distributed Crawler
pub struct CrawlerFrontier {
    pub target_url: String,
    pub num_daemons: usize,
    pub is_onion: bool,

    // The Tor Swarm holding active Daemons (will be cleaned up on Drop)
    // Wrapped in an Arc<tokio::sync::Mutex> to allow specific circuit hot-swapping
    pub swarm_guard: Option<Arc<tokio::sync::Mutex<TorProcessGuard>>>,

    // Memory Efficiency Strategy (Phase 4.5)
    pub visited_bloom: Mutex<Bloom<String>>,
    pub visited_hashes: Arc<DashSet<u64>>,

    // Persistent Connection Pooling
    pub http_clients: Vec<crate::arti_client::ArtiClient>,
    pub client_daemon_map: Vec<usize>,
    pub client_counter: AtomicUsize,

    // Advanced Politeness Throttle
    pub politeness_semaphore: Arc<Semaphore>,
    pub max_worker_permits: usize,

    // Phase 4 Orchestration
    pub scorer: Arc<CircuitScorer>,
    pub bbr: Arc<BbrController>,

    // Phase 5 Options
    pub active_options: CrawlOptions,

    // Cancellation flag — checked by workers to abort early
    pub cancel_flag: Arc<std::sync::atomic::AtomicBool>,
    pub processed_requests: AtomicUsize,

    // Write-Ahead-Log Phase 4.8
    pub wal_tx: UnboundedSender<String>,

    // Phase 47: Differential Crawl Resume Delta Tracking
    pub delta_new_files: AtomicUsize,

    // Phase 49: Circuit Starvation Failsafe — dead circuits get blacklisted for 60s
    pub circuit_blacklist: DashMap<usize, std::time::Instant>,

    // Phase 58: Universal Explorer Prefix Learning Ledger Integration
    pub target_paths: Option<crate::target_state::TargetPaths>,
}

fn sanitize_filename(url: &str) -> String {
    url.chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect()
}

fn wal_resume_enabled() -> bool {
    matches!(
        std::env::var("CRAWLI_WAL_RESUME").ok().as_deref(),
        Some("1" | "true" | "TRUE" | "yes" | "YES")
    )
}

fn env_usize(name: &str) -> Option<usize> {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
}

impl CrawlerFrontier {
    pub fn active_client_count(&self) -> usize {
        if self.is_onion {
            crate::tor_native::active_tor_clients()
                .len()
                .max(self.http_clients.len())
                .max(1)
        } else {
            self.http_clients.len().max(1)
        }
    }

    pub fn new(
        app: Option<tauri::AppHandle>,
        target_url: String,
        mut num_daemons: usize,
        is_onion: bool,
        _active_ports: Vec<u16>,
        arti_clients: Vec<crate::tor_native::SharedTorClient>,
        options: CrawlOptions,
        target_paths: Option<crate::target_state::TargetPaths>,
    ) -> Self {
        if num_daemons == 0 {
            num_daemons = 4;
        }

        let total_circuits = options.circuits.unwrap_or(120);
        let worker_cap = crate::resource_governor::recommend_frontier_worker_cap(
            total_circuits,
            is_onion,
            options.download,
            None,
        )
        .clamp(8, 180);
        let mut clients = Vec::new();
        let mut client_daemon_map = Vec::new();

        if is_onion {
            for (idx, shared_client) in arti_clients.iter().enumerate() {
                let tor_client_arc = if tokio::runtime::Handle::try_current().is_ok() {
                    tokio::task::block_in_place(|| shared_client.blocking_read().clone())
                } else {
                    shared_client.blocking_read().clone()
                };
                let isolation_token = arti_client::IsolationToken::new();
                let client = crate::arti_client::ArtiClient::new(
                    (*tor_client_arc).clone(),
                    Some(isolation_token),
                );
                clients.push(client);
                client_daemon_map.push(idx % num_daemons.max(1));
            }
        } else {
            for daemon_idx in 0..num_daemons.max(1) {
                clients.push(crate::arti_client::ArtiClient::new_clearnet());
                client_daemon_map.push(daemon_idx);
            }
        }

        if clients.is_empty() {
            if app.is_none() {
                for idx in 0..total_circuits.max(1) {
                    clients.push(crate::arti_client::ArtiClient::new_clearnet());
                    client_daemon_map.push(idx % num_daemons.max(1));
                }
            } else {
                panic!("Could not initialize Tor clients for runtime bootstrap.");
            }
        }

        let mut bloom = Bloom::new_for_fp_rate(5_000_000, 0.01).expect("Failed to init bloom");
        let hashes = DashSet::new();

        let safe_name = sanitize_filename(&target_url);
        let wal_path = std::env::temp_dir().join(format!("crawli_{}.wal", safe_name));
        let allow_wal_resume = wal_resume_enabled() || options.resume;

        // Default to fresh crawls so stale WAL state never suppresses new traversal.
        if !allow_wal_resume {
            let _ = std::fs::remove_file(&wal_path);
        }

        // Pre-load from WAL if resuming from a crash
        let mut loaded_count = 0;
        if allow_wal_resume {
            if let Ok(file) = std::fs::File::open(&wal_path) {
                let reader = BufReader::new(file);
                for line in reader.lines().map_while(Result::ok) {
                    let mut hasher = DefaultHasher::new();
                    line.hash(&mut hasher);
                    bloom.set(&line);
                    hashes.insert(hasher.finish());
                    loaded_count += 1;
                }
                if loaded_count > 0 {
                    use tauri::Emitter;
                    if let Some(app_handle) = &app {
                        let _ = app_handle.emit("crawl_log", format!("[FLIGHT DATA RECORDER] 💾 WAL engine activated. Recovered {} perfectly mapped nodes. Restoring mission state...", loaded_count));
                    }
                }
            }
        }

        // Phase 47: Differential Resume Index Pre-Loading
        if let Some(resume_path) = &options.resume_index {
            if let Ok(file) = std::fs::File::open(resume_path) {
                let reader = BufReader::new(file);
                let mut index_count = 0;
                // Parse standard onionforge index formats
                for line in reader.lines().map_while(Result::ok) {
                    if line.starts_with('#') || line.starts_with('=') || line.is_empty() {
                        continue;
                    }
                    if line.starts_with("TOTAL ENTRIES:")
                        || line.starts_with("CRAWL INDEX COMPLETED AT:")
                    {
                        continue; // Skip headers
                    }

                    // Extract URL or Path. Example: `[FILE]   840.40 MB (/games/test.zip)`
                    // Or raw paths. We will extract the exact path and agnostic hash it.
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 3 {
                        let path_str = parts.last().unwrap_or(&"");
                        let state_key = if options.agnostic_state {
                            crate::path_utils::extract_agnostic_path(path_str)
                        } else {
                            path_str.to_string()
                        };
                        let mut hasher = DefaultHasher::new();
                        state_key.hash(&mut hasher);
                        bloom.set(&state_key);
                        hashes.insert(hasher.finish());
                        index_count += 1;
                    }
                }
                if index_count > 0 {
                    use tauri::Emitter;
                    if let Some(app_handle) = &app {
                        let _ = app_handle.emit("crawl_log", format!("[Differential Crawl] 🔭 Resume Index loaded. Ignored {} previously completed entries. Delta-Sync active.", index_count));
                    }
                }
            }
        }

        let (wal_tx, mut wal_rx) = unbounded_channel::<String>();
        let wal_path_clone = wal_path.clone();

        // Background WAL append task (Event-Sourcing with IO Buffering for HDDs/SSDs)
        tokio::spawn(async move {
            use tokio::fs::OpenOptions as AsyncOpenOptions;
            use tokio::io::AsyncWriteExt;
            use tokio::io::BufWriter;

            if let Ok(file) = AsyncOpenOptions::new()
                .create(true)
                .append(true)
                .open(&wal_path_clone)
                .await
            {
                // 128 KB buffer to prevent IO chokes on mechanical spinning rust or slow SSDs
                let mut writer = BufWriter::with_capacity(128 * 1024, file);
                let mut flush_interval =
                    tokio::time::interval(std::time::Duration::from_millis(500));

                loop {
                    tokio::select! {
                        url_opt = wal_rx.recv() => {
                            match url_opt {
                                Some(url) => {
                                    let _ = writer.write_all(url.as_bytes()).await;
                                    let _ = writer.write_all(b"\n").await;
                                },
                                None => break, // Channel closed
                            }
                        }
                        _ = flush_interval.tick() => {
                            let _ = writer.flush().await;
                        }
                    }
                }
                let _ = writer.flush().await; // Final flush
            }
        });

        let bbr_max = total_circuits.max(1);
        // Cold-start below the ceiling so hostile/high-latency targets do not begin fully oversubscribed.
        let default_bbr_initial = if is_onion {
            num_daemons.max(1)
        } else {
            num_daemons.max(1).saturating_mul(2)
        };
        let bbr_initial = env_usize("CRAWLI_BBR_INITIAL")
            .unwrap_or(default_bbr_initial)
            .clamp(1, bbr_max);

        let scorer_capacity = total_circuits.max(client_daemon_map.len()).max(1);

        Self {
            target_url,
            num_daemons,
            is_onion,
            swarm_guard: None, // swarm_guard is typically set after `new` in an async context
            visited_bloom: Mutex::new(bloom),
            visited_hashes: Arc::new(hashes),
            http_clients: clients,
            client_daemon_map,
            client_counter: AtomicUsize::new(0),
            politeness_semaphore: Arc::new(Semaphore::new(worker_cap)),
            max_worker_permits: worker_cap,
            scorer: Arc::new(CircuitScorer::new(scorer_capacity)),
            bbr: Arc::new(BbrController::new(bbr_initial, bbr_max)),
            active_options: options,
            cancel_flag: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            processed_requests: AtomicUsize::new(0),
            wal_tx,
            delta_new_files: AtomicUsize::new(0),
            circuit_blacklist: DashMap::new(),
            target_paths,
        }
    }

    /// Signal all workers to stop
    pub fn cancel(&self) {
        self.cancel_flag.store(true, Ordering::Relaxed);
    }

    /// Check if cancellation has been requested
    pub fn is_cancelled(&self) -> bool {
        self.cancel_flag.load(Ordering::Relaxed)
    }

    /// Mark URL as visited, returns true if newly added, false if already visited
    pub fn mark_visited(&self, url: &str) -> bool {
        let state_key = if self.active_options.agnostic_state {
            crate::path_utils::extract_agnostic_path(url)
        } else {
            url.to_string()
        };

        let mut hasher = DefaultHasher::new();
        state_key.hash(&mut hasher);
        let hash = hasher.finish();

        let mut bloom = self.visited_bloom.lock().unwrap();
        if bloom.check(&state_key) {
            // Might be visited. Determine definitively:
            self.visited_hashes.insert(hash)
        } else {
            // Definitely not visited.
            bloom.set(&state_key);
            self.visited_hashes.insert(hash);
            let _ = self.wal_tx.send(state_key);
            self.delta_new_files.fetch_add(1, Ordering::Relaxed);
            true
        }
    }

    /// Get a client based on AIMD targeted concurrency scale.
    /// Phase 49: Skips circuits that are blacklisted (dead) with a 60s TTL.
    pub fn get_client(&self) -> (usize, crate::arti_client::ArtiClient) {
        let active = if self.is_onion && self.active_options.listing {
            self.active_client_count()
        } else {
            self.bbr.current_active().max(1)
        };
        let total = active.max(self.http_clients.len()).max(1);
        let live_clients = if self.is_onion {
            Some(crate::tor_native::active_tor_clients())
        } else {
            None
        };
        let blacklist_ttl = std::time::Duration::from_secs(60);

        // Try up to `total` slots to find a non-blacklisted circuit
        for _ in 0..total {
            let client_id = self.client_counter.fetch_add(1, Ordering::Relaxed) % active.max(1);
            let cid = client_id % total;

            // Phase 49: Check blacklist with TTL eviction
            if let Some(entry) = self.circuit_blacklist.get(&cid) {
                if entry.value().elapsed() < blacklist_ttl {
                    continue; // Still blacklisted, try next
                } else {
                    drop(entry);
                    self.circuit_blacklist.remove(&cid); // TTL expired, rehabilitate
                }
            }
            if let Some(live_clients) = &live_clients {
                if let Some(shared_client) = live_clients.get(cid % live_clients.len().max(1)) {
                    let tor_client_arc = if tokio::runtime::Handle::try_current().is_ok() {
                        tokio::task::block_in_place(|| shared_client.blocking_read().clone())
                    } else {
                        shared_client.blocking_read().clone()
                    };
                    let isolation_token = arti_client::IsolationToken::new();
                    return (
                        cid,
                        crate::arti_client::ArtiClient::new(
                            (*tor_client_arc).clone(),
                            Some(isolation_token),
                        ),
                    );
                }
            }
            return (
                cid % self.http_clients.len().max(1),
                self.http_clients[cid % self.http_clients.len().max(1)].clone(),
            );
        }

        // All circuits blacklisted — forcibly return the least-recently-blacklisted one
        let oldest = self
            .circuit_blacklist
            .iter()
            .min_by_key(|e| *e.value())
            .map(|e| *e.key());
        if let Some(cid) = oldest {
            self.circuit_blacklist.remove(&cid);
            return (cid, self.http_clients[cid].clone());
        }

        // Absolute fallback
        let cid = self.client_counter.fetch_add(1, Ordering::Relaxed) % total;
        if let Some(live_clients) = &live_clients {
            if let Some(shared_client) = live_clients.get(cid % live_clients.len().max(1)) {
                let tor_client_arc = if tokio::runtime::Handle::try_current().is_ok() {
                    tokio::task::block_in_place(|| shared_client.blocking_read().clone())
                } else {
                    shared_client.blocking_read().clone()
                };
                let isolation_token = arti_client::IsolationToken::new();
                return (
                    cid,
                    crate::arti_client::ArtiClient::new(
                        (*tor_client_arc).clone(),
                        Some(isolation_token),
                    ),
                );
            }
        }
        let fallback_idx = cid % self.http_clients.len().max(1);
        (fallback_idx, self.http_clients[fallback_idx].clone())
    }

    /// Report a successful HTTP fetch to adjust the AIMD window and Scorer weights
    pub fn record_success(&self, cid: usize, bytes: u64, elapsed_ms: u64) {
        self.processed_requests.fetch_add(1, Ordering::Relaxed);
        self.bbr.on_success(bytes.max(1), elapsed_ms.max(1));
        self.scorer.record_piece(cid, bytes, elapsed_ms);
    }

    /// Report a failed HTTP fetch (timeout/error) to slice the AIMD window
    pub fn record_failure(&self, _cid: usize) {
        self.processed_requests.fetch_add(1, Ordering::Relaxed);
        self.bbr.on_timeout();
    }

    pub fn visited_count(&self) -> usize {
        self.visited_hashes.len()
    }

    pub fn processed_count(&self) -> usize {
        self.processed_requests.load(Ordering::Relaxed)
    }

    pub fn active_workers(&self) -> usize {
        self.max_worker_permits
            .saturating_sub(self.politeness_semaphore.available_permits())
    }

    pub fn worker_target(&self) -> usize {
        if self.is_onion && self.active_options.listing {
            self.active_client_count().clamp(1, self.max_worker_permits)
        } else {
            self.bbr.current_active().clamp(1, self.max_worker_permits)
        }
    }

    /// Recommended worker count for metadata/listing crawls.
    ///
    /// This is intentionally more conservative than the raw permit budget so
    /// adapter-local queues do not oversubscribe the native Arti swarm.
    pub fn recommended_listing_workers(&self) -> usize {
        let client_budget = self.active_client_count();
        let permit_budget = self.max_worker_permits.max(1);

        let default_cap = if self.active_options.download { 36 } else { 64 };
        let override_cap = if self.active_options.download {
            env_usize("CRAWLI_LISTING_WORKERS_DOWNLOAD_MAX")
        } else {
            env_usize("CRAWLI_LISTING_WORKERS_MAX")
        };
        let cap = override_cap.unwrap_or(default_cap).max(1);

        let budget = crate::resource_governor::recommend_listing_budget(
            client_budget,
            permit_budget,
            self.is_onion,
            self.active_options.download,
            None,
        );

        budget
            .worker_cap
            .min(cap)
            .min(client_budget.max(1))
            .min(permit_budget)
            .max(1)
    }

    /// Phase 46: Aerospace Grade Intelligent Healing
    /// Explicitly trigger a NEWNYM equivalent circuit drop and Phantom replacement
    /// for a specifically degraded HTTP Client ID.
    pub async fn trigger_circuit_isolation(&self, cid: usize) {
        if let Some(guard) = &self.swarm_guard {
            let g = guard.lock().await;
            if let Some(swarm) = &g.native_swarm {
                let daemon_idx = self
                    .client_daemon_map
                    .get(cid)
                    .copied()
                    .unwrap_or_else(|| cid % self.num_daemons.max(1));
                if let Err(_e) = swarm.isolate_circuit(daemon_idx).await {
                    // Phase 49: Isolation failed — blacklist this circuit for 60s
                    self.circuit_blacklist
                        .insert(cid, std::time::Instant::now());
                    eprintln!(
                        "[Phase 49] Circuit {} blacklisted for 60s after isolation failure",
                        cid
                    );
                }
            }
        }
    }

    /// Accessor for the persistent Target Ledger pathing logic
    pub fn target_paths(&self) -> Option<&crate::target_state::TargetPaths> {
        self.target_paths.as_ref()
    }
}
