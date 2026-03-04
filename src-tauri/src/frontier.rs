use crate::bbr::BbrController;
use crate::scorer::CircuitScorer;
use crate::tor::TorProcessGuard;
use bloomfilter::Bloom;
use dashmap::DashSet;
use reqwest::{Client, Proxy};
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
}

impl Default for CrawlOptions {
    fn default() -> Self {
        Self {
            listing: true,
            sizes: true,
            download: false,
            circuits: Some(120),
            daemons: Some(4),
        }
    }
}

/// The central Brain for the Distributed Crawler
pub struct CrawlerFrontier {
    pub target_url: String,
    pub num_daemons: usize,
    pub is_onion: bool,

    // The Tor Swarm holding active Daemons (will be cleaned up on Drop)
    pub swarm_guard: Option<TorProcessGuard>,

    // Memory Efficiency Strategy (Phase 4.5)
    pub visited_bloom: Mutex<Bloom<String>>,
    pub visited_hashes: Arc<DashSet<u64>>,

    // Persistent Connection Pooling
    pub http_clients: Vec<Client>,
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

fn insecure_tls_allowed() -> bool {
    matches!(
        std::env::var("CRAWLI_ALLOW_INSECURE_TLS").ok().as_deref(),
        Some("1" | "true" | "TRUE" | "yes" | "YES")
    )
}

fn apply_tls_policy(builder: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
    if insecure_tls_allowed() {
        builder.danger_accept_invalid_certs(true)
    } else {
        builder
    }
}

impl CrawlerFrontier {
    pub fn new(
        app: Option<tauri::AppHandle>,
        target_url: String,
        mut num_daemons: usize,
        is_onion: bool,
        active_ports: Vec<u16>,
        options: CrawlOptions,
    ) -> Self {
        if num_daemons == 0 {
            num_daemons = 4;
        }

        // Initialize Persistent connection pools
        let total_circuits = options.circuits.unwrap_or(120);
        let circuits_per_daemon = if num_daemons > 0 {
            total_circuits.div_ceil(num_daemons)
        } else {
            120
        };
        let worker_cap = total_circuits.clamp(16, 180);
        let mut clients = Vec::new();
        for daemon_idx in 0..num_daemons.max(1) {
            let port = active_ports
                .get(daemon_idx)
                .copied()
                .unwrap_or(9051 + daemon_idx as u16);
            for circuit_idx in 0..circuits_per_daemon {
                if is_onion {
                    // Setting a unique auth string enforces Tor to use an isolated circuit for this exact socket
                    let proxy_url = format!("socks5h://circuit_{circuit_idx}:pwd@127.0.0.1:{port}");
                    if let Ok(proxy) = Proxy::all(&proxy_url) {
                        if let Ok(client) = apply_tls_policy(
                            Client::builder()
                                .proxy(proxy)
                                .timeout(std::time::Duration::from_secs(120))
                                .connect_timeout(std::time::Duration::from_secs(45))
                                .pool_max_idle_per_host(8) // Keep-alives
                                .tcp_nodelay(true),
                        )
                        .build()
                        {
                            clients.push(client);
                        }
                    }
                } else if let Ok(client) = apply_tls_policy(
                    Client::builder()
                        .timeout(std::time::Duration::from_secs(120))
                        .connect_timeout(std::time::Duration::from_secs(45))
                        .pool_max_idle_per_host(8)
                        .tcp_nodelay(true),
                )
                .build()
                {
                    clients.push(client);
                    break; // non-onion only needs 1 proxy-less client per daemon (or just 1 total)
                }
            }
        }

        // Safety fallback
        if clients.is_empty() {
            clients.push(Client::new());
        }

        let mut bloom = Bloom::new_for_fp_rate(5_000_000, 0.01).expect("Failed to init bloom");
        let hashes = DashSet::new();

        let safe_name = sanitize_filename(&target_url);
        let wal_path = format!("/tmp/crawli_{}.wal", safe_name);
        let allow_wal_resume = wal_resume_enabled();

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
        // Start aggressively at configured circuit ceiling; BBR snaps instantly to active BDP anyway.
        let bbr_initial = bbr_max.max(num_daemons.max(1)).min(bbr_max);

        Self {
            target_url,
            num_daemons,
            is_onion,
            swarm_guard: None, // swarm_guard is typically set after `new` in an async context
            visited_bloom: Mutex::new(bloom),
            visited_hashes: Arc::new(hashes),
            http_clients: clients,
            client_counter: AtomicUsize::new(0),
            politeness_semaphore: Arc::new(Semaphore::new(worker_cap)),
            max_worker_permits: worker_cap,
            scorer: Arc::new(CircuitScorer::new(num_daemons.max(1))),
            bbr: Arc::new(BbrController::new(bbr_initial, bbr_max)),
            active_options: options,
            cancel_flag: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            processed_requests: AtomicUsize::new(0),
            wal_tx,
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
        let mut hasher = DefaultHasher::new();
        url.hash(&mut hasher);
        let hash = hasher.finish();

        let mut bloom = self.visited_bloom.lock().unwrap();
        let url_string = url.to_string();
        if bloom.check(&url_string) {
            // Might be visited. Determine definitively:
            self.visited_hashes.insert(hash)
        } else {
            // Definitely not visited.
            bloom.set(&url_string);
            self.visited_hashes.insert(hash);
            let _ = self.wal_tx.send(url_string);
            true
        }
    }

    /// Get a client based on AIMD targeted concurrency scale
    pub fn get_client(&self) -> (usize, reqwest::Client) {
        // Crawl path for onion targets is operator-pinned to the configured circuit pool.
        // Keep full fanout available for crawler workers instead of shrinking on transient AIMD backoff.
        let active = if self.is_onion && self.active_options.listing {
            self.http_clients.len().max(1)
        } else {
            self.bbr.current_active().max(1)
        };
        let client_id = self.client_counter.fetch_add(1, Ordering::Relaxed) % active.max(1);
        let cid = client_id % self.http_clients.len();
        (cid, self.http_clients[cid].clone())
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
            self.max_worker_permits
        } else {
            self.bbr.current_active().clamp(1, self.max_worker_permits)
        }
    }
}
