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

/// Phase 140D: Unified speed mode — single selector controls everything.
/// All modes use the same proven circuit/worker values from Test 1 (1.83 MB/s).
/// The ONLY difference is how many base TorClients (independent guard nodes)
/// are bootstrapped, which directly controls aggregate download bandwidth:
/// - **Default:** 2 guard nodes (proven 1.83 MB/s peak, arti cap from resource governor)
/// - **High:** 3 guard nodes (~50% more bandwidth, arti cap override = 12)
/// - **Aggressive:** 4 guard nodes (~100% more bandwidth, arti cap override = 16)
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub enum DownloadMode {
    /// Default — proven Test 1 config. 2 base TorClients, resource governor arti cap.
    #[serde(alias = "low", alias = "medium", alias = "default")]
    Default,
    /// High — 3 base TorClients, arti cap override = 12. +50% bandwidth.
    #[serde(alias = "high")]
    High,
    /// Aggressive — 4 base TorClients, arti cap override = 16. +100% bandwidth.
    #[serde(alias = "aggressive")]
    Aggressive,
}

impl Default for DownloadMode {
    fn default() -> Self {
        Self::Default
    }
}

impl std::fmt::Display for DownloadMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Default => write!(f, "default"),
            Self::High => write!(f, "high"),
            Self::Aggressive => write!(f, "aggressive"),
        }
    }
}

impl DownloadMode {
    /// Circuit cap multipliers for onion content-size tiers.
    /// Returns (micro, small, medium, large) caps.
    /// Phase 140D: All modes use aggressive-tier values (proven in Test 1).
    pub fn onion_content_caps(self) -> (usize, usize, usize, usize) {
        //           <16MB  <64MB  <256MB  <1GB
        (20, 28, 40, 56)
    }

    /// Maximum parallel download circuits during crawl overlap.
    /// Phase 140D: All modes use 24 (proven in Test 1).
    pub fn parallel_download_cap(self) -> usize {
        24
    }

    /// Default circuit count when none specified.
    /// Phase 140D: All modes use 24 (proven in Test 1).
    pub fn default_circuits(self) -> usize {
        24
    }

    /// Large pipeline clamp range (min, max) for onion.
    pub fn large_pipeline_clamp(self) -> (usize, usize) {
        (6, 24)
    }

    /// Number of Tor swarm clients (Arti instances) to bootstrap for crawl.
    pub fn tor_swarm_clients(self) -> usize {
        8
    }

    /// Crawl worker ceiling for the Qilin adaptive governor.
    pub fn crawl_worker_ceiling(self) -> usize {
        6
    }

    /// Phase 140D: Arti cap override for the download swarm.
    /// Returns None to use resource governor default, or Some(cap) to override.
    /// This is the key differentiator: more base TorClients = more guard nodes = more bandwidth.
    pub fn arti_cap_override(self) -> Option<usize> {
        match self {
            Self::Default => None,       // Use resource governor default (~8 → 2 base TorClients)
            Self::High => Some(12),      // 3 base TorClients → 3 guard nodes
            Self::Aggressive => Some(16), // 4 base TorClients → 4 guard nodes
        }
    }

    /// Serde value for JSON serialization (used in frontend communication).
    pub fn serde_value(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::High => "high",
            Self::Aggressive => "aggressive",
        }
    }
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CrawlOptions {
    pub listing: bool,
    pub sizes: bool,
    pub download: bool,
    pub circuits: Option<usize>,
    #[serde(default)]
    pub agnostic_state: bool,
    #[serde(default)]
    pub resume: bool,
    #[serde(default)]
    pub resume_index: Option<String>,
    #[serde(default)]
    pub force_clearnet: bool,
    /// Optional password for Mega.nz password-protected links (#P! format)
    #[serde(default)]
    pub mega_password: Option<String>,
    #[serde(default)]
    pub stealth_ramp: bool,
    /// Phase 119: Download files in parallel while the crawl is still running.
    /// Unlike `download` (which waits for crawl completion), this streams
    /// discovered files to the download engine in real-time.
    #[serde(default)]
    pub parallel_download: bool,
    /// Phase 133: Download speed mode — low/medium/aggressive.
    #[serde(default)]
    pub download_mode: DownloadMode,
}

impl Default for CrawlOptions {
    fn default() -> Self {
        Self {
            listing: true,
            sizes: true,
            download: false,
            circuits: Some(120),
            agnostic_state: true,
            resume: false,
            resume_index: None,
            force_clearnet: false,
            mega_password: None,
            stealth_ramp: true,
            parallel_download: false,
            download_mode: DownloadMode::Default,
        }
    }
}

/// The central Brain for the Distributed Crawler
pub struct CrawlerFrontier {
    pub target_url: String,
    pub num_clients: usize,
    pub is_onion: bool,

    // The Tor Swarm holding active Daemons (will be cleaned up on Drop)
    // Wrapped in an Arc<tokio::sync::Mutex> to allow specific circuit hot-swapping
    pub swarm_guard: Option<Arc<tokio::sync::Mutex<TorProcessGuard>>>,

    // Memory Efficiency Strategy (Phase 4.5)
    pub visited_bloom: Mutex<Bloom<String>>,
    pub visited_hashes: Arc<DashSet<u64>>,

    // Persistent Connection Pooling
    pub http_clients: Vec<crate::arti_client::ArtiClient>,
    pub client_slot_map: Vec<usize>,
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
    pub successful_requests: AtomicUsize,
    pub failed_requests: AtomicUsize,
    pub adapter_pending_requests: AtomicUsize,
    pub adapter_active_workers: AtomicUsize,
    pub adapter_worker_target: AtomicUsize,

    // Write-Ahead-Log Phase 4.8
    pub wal_tx: UnboundedSender<String>,

    // Phase 47: Differential Crawl Resume Delta Tracking
    pub delta_new_files: AtomicUsize,

    // Phase 49: Circuit Starvation Failsafe — dead circuits get blacklisted for 60s
    pub circuit_blacklist: DashMap<usize, std::time::Instant>,

    // Phase 58: Universal Explorer Prefix Learning Ledger Integration
    pub target_paths: Option<crate::target_state::TargetPaths>,

    // Phase 79: Multi-Node Global Failover Rotation
    pub seed_manager: Arc<crate::seed_manager::SeedManager>,

    // Phase 141: Event-driven parallel download feed.
    // When set, discovered file entries are pushed here for immediate download
    // instead of waiting for VFS polling. Set via set_download_feed().
    pub download_feed_tx: Option<std::sync::Arc<tokio::sync::mpsc::UnboundedSender<crate::adapters::FileEntry>>>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct FrontierProgressSnapshot {
    pub visited: usize,
    pub processed: usize,
    pub queued: usize,
    pub active_workers: usize,
    pub worker_target: usize,
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

fn build_onion_clients(
    arti_clients: &[crate::tor_native::SharedTorClient],
) -> (Vec<crate::arti_client::ArtiClient>, Vec<usize>) {
    let mut clients = Vec::with_capacity(arti_clients.len());
    let mut client_slot_map = Vec::with_capacity(arti_clients.len());

    for (idx, shared_client) in arti_clients.iter().enumerate() {
        let tor_client_arc = shared_client.read().unwrap().clone();
        let isolation_token = arti_client::IsolationToken::new();
        let client =
            crate::arti_client::ArtiClient::new((*tor_client_arc).clone(), Some(isolation_token));
        clients.push(client);
        client_slot_map.push(idx % arti_clients.len().max(1));
    }

    (clients, client_slot_map)
}

impl CrawlerFrontier {
    pub fn active_client_count(&self) -> usize {
        self.http_clients.len().max(1)
    }

    pub fn sync_arti_clients(
        &mut self,
        arti_clients: &[crate::tor_native::SharedTorClient],
    ) -> usize {
        if !self.is_onion || arti_clients.is_empty() {
            return self.http_clients.len();
        }

        let (clients, client_slot_map) = build_onion_clients(arti_clients);
        self.http_clients = clients;
        self.client_slot_map = client_slot_map;
        self.client_counter.store(0, Ordering::Relaxed);
        self.http_clients.len()
    }

    pub fn new(
        app: Option<tauri::AppHandle>,
        target_url: String,
        num_clients: usize,
        is_onion: bool,
        _active_ports: Vec<u16>,
        arti_clients: Vec<crate::tor_native::SharedTorClient>,
        options: CrawlOptions,
        target_paths: Option<crate::target_state::TargetPaths>,
    ) -> Self {
        let num_clients = num_clients.max(1);

        let total_circuits = options.circuits.unwrap_or(120);
        let worker_cap = crate::resource_governor::recommend_frontier_worker_cap(
            total_circuits,
            is_onion,
            options.download,
            None,
        )
        .clamp(8, 180);
        let mut clients = Vec::new();
        let mut client_slot_map = Vec::new();

        if is_onion {
            (clients, client_slot_map) = build_onion_clients(&arti_clients);
        } else {
            for slot_idx in 0..num_clients {
                clients.push(crate::arti_client::ArtiClient::new_clearnet());
                client_slot_map.push(slot_idx);
            }
        }

        if clients.is_empty() {
            if app.is_none() {
                for idx in 0..total_circuits.max(1) {
                    clients.push(crate::arti_client::ArtiClient::new_clearnet());
                    client_slot_map.push(idx % num_clients);
                }
            } else {
                panic!("Could not initialize Tor clients for runtime bootstrap.");
            }
        }

        // Phase 130: Right-sized bloom filter. Old 5M init wasted ~5.7MB for Qilin targets
        // (~50K URLs). 200K init uses ~240KB — 24× RAM savings. DashSet backup handles collisions.
        let mut bloom = Bloom::new_for_fp_rate(200_000, 0.01).expect("Failed to init bloom");
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
        // Phase 117: BBR cold-start uses client count instead of legacy num_daemons
        let default_bbr_initial = if is_onion {
            clients.len().max(1)
        } else {
            clients.len().max(1).saturating_mul(2)
        };
        let bbr_initial = env_usize("CRAWLI_BBR_INITIAL")
            .unwrap_or(default_bbr_initial)
            .clamp(1, bbr_max);

        let scorer_capacity = total_circuits.max(client_slot_map.len()).max(1);

        Self {
            target_url: target_url.clone(),
            num_clients,
            is_onion,
            swarm_guard: None, // swarm_guard is typically set after `new` in an async context
            visited_bloom: Mutex::new(bloom),
            visited_hashes: Arc::new(hashes),
            http_clients: clients,
            client_slot_map,
            client_counter: AtomicUsize::new(0),
            politeness_semaphore: Arc::new(Semaphore::new(worker_cap)),
            max_worker_permits: worker_cap,
            scorer: Arc::new(CircuitScorer::new(scorer_capacity)),
            bbr: Arc::new(BbrController::new(bbr_initial, bbr_max)),
            active_options: options,
            cancel_flag: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            processed_requests: AtomicUsize::new(0),
            successful_requests: AtomicUsize::new(0),
            failed_requests: AtomicUsize::new(0),
            adapter_pending_requests: AtomicUsize::new(0),
            adapter_active_workers: AtomicUsize::new(0),
            adapter_worker_target: AtomicUsize::new(0),
            wal_tx,
            delta_new_files: AtomicUsize::new(0),
            circuit_blacklist: DashMap::new(),
            target_paths,
            seed_manager: Arc::new(crate::seed_manager::SeedManager::new(
                target_url.clone(),
                Vec::new(),
            )),
            download_feed_tx: None,
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

    /// Expose whether the Vanguard stealth ramp is active for this session
    pub fn stealth_ramp_active(&self) -> bool {
        self.active_options.stealth_ramp
    }

    /// Phase 141: Set the download feed channel sender.
    /// Called from lib.rs after frontier construction but before crawl starts.
    pub fn set_download_feed(&mut self, tx: std::sync::Arc<tokio::sync::mpsc::UnboundedSender<crate::adapters::FileEntry>>) {
        self.download_feed_tx = Some(tx);
    }

    /// Phase 141: Push a batch of entries to the download feed channel.
    /// Non-blocking, best-effort. Returns how many entries were successfully sent.
    pub fn notify_download_feed(&self, entries: &[crate::adapters::FileEntry]) -> usize {
        let tx = match &self.download_feed_tx {
            Some(tx) => tx,
            None => return 0,
        };
        let mut sent = 0;
        for entry in entries {
            if matches!(entry.entry_type, crate::adapters::EntryType::File) {
                if tx.send(entry.clone()).is_ok() {
                    sent += 1;
                }
            }
        }
        sent
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
            let fallback_idx = cid % self.http_clients.len().max(1);
            return (fallback_idx, self.http_clients[fallback_idx].clone());
        }

        // Absolute fallback
        let cid = self.client_counter.fetch_add(1, Ordering::Relaxed) % total;
        let fallback_idx = cid % self.http_clients.len().max(1);
        (fallback_idx, self.http_clients[fallback_idx].clone())
    }

    /// Report a successful HTTP fetch to adjust the AIMD window and Scorer weights
    pub fn record_success(&self, cid: usize, bytes: u64, elapsed_ms: u64) {
        self.processed_requests.fetch_add(1, Ordering::Relaxed);
        self.successful_requests.fetch_add(1, Ordering::Relaxed);
        self.bbr.on_success(bytes.max(1), elapsed_ms.max(1));
        self.scorer.record_piece(cid, bytes, elapsed_ms);
    }

    /// Report a failed HTTP fetch (timeout/error) to slice the AIMD window
    pub fn record_failure(&self, _cid: usize) {
        self.processed_requests.fetch_add(1, Ordering::Relaxed);
        self.failed_requests.fetch_add(1, Ordering::Relaxed);
        self.bbr.on_timeout();
    }

    pub fn visited_count(&self) -> usize {
        self.visited_hashes.len()
    }

    pub fn processed_count(&self) -> usize {
        self.processed_requests.load(Ordering::Relaxed)
    }

    pub fn successful_count(&self) -> usize {
        self.successful_requests.load(Ordering::Relaxed)
    }

    pub fn failed_count(&self) -> usize {
        self.failed_requests.load(Ordering::Relaxed)
    }

    pub fn set_adapter_pending_requests(&self, pending: usize) {
        self.adapter_pending_requests
            .store(pending, Ordering::Relaxed);
    }

    pub fn set_adapter_worker_target(&self, worker_target: usize) {
        self.adapter_worker_target
            .store(worker_target, Ordering::Relaxed);
    }

    pub fn begin_adapter_request(&self) -> usize {
        self.adapter_active_workers.fetch_add(1, Ordering::AcqRel) + 1
    }

    pub fn finish_adapter_request(&self) -> usize {
        self.adapter_active_workers
            .fetch_update(Ordering::AcqRel, Ordering::Relaxed, |current| {
                Some(current.saturating_sub(1))
            })
            .unwrap_or(0)
            .saturating_sub(1)
    }

    pub fn clear_adapter_progress(&self) {
        self.adapter_pending_requests.store(0, Ordering::Relaxed);
        self.adapter_active_workers.store(0, Ordering::Relaxed);
        self.adapter_worker_target.store(0, Ordering::Relaxed);
    }

    pub fn active_workers(&self) -> usize {
        let permit_workers = self
            .max_worker_permits
            .saturating_sub(self.politeness_semaphore.available_permits());
        permit_workers.max(self.adapter_active_workers.load(Ordering::Relaxed))
    }

    pub fn worker_target(&self) -> usize {
        let base_target = if self.is_onion && self.active_options.listing {
            self.active_client_count().clamp(1, self.max_worker_permits)
        } else {
            self.bbr.current_active().clamp(1, self.max_worker_permits)
        };
        let adapter_target = self.adapter_worker_target.load(Ordering::Relaxed);
        base_target.max(adapter_target).max(self.active_workers())
    }

    pub fn progress_snapshot(&self) -> FrontierProgressSnapshot {
        let processed = self.processed_count();
        let queued = self
            .visited_count()
            .saturating_sub(processed)
            .max(self.adapter_pending_requests.load(Ordering::Relaxed));
        let active_workers = self.active_workers();
        let worker_target = self.worker_target().max(active_workers);
        let visited = self.visited_count().max(processed.saturating_add(queued));

        FrontierProgressSnapshot {
            visited,
            processed,
            queued,
            active_workers,
            worker_target,
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
                    .client_slot_map
                    .get(cid)
                    .copied()
                    .unwrap_or_else(|| cid % self.num_clients.max(1));
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

#[cfg(test)]
mod tests {
    use super::{CrawlOptions, CrawlerFrontier};

    fn test_frontier() -> CrawlerFrontier {
        CrawlerFrontier::new(
            None,
            "http://example.onion/root/".to_string(),
            1,
            false,
            Vec::new(),
            Vec::new(),
            CrawlOptions::default(),
            None,
        )
    }

    #[tokio::test]
    async fn adapter_progress_snapshot_overlays_pending_and_workers() {
        let frontier = test_frontier();
        assert!(frontier.mark_visited("http://example.onion/root/"));
        assert!(frontier.mark_visited("http://example.onion/root/A/"));
        frontier.record_success(0, 4096, 10);
        frontier.set_adapter_pending_requests(3);
        frontier.set_adapter_worker_target(12);
        let started = frontier.begin_adapter_request();

        let snapshot = frontier.progress_snapshot();

        assert_eq!(started, 1);
        assert_eq!(snapshot.processed, 1);
        assert_eq!(snapshot.queued, 3);
        assert_eq!(snapshot.active_workers, 1);
        assert_eq!(snapshot.worker_target, 12);
        assert_eq!(snapshot.visited, 4);

        let remaining = frontier.finish_adapter_request();
        assert_eq!(remaining, 0);
    }

    #[tokio::test]
    async fn clearing_adapter_progress_resets_overlay() {
        let frontier = test_frontier();
        frontier.set_adapter_pending_requests(5);
        frontier.set_adapter_worker_target(9);
        frontier.begin_adapter_request();
        frontier.clear_adapter_progress();

        let snapshot = frontier.progress_snapshot();

        assert_eq!(snapshot.queued, 0);
        assert_eq!(snapshot.active_workers, 0);
        assert!(snapshot.worker_target >= 1);
    }
}
