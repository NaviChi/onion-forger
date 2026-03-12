// src-tauri/src/multi_client_pool.rs
// Phase 51F: Multi-Client Parallel Crawling (4 independent TorClients)
// Integrates directly with the new Phase 51E resource governor

use arti_client::TorClient;
use std::sync::Arc;
use tokio::sync::RwLock;
use tor_rtcompat::PreferredRuntime;

pub type SharedTorClient = Arc<RwLock<Option<Arc<TorClient<PreferredRuntime>>>>>;

#[derive(Clone)]
pub struct MultiClientPool {
    clients: Vec<SharedTorClient>,
    next_slot: Arc<std::sync::atomic::AtomicUsize>,
    vanguard_cache_path: std::path::PathBuf,
    seed_clients: Arc<Vec<Arc<TorClient<PreferredRuntime>>>>,
    borrowed_client_count: usize,
    // Phase 74: Limit concurrent Arti bootstrap to prevent Tokio thread pool starvation
    bootstrap_semaphore: Arc<tokio::sync::Semaphore>,
}

impl MultiClientPool {
    pub async fn new(
        count: usize,
        telemetry: Option<crate::runtime_metrics::RuntimeTelemetry>,
    ) -> anyhow::Result<Self> {
        Self::new_seeded(count, Vec::new(), telemetry).await
    }

    pub async fn new_seeded(
        count: usize,
        seed_clients: Vec<Arc<TorClient<PreferredRuntime>>>,
        telemetry: Option<crate::runtime_metrics::RuntimeTelemetry>,
    ) -> anyhow::Result<Self> {
        let mut clients = Vec::with_capacity(count);
        let vanguard_cache_path = crate::tor_runtime::state_root().join("arti/node_100/cache");
        let borrowed_client_count = seed_clients.len().min(count);
        let seed_clients = Arc::new(seed_clients);

        if count > 0 {
            if borrowed_client_count > 0 {
                for slot in 0..count {
                    let slot_client = if slot < borrowed_client_count {
                        seed_clients[slot].clone()
                    } else {
                        let source = seed_clients[slot % borrowed_client_count].clone();
                        Arc::new(source.isolated_client())
                    };
                    clients.push(Arc::new(RwLock::new(Some(slot_client))));
                }
            } else {
                // Phase 74: Seed Vanguard from native TorSwarm node 0 cache to avoid 15s consensus download
                let root_cache = crate::tor_runtime::state_root().join("arti/node_0/cache");
                let target_cache = vanguard_cache_path.clone();

                if root_cache.exists() {
                    tokio::task::spawn_blocking(move || {
                        let _ = std::fs::remove_dir_all(&target_cache);
                        let _ = std::fs::create_dir_all(&target_cache);
                        for entry in walkdir::WalkDir::new(&root_cache)
                            .into_iter()
                            .filter_map(|e| e.ok())
                        {
                            let name = entry.file_name().to_string_lossy();
                            if name == "lock" || name.contains(".lock") {
                                continue; // Skip file locks
                            }
                            let relative_path = entry.path().strip_prefix(&root_cache).unwrap();
                            let target_path = target_cache.join(relative_path);
                            if entry.file_type().is_dir() {
                                let _ = std::fs::create_dir_all(&target_path);
                            } else if entry.file_type().is_file() {
                                let _ = std::fs::copy(entry.path(), &target_path);
                            }
                        }
                    })
                    .await
                    .unwrap_or_default();
                }

                // Boot the first client sequentially. It acts as the "Consensus Vanguard".
                let first_config = crate::tor_native::build_tor_config(100)?;
                let first_client = TorClient::create_bootstrapped(first_config).await?;
                clients.push(Arc::new(RwLock::new(Some(Arc::new(first_client)))));

                for _ in 1..count {
                    clients.push(Arc::new(RwLock::new(None)));
                }
            }
        }

        let pool = Self {
            clients,
            next_slot: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            vanguard_cache_path,
            seed_clients,
            borrowed_client_count,
            bootstrap_semaphore: Arc::new(tokio::sync::Semaphore::new(1)), // Only 1 JIT bootstrap at a time
        };

        if let Some(t) = telemetry {
            let next_slot = pool.next_slot.clone();
            let len = pool.clients.len();
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
                loop {
                    interval.tick().await;
                    if !t.is_active() {
                        continue;
                    }
                    t.set_multi_client_metrics(
                        next_slot.load(std::sync::atomic::Ordering::Relaxed),
                        len,
                    );
                }
            });
        }

        Ok(pool)
    }

    pub fn borrowed_client_count(&self) -> usize {
        self.borrowed_client_count
    }

    fn isolated_seed_for_slot(&self, slot: usize) -> Option<Arc<TorClient<PreferredRuntime>>> {
        if self.seed_clients.is_empty() {
            return None;
        }

        let source = self.seed_clients[slot % self.seed_clients.len()].clone();
        Some(Arc::new(source.isolated_client()))
    }

    // Phase 69/74: Lazy initialization bridging Tor Clients
    // Governor will call this to get a dynamic client for any worker
    pub async fn get_client(&self, worker_idx: usize) -> Arc<TorClient<PreferredRuntime>> {
        let slot = worker_idx % self.clients.len().max(1);
        self.next_slot
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        {
            let guard = self.clients[slot].read().await;
            if let Some(c) = &*guard {
                return c.clone();
            }
        }

        let mut guard = self.clients[slot].write().await;
        if let Some(c) = &*guard {
            return c.clone();
        }

        if let Some(seed_client) = self.isolated_seed_for_slot(slot) {
            *guard = Some(seed_client.clone());
            return seed_client;
        }

        // Lock the bootstrap semaphore to ensure only 1 thread does heavy I/O and crypto concurrently
        let _permit = self.bootstrap_semaphore.acquire().await.ok();

        if slot > 0 {
            let vanguard_cache = self.vanguard_cache_path.clone();
            let target_cache =
                crate::tor_runtime::state_root().join(format!("arti/node_{}/cache", 100 + slot));
            tokio::task::spawn_blocking(move || {
                let _ = std::fs::remove_dir_all(&target_cache);
                let _ = std::fs::create_dir_all(&target_cache);
                for entry in walkdir::WalkDir::new(&vanguard_cache)
                    .into_iter()
                    .filter_map(|e| e.ok())
                {
                    let relative_path = entry.path().strip_prefix(&vanguard_cache).unwrap();
                    let target_path = target_cache.join(relative_path);
                    if entry.file_type().is_dir() {
                        let _ = std::fs::create_dir_all(&target_path);
                    } else if entry.file_type().is_file() {
                        let _ = std::fs::copy(entry.path(), &target_path);
                    }
                }
            })
            .await
            .unwrap_or_default();
        }

        let config = crate::tor_native::build_tor_config(slot + 100).unwrap();
        // Fallback to unwrap internally as get_client is usually infallible in signature
        let new_client = TorClient::create_bootstrapped(config).await.unwrap();
        let arc_client = Arc::new(new_client);
        *guard = Some(arc_client.clone());
        arc_client
    }

    // Called by healing engine when a whole client needs rotation
    pub async fn rotate_client(&self, slot: usize) {
        if let Some(seed_client) = self.isolated_seed_for_slot(slot) {
            let mut guard = self.clients[slot].write().await;
            *guard = Some(seed_client);
            return;
        }

        let _permit = self.bootstrap_semaphore.acquire().await.ok();
        let config = crate::tor_native::build_tor_config(slot + 100).unwrap();
        let new_client = TorClient::create_bootstrapped(config).await.unwrap();
        let mut guard = self.clients[slot].write().await;
        *guard = Some(Arc::new(new_client));
    }

    // Phase 70: Metric export for round-robin validation against DDoS heuristics
    pub fn get_total_client_requests(&self) -> usize {
        self.next_slot.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn clients_count(&self) -> usize {
        self.clients.len()
    }
}

pub fn snapshot_seed_clients(
    shared_clients: &[crate::tor_native::SharedTorClient],
    limit: usize,
) -> Vec<Arc<TorClient<PreferredRuntime>>> {
    shared_clients
        .iter()
        .take(limit)
        .filter_map(|slot| slot.read().ok().map(|guard| guard.clone()))
        .collect()
}
