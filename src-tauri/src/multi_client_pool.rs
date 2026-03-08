// src-tauri/src/multi_client_pool.rs
// Phase 51F: Multi-Client Parallel Crawling (4 independent TorClients)
// Integrates directly with the new Phase 51E resource governor

use arti_client::TorClient;
use std::sync::Arc;
use tokio::sync::RwLock;
use tor_rtcompat::PreferredRuntime;

#[derive(Clone)]
pub struct MultiClientPool {
    clients: Vec<Arc<RwLock<Arc<TorClient<PreferredRuntime>>>>>,
}

impl MultiClientPool {
    pub async fn new(count: usize) -> anyhow::Result<Self> {
        let mut clients = Vec::with_capacity(count);

        if count > 0 {
            // Boot the first client sequentially. It acts as the "Consensus Vanguard".
            // It will hit Tor Directory Authorities and download the large microdescriptor payload.
            let first_config = crate::tor_native::build_tor_config(100)?;
            let first_client = TorClient::create_bootstrapped(first_config).await?;
            clients.push(Arc::new(RwLock::new(Arc::new(first_client))));

            let state_root = crate::tor_runtime::state_root();
            let vanguard_cache = state_root.join("arti/node_100/cache");

            // Copy the Vanguard's localized DB cache perfectly onto all other clients.
            // This bypasses the Tor Directory Authority bot-swarm rate limits when expanding.
            for i in 1..count {
                let target_cache = state_root.join(format!("arti/node_{}/cache", 100 + i));
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
            }

            // Now safely parallel-boot exactly what we want without overwhelming Tor consensus
            let mut streams = Vec::with_capacity(count - 1);
            for i in 1..count {
                let config = crate::tor_native::build_tor_config(i + 100)?;
                streams.push(tokio::spawn(async move {
                    let client = TorClient::create_bootstrapped(config).await?;
                    Ok::<_, anyhow::Error>(Arc::new(RwLock::new(Arc::new(client))))
                }));
            }
            let results = futures::future::join_all(streams).await;
            for res in results {
                clients.push(res??);
            }
        }

        Ok(Self { clients })
    }

    // Governor will call this to get a client for a specific worker
    pub async fn get_client(&self, worker_idx: usize) -> Arc<TorClient<PreferredRuntime>> {
        let slot = worker_idx % self.clients.len();
        self.clients[slot].read().await.clone()
    }

    // Called by healing engine when a whole client needs rotation
    pub async fn rotate_client(&self, slot: usize) {
        let config = crate::tor_native::build_tor_config(slot + 100).unwrap();
        let new_client = TorClient::create_bootstrapped(config).await.unwrap();
        let mut guard = self.clients[slot].write().await;
        *guard = Arc::new(new_client);
    }
}
