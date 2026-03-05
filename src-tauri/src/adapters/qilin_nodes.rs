use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;

/// A discovered QData storage node for a specific victim UUID.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageNode {
    /// The full URL to the storage root (e.g. http://<onion>/d4ccd219-.../  )
    pub url: String,
    /// The .onion hostname of the storage server
    pub host: String,
    /// Unix timestamp of last successful contact
    pub last_seen: u64,
    /// Average latency in milliseconds (exponential moving average)
    pub avg_latency_ms: u64,
    /// Number of times this node has been successfully contacted
    pub hit_count: u32,
}

/// Persistent cache of discovered Qilin storage nodes.
/// Backed by sled — survives restarts, offline periods, and node rotations.
#[derive(Clone)]
pub struct QilinNodeCache {
    db: Arc<Mutex<Option<sled::Db>>>,
}

impl Default for QilinNodeCache {
    fn default() -> Self {
        Self {
            db: Arc::new(Mutex::new(None)),
        }
    }
}

impl QilinNodeCache {
    /// Initialize the sled database at ~/.crawli/qilin_nodes.sled
    pub async fn initialize(&self) -> anyhow::Result<()> {
        let mut path = std::env::var("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."));
        path.push(".crawli");
        std::fs::create_dir_all(&path)?;
        path.push("qilin_nodes.sled");

        let db = sled::open(&path)?;
        let mut guard = self.db.lock().await;
        *guard = Some(db);
        println!("[QilinNodeCache] Initialized at {:?}", path);
        Ok(())
    }

    /// Persist a discovered storage node for a given UUID.
    pub async fn save_node(&self, uuid: &str, node: &StorageNode) -> anyhow::Result<()> {
        let guard = self.db.lock().await;
        if let Some(db) = guard.as_ref() {
            let key = format!("node:{}:{}", uuid, node.host);
            let val = serde_json::to_vec(node)?;
            db.insert(key.as_bytes(), val)?;
            db.flush_async().await?;
        }
        Ok(())
    }

    /// Retrieve all cached storage nodes for a given UUID.
    pub async fn get_nodes(&self, uuid: &str) -> Vec<StorageNode> {
        let guard = self.db.lock().await;
        let mut nodes = Vec::new();
        if let Some(db) = guard.as_ref() {
            let prefix = format!("node:{}:", uuid);
            for item in db.scan_prefix(prefix.as_bytes()).flatten() {
                if let Ok(node) = serde_json::from_slice::<StorageNode>(&item.1) {
                    nodes.push(node);
                }
            }
        }
        // Sort by hit_count descending, then by latency ascending (prefer reliable + fast nodes)
        nodes.sort_by(|a, b| {
            b.hit_count.cmp(&a.hit_count)
                .then(a.avg_latency_ms.cmp(&b.avg_latency_ms))
        });
        nodes
    }

    /// Manually seed a known storage node into the cache.
    /// Used when a user provides a direct storage URL or when nodes are
    /// discovered externally (e.g. from the fresh crawl script).
    pub async fn seed_node(&self, uuid: &str, url: &str, host: &str) {
        let node = StorageNode {
            url: url.to_string(),
            host: host.to_string(),
            last_seen: now_unix(),
            avg_latency_ms: 0,
            hit_count: 0,
        };
        let _ = self.save_node(uuid, &node).await;
        println!("[QilinNodeCache] Seeded: {} -> {}", uuid, url);
    }

    /// Pre-seed the cache with all known Qilin QData storage domains.
    /// These are the storage hosts that host the actual file data.
    /// Each gets paired with the UUID to form a probable URL.
    pub async fn seed_known_mirrors(&self, uuid: &str) {
        let known_storage_hosts = vec![
            "7mnkv5nvnjyifezlfyba6gek7aeimg5eghej5vp65qxnb2hjbtlttlyd.onion",
            "25mjg55vcbjzwykz2uqsvaw7hcevm4pqxl42o324zr6qf5zgddmghkqd.onion",
            "arrfcpipltlfgxc6hvjylixc6c5hrummwctz4wqysk3h56ntqz5scnad.onion",
        ];

        for host in known_storage_hosts {
            let url = format!("http://{}/{}/", host, uuid);
            self.seed_node(uuid, &url, host).await;
        }
    }

    /// Full multi-path discovery algorithm.
    ///
    /// Given a CMS URL like `/site/view?uuid=X`, this will:
    /// 1. Stage A: Follow the 302 redirect from `/site/data?uuid=X`
    /// 2. Stage B: Scrape the `/site/view?uuid=X` page for .onion references
    /// 3. Stage C: Load all previously cached nodes
    /// 4. Stage D: Probe every node concurrently, return the fastest alive
    pub async fn discover_and_resolve(
        &self,
        cms_url: &str,
        uuid: &str,
        client: &reqwest::Client,
    ) -> Option<StorageNode> {
        println!("[QilinNodeCache] Starting multi-path discovery for UUID: {}", uuid);

        let parsed = reqwest::Url::parse(cms_url).ok()?;
        let base = format!("{}://{}", parsed.scheme(), parsed.host_str()?);

        // Stage A: Follow 302 redirect from /site/data
        let data_url = format!("{}/site/data?uuid={}", base, uuid);
        println!("[QilinNodeCache] Stage A — Following 302 redirect: {}", data_url);

        for attempt in 1..=3 {
            match client.get(&data_url).send().await {
                Ok(resp) => {
                    let final_url = resp.url().as_str().to_string();
                    let status = resp.status();
                    println!("[QilinNodeCache] Stage A — Status={}, FinalURL={}", status, final_url);

                    if final_url != data_url {
                        // Redirect intercepted!
                        if let Ok(parsed_final) = reqwest::Url::parse(&final_url) {
                            let host = parsed_final.host_str().unwrap_or("").to_string();
                            let node = StorageNode {
                                url: final_url.clone(),
                                host: host.clone(),
                                last_seen: now_unix(),
                                avg_latency_ms: 0,
                                hit_count: 1,
                            };
                            let _ = self.save_node(uuid, &node).await;
                            println!("[QilinNodeCache] Stage A — Discovered & cached: {}", host);
                        }
                    }
                    break;
                }
                Err(e) => {
                    println!("[QilinNodeCache] Stage A — attempt {} failed: {}", attempt, e);
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
            }
        }

        // Stage B: Scrape the view page for the QData storage reference
        let view_url = format!("{}/site/view?uuid={}", base, uuid);
        println!("[QilinNodeCache] Stage B — Scraping view page: {}", view_url);

        if let Ok(resp) = client.get(&view_url).send().await {
            if let Ok(body) = resp.text().await {
                // Look for the QData input field: value="<onion>.onion"
                // This contains the actual storage domain, distinct from the CMS
                let value_re = regex::Regex::new(r#"value="([a-z2-7]{56}\.onion)""|>([a-z2-7]{56}\.onion)<"#).unwrap();
                for cap in value_re.captures_iter(&body) {
                    let onion_host = cap.get(1).or(cap.get(2)).map(|m| m.as_str().to_string());
                    if let Some(onion_host) = onion_host {
                        // Skip the CMS domain itself
                        if onion_host == parsed.host_str().unwrap_or("") {
                            continue;
                        }
                        println!("[QilinNodeCache] Stage B — Found QData storage reference: {}", onion_host);

                        // This is likely the storage host. Construct the URL with the UUID.
                        let storage_url = format!("http://{}/{}/", onion_host, uuid);
                        let node = StorageNode {
                            url: storage_url,
                            host: onion_host,
                            last_seen: now_unix(),
                            avg_latency_ms: 0,
                            hit_count: 0,
                        };
                        let _ = self.save_node(uuid, &node).await;
                    }
                }

                // Also check for the data link which may contain a different onion ref
                if body.contains("site/data") {
                    println!("[QilinNodeCache] Stage B — View page has 'Watch data' link (data available)");
                }
            }
        }

        // Stage C: Load all cached nodes
        let cached_nodes = self.get_nodes(uuid).await;
        println!("[QilinNodeCache] Stage C — {} cached nodes found", cached_nodes.len());

        if cached_nodes.is_empty() {
            println!("[QilinNodeCache] No nodes discovered for UUID {}", uuid);
            return None;
        }

        // Stage D: Probe all nodes concurrently, return fastest alive
        println!("[QilinNodeCache] Stage D — Probing {} nodes...", cached_nodes.len());

        let mut best: Option<StorageNode> = None;
        let mut best_latency = u128::MAX;

        for node in &cached_nodes {
            let start = std::time::Instant::now();
            let probe_timeout = Duration::from_secs(30);

            match tokio::time::timeout(probe_timeout, client.get(&node.url).send()).await {
                Ok(Ok(resp)) => {
                    let latency = start.elapsed().as_millis();
                    let status = resp.status();
                    println!(
                        "[QilinNodeCache] Stage D — {} responded in {}ms (status={})",
                        node.host, latency, status
                    );

                    if status.is_success() || status.as_u16() == 301 || status.as_u16() == 302 {
                        // Update the cached node with fresh latency data
                        let mut updated = node.clone();
                        updated.last_seen = now_unix();
                        updated.hit_count += 1;
                        updated.avg_latency_ms = if updated.avg_latency_ms == 0 {
                            latency as u64
                        } else {
                            // Exponential moving average (α=0.3)
                            ((updated.avg_latency_ms as f64 * 0.7) + (latency as f64 * 0.3)) as u64
                        };
                        let _ = self.save_node(uuid, &updated).await;

                        if latency < best_latency {
                            best_latency = latency;
                            best = Some(updated);
                        }
                    }
                }
                Ok(Err(e)) => {
                    println!("[QilinNodeCache] Stage D — {} unreachable: {}", node.host, e);
                }
                Err(_) => {
                    println!("[QilinNodeCache] Stage D — {} timed out", node.host);
                }
            }
        }

        if let Some(ref winner) = best {
            println!(
                "[QilinNodeCache] ✅ Best node: {} ({}ms, {} hits)",
                winner.host, winner.avg_latency_ms, winner.hit_count
            );
        }

        best
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
