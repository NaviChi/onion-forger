use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;

const NODE_TTL_SECS: u64 = 604_800; // 7 days
const PROBE_TIMEOUT_SECS: u64 = 15;
const PREFERRED_NODE_TIMEOUT_SECS: u64 = 8;
const TOURNAMENT_HEAD_WIDTH: usize = 4;
const BASE_NODE_COOLDOWN_SECS: u64 = 45;
const MAX_NODE_COOLDOWN_SECS: u64 = 15 * 60;

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
    /// Successful probe count used for tournament scoring
    #[serde(default)]
    pub success_count: u32,
    /// Failed probe count used for tournament scoring
    #[serde(default)]
    pub failure_count: u32,
    /// Consecutive failures used to apply temporary demotion
    #[serde(default)]
    pub failure_streak: u32,
    /// Unix timestamp until which this node should be temporarily deprioritized
    #[serde(default)]
    pub cooldown_until: u64,
}

impl StorageNode {
    fn success_ratio(&self) -> f64 {
        let success = self.success_count as f64 + 1.0;
        let total = (self.success_count + self.failure_count) as f64 + 2.0;
        success / total
    }

    fn is_cooling_down(&self, now: u64) -> bool {
        self.cooldown_until > now
    }

    fn tournament_score(&self, now: u64) -> f64 {
        let latency_ms = if self.avg_latency_ms == 0 {
            20_000.0
        } else {
            self.avg_latency_ms as f64
        };
        let latency_score = (15_000.0 / latency_ms.clamp(250.0, 30_000.0)).clamp(0.1, 60.0);
        let reliability_score = self.success_ratio() * 100.0;
        let stickiness_bonus = (self.hit_count.min(20) as f64) * 1.25;
        let freshness_bonus = if now.saturating_sub(self.last_seen) <= 60 * 60 {
            6.0
        } else {
            0.0
        };
        let streak_penalty = self.failure_streak.min(12) as f64 * 5.0;
        let cooldown_penalty = if self.is_cooling_down(now) {
            250.0
        } else {
            0.0
        };

        reliability_score + latency_score + stickiness_bonus + freshness_bonus
            - streak_penalty
            - cooldown_penalty
    }

    fn record_success(&mut self, latency_ms: u64) {
        self.last_seen = now_unix();
        self.hit_count = self.hit_count.saturating_add(1);
        self.success_count = self.success_count.saturating_add(1);
        self.failure_streak = 0;
        self.cooldown_until = 0;
        self.avg_latency_ms = if self.avg_latency_ms == 0 {
            latency_ms
        } else {
            ((self.avg_latency_ms as f64 * 0.65) + (latency_ms as f64 * 0.35)) as u64
        };
    }

    fn record_failure(&mut self) {
        self.failure_count = self.failure_count.saturating_add(1);
        self.failure_streak = self.failure_streak.saturating_add(1);
        let cooldown = BASE_NODE_COOLDOWN_SECS
            .saturating_mul(1_u64 << self.failure_streak.min(4))
            .min(MAX_NODE_COOLDOWN_SECS);
        self.cooldown_until = now_unix().saturating_add(cooldown);
    }
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
    fn looks_like_live_qdata_listing(body: &str) -> bool {
        body.contains("QData")
            || body.contains("Data browser")
            || body.contains("<table id=\"list\">")
            || body.contains("File Name")
            || body.contains("File Size")
            || body.contains("<td class=\"link\">")
    }

    fn resolve_candidate_url(base_url: &str, raw_target: &str) -> Option<String> {
        if let Ok(parsed) = reqwest::Url::parse(raw_target) {
            return Some(parsed.to_string());
        }
        let base = reqwest::Url::parse(base_url).ok()?;
        base.join(raw_target).ok().map(|joined| joined.to_string())
    }

    async fn validate_stage_a_candidate(
        &self,
        uuid: &str,
        final_url: &str,
        body: &str,
    ) -> Option<StorageNode> {
        if !Self::looks_like_live_qdata_listing(body) {
            println!(
                "[QilinNodeCache] Stage A — redirect landed, but body did not look like a QData listing. Continuing discovery."
            );
            return None;
        }

        let parsed_final = reqwest::Url::parse(final_url).ok()?;
        let host = parsed_final.host_str().unwrap_or("").to_string();
        let node = StorageNode {
            url: final_url.to_string(),
            host: host.clone(),
            last_seen: now_unix(),
            avg_latency_ms: 0,
            hit_count: 5,
            success_count: 1,
            failure_count: 0,
            failure_streak: 0,
            cooldown_until: 0,
        };
        let _ = self.save_node(uuid, &node).await;
        println!(
            "[QilinNodeCache] Stage A — Valid live listing discovered & cached: {}",
            host
        );
        Some(node)
    }

    async fn follow_watch_data_target(
        &self,
        client: &crate::arti_client::ArtiClient,
        cms_url: &str,
        uuid: &str,
        raw_target: &str,
    ) -> Option<StorageNode> {
        let resolved = Self::resolve_candidate_url(cms_url, raw_target)?;
        println!(
            "[QilinNodeCache] Stage B — Following Watch data candidate: {}",
            resolved
        );

        let resp = client.get(&resolved).send().await.ok()?;
        let final_url = resp.url().as_str().to_string();
        let status = resp.status();
        let body = resp.text().await.ok()?;

        println!(
            "[QilinNodeCache] Stage B — Watch data candidate status={}, final={}",
            status, final_url
        );

        if !status.is_success() && status.as_u16() != 301 && status.as_u16() != 302 {
            return None;
        }

        self.validate_stage_a_candidate(uuid, &final_url, &body)
            .await
    }

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
    /// Phase 42 Fix 2: Automatically evicts nodes older than 7 days (604800s).
    pub async fn get_nodes(&self, uuid: &str) -> Vec<StorageNode> {
        let guard = self.db.lock().await;
        let mut nodes = Vec::new();
        let now = now_unix();
        if let Some(db) = guard.as_ref() {
            let prefix = format!("node:{}:", uuid);
            let mut stale_keys = Vec::new();
            for item in db.scan_prefix(prefix.as_bytes()).flatten() {
                if let Ok(node) = serde_json::from_slice::<StorageNode>(&item.1) {
                    if now.saturating_sub(node.last_seen) > NODE_TTL_SECS {
                        stale_keys.push(item.0.to_vec());
                        println!(
                            "[QilinNodeCache] TTL eviction: {} (last seen {}s ago)",
                            node.host,
                            now - node.last_seen
                        );
                    } else {
                        nodes.push(node);
                    }
                }
            }
            for key in stale_keys {
                let _ = db.remove(key);
            }
            if !nodes.is_empty() {
                let _ = db.flush_async().await;
            }
        }
        nodes.sort_by(|a, b| {
            b.tournament_score(now)
                .partial_cmp(&a.tournament_score(now))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.avg_latency_ms.cmp(&b.avg_latency_ms))
                .then(b.hit_count.cmp(&a.hit_count))
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
            success_count: 0,
            failure_count: 0,
            failure_streak: 0,
            cooldown_until: 0,
        };
        let _ = self.save_node(uuid, &node).await;
        println!("[QilinNodeCache] Seeded: {} -> {}", uuid, url);
    }

    async fn persist_node_state(&self, uuid: &str, node: StorageNode) {
        let _ = self.save_node(uuid, &node).await;
    }

    async fn record_probe_failure(&self, uuid: &str, node: &StorageNode, reason: &str) {
        let mut updated = node.clone();
        updated.record_failure();
        self.persist_node_state(uuid, updated.clone()).await;
        println!(
            "[QilinNodeCache] Demoted node {} after {} (cooldown until {})",
            updated.host, reason, updated.cooldown_until
        );
    }

    async fn record_probe_success(
        &self,
        uuid: &str,
        node: &StorageNode,
        latency_ms: u64,
    ) -> StorageNode {
        let mut updated = node.clone();
        updated.record_success(latency_ms);
        self.persist_node_state(uuid, updated.clone()).await;
        updated
    }

    async fn probe_node(
        &self,
        uuid: &str,
        client: &crate::arti_client::ArtiClient,
        node: StorageNode,
        timeout_secs: u64,
    ) -> Option<(StorageNode, u128)> {
        let start = std::time::Instant::now();
        match tokio::time::timeout(
            Duration::from_secs(timeout_secs),
            client.get(&node.url).send(),
        )
        .await
        {
            Ok(Ok(resp)) => {
                let latency = start.elapsed().as_millis();
                let status = resp.status();
                println!(
                    "[QilinNodeCache] Probe — {} responded in {}ms (status={})",
                    node.host, latency, status
                );

                if status.is_success() || status.as_u16() == 301 || status.as_u16() == 302 {
                    let updated = self.record_probe_success(uuid, &node, latency as u64).await;
                    Some((updated, latency))
                } else {
                    self.record_probe_failure(uuid, &node, &format!("status {}", status))
                        .await;
                    None
                }
            }
            Ok(Err(e)) => {
                println!("[QilinNodeCache] Probe — {} unreachable: {}", node.host, e);
                self.record_probe_failure(uuid, &node, &e.to_string()).await;
                None
            }
            Err(_) => {
                println!("[QilinNodeCache] Probe — {} timed out", node.host);
                self.record_probe_failure(uuid, &node, "timeout").await;
                None
            }
        }
    }

    /// Pre-seed the cache with all known Qilin QData storage domains.
    /// These are the storage hosts that host the actual file data.
    /// Each gets paired with the UUID to form a probable URL.
    /// Phase 42: Expanded with all historically discovered nodes so they
    /// are always checked as fallback candidates during Stage D probing.
    pub async fn seed_known_mirrors(&self, uuid: &str) {
        let known_storage_hosts = vec![
            // === Active (confirmed alive 2026-03-05) ===
            "szgkpzhcrnshftjb5mtvd6bc5oep5yabmgfmwt7u3tiqzfikoew27hqd.onion",
            // === Previously active QData storage nodes ===
            "7mnkv5nvnjyifezlfyba6gek7aeimg5eghej5vp65qxnb2hjbtlttlyd.onion",
            "25mjg55vcbjzwykz2uqsvaw7hcevm4pqxl42o324zr6qf5zgddmghkqd.onion",
            "arrfcpipltlfgxc6hvjylixc6c5hrummwctz4wqysk3h56ntqz5scnad.onion",
            // === Discovered via sled cache (prior runs) ===
            "qjupqf5xbmc76jzne7xu7y2ddmwtfxbbzzeax6gs4lezg3dyr5bfu2qd.onion",
            "sbedmjsyphfctagwoxuspblefvzjvb7yig4gsq5ddwjhnyq4rqcqg3ad.onion",
            "xy6pysqr5myuau4aq6uszwdgdmjx4ypjlvngupxfjdtzfsq6jugcadyd.onion",
            "amkryua4xdnbvk4urxleuxkcdgiirmus7m2wnqj3o4uh2xcgbkpcjoyd.onion",
            "astvjnzh4ftvnp37n47zgr3qhbyftlmjdocjnwjb5xlua5xgdckew6yd.onion",
            "bmwlkiljav3aqxbgyrqgcmotasrnnolqfivzorpn7snrmprj2sqqlbqd.onion",
            "cw2kf4ieepslxvydi7qgb5vc2itst4b6roah5rc3ozeu4ulbqz4v3rqd.onion",
            "ghnqjhi7usidnrnktsctb5do26m4xbaprenpy3fzkfatvf536w5drrid.onion",
            "vzgsc7keieq52csmskmmhop2yc2tys32jpj7wdgzhsoctpi4wx4hx3ad.onion",
            "n2bpey4k45pkwjfsuqpuagm2rjyaefako4hqz2pgwqaew3rs4iy7brid.onion",
            "ckj4f6jmx7rwvr6qcc7bkx3ziluf6s2kas2xua47ze7jcjvrh6bvihyd.onion",
            "7zffbbkye7c7m4676sqfxhcwtjcuslhlmxmeg7yhf3a24xl7ppm36tid.onion",
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
        client: &crate::arti_client::ArtiClient,
    ) -> Option<StorageNode> {
        println!(
            "[QilinNodeCache] Starting multi-path discovery for UUID: {}",
            uuid
        );

        let parsed = reqwest::Url::parse(cms_url).ok()?;
        let base = format!("{}://{}", parsed.scheme(), parsed.host_str()?);

        // Stage A: Follow 302 redirect from /site/data
        let data_url = format!("{}/site/data?uuid={}", base, uuid);
        println!(
            "[QilinNodeCache] Stage A — Following 302 redirect: {}",
            data_url
        );

        for attempt in 1..=3 {
            match client.get(&data_url).header("Referer", &base).send().await {
                Ok(resp) => {
                    let final_url = resp.url().as_str().to_string();
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    println!(
                        "[QilinNodeCache] Stage A — Status={}, FinalURL={}",
                        status, final_url
                    );

                    if final_url != data_url {
                        if let Some(node) = self
                            .validate_stage_a_candidate(uuid, &final_url, &body)
                            .await
                        {
                            return Some(node);
                        }
                    }
                    break;
                }
                Err(e) => {
                    println!(
                        "[QilinNodeCache] Stage A — attempt {} failed: {}",
                        attempt, e
                    );
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
            }
        }

        // Stage B: Scrape the view page for the QData storage reference
        let view_url = format!("{}/site/view?uuid={}", base, uuid);
        println!(
            "[QilinNodeCache] Stage B — Scraping view page: {}",
            view_url
        );

        if let Ok(resp) = client.get(&view_url).send().await {
            if let Ok(body) = resp.text().await {
                let watch_data_re = regex::Regex::new(
                    r#"(?is)(?:href|data-url|onclick)\s*=\s*["']([^"']*(?:site/data\?uuid=[^"']+|http://[a-z2-7]{56}\.onion/[^"']+/))["'][^>]*>\s*[^<]*watch data"#
                ).unwrap();
                for cap in watch_data_re.captures_iter(&body) {
                    if let Some(target) = cap.get(1).map(|m| m.as_str()) {
                        if let Some(node) = self
                            .follow_watch_data_target(client, &view_url, uuid, target)
                            .await
                        {
                            return Some(node);
                        }
                    }
                }

                // Phase 42 Fix 3: Hardened regex patterns for QData storage node discovery
                // Captures: value="<onion>", >onion<, href="http://onion/...", data-url="...", iframe src="..."
                let value_re = regex::Regex::new(
                    r#"(?:value="|href="http://|data-url="http://|src="http://)([a-z2-7]{56}\.onion)[/"\s]|>([a-z2-7]{56}\.onion)<"#
                ).unwrap();
                for cap in value_re.captures_iter(&body) {
                    let onion_host = cap.get(1).or(cap.get(2)).map(|m| m.as_str().to_string());
                    if let Some(onion_host) = onion_host {
                        // Skip the CMS domain itself
                        if onion_host == parsed.host_str().unwrap_or("") {
                            continue;
                        }
                        println!(
                            "[QilinNodeCache] Stage B — Found QData storage reference: {}",
                            onion_host
                        );

                        // This is likely the storage host. Construct the URL with the UUID.
                        let storage_url = format!("http://{}/{}/", onion_host, uuid);
                        let node = StorageNode {
                            url: storage_url,
                            host: onion_host,
                            last_seen: now_unix(),
                            avg_latency_ms: 0,
                            hit_count: 0,
                            success_count: 0,
                            failure_count: 0,
                            failure_streak: 0,
                            cooldown_until: 0,
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
        println!(
            "[QilinNodeCache] Stage C — {} cached nodes found",
            cached_nodes.len()
        );

        if cached_nodes.is_empty() {
            println!("[QilinNodeCache] No nodes discovered for UUID {}", uuid);
            return None;
        }

        let now = now_unix();
        let mut preferred_nodes: Vec<StorageNode> = cached_nodes
            .iter()
            .filter(|node| !node.is_cooling_down(now))
            .cloned()
            .collect();
        let mut fallback_nodes: Vec<StorageNode> = cached_nodes
            .iter()
            .filter(|node| node.is_cooling_down(now))
            .cloned()
            .collect();

        if preferred_nodes.is_empty() {
            preferred_nodes.append(&mut fallback_nodes);
        } else {
            preferred_nodes.extend(fallback_nodes.into_iter().take(2));
        }

        if let Some(sticky) = preferred_nodes.first().cloned() {
            let sticky_confident = sticky.hit_count >= 3 && sticky.success_ratio() >= 0.65;
            if sticky_confident {
                println!(
                    "[QilinNodeCache] Sticky winner probe: {} (score {:.2})",
                    sticky.host,
                    sticky.tournament_score(now)
                );
                if let Some((winner, _)) = self
                    .probe_node(uuid, client, sticky, PREFERRED_NODE_TIMEOUT_SECS)
                    .await
                {
                    println!(
                        "[QilinNodeCache] ✅ Sticky winner held: {} ({}ms avg, {} hits)",
                        winner.host, winner.avg_latency_ms, winner.hit_count
                    );
                    return Some(winner);
                }
            }
        }

        // Stage D: Probe the tournament head first, then score by reliability + latency.
        println!(
            "[QilinNodeCache] Stage D — Probing {} candidate nodes ({} preferred) concurrently...",
            preferred_nodes.len(),
            preferred_nodes
                .iter()
                .filter(|node| !node.is_cooling_down(now))
                .count()
        );

        let best: Arc<Mutex<Option<(StorageNode, u128, f64)>>> = Arc::new(Mutex::new(None));
        let cache_ref = self.clone();
        let uuid_owned = uuid.to_string();

        preferred_nodes.sort_by(|a, b| {
            b.tournament_score(now)
                .partial_cmp(&a.tournament_score(now))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let head_width = TOURNAMENT_HEAD_WIDTH.min(preferred_nodes.len());
        let head_nodes = preferred_nodes[..head_width].to_vec();
        let tail_nodes = preferred_nodes[head_width..].to_vec();

        let mut head_tasks = tokio::task::JoinSet::new();
        for node in head_nodes {
            let client = client.clone();
            let best_ref = best.clone();
            let cache = cache_ref.clone();
            let uuid_str = uuid_owned.clone();
            let score_snapshot = node.tournament_score(now);

            head_tasks.spawn(async move {
                if let Some((updated, latency)) = cache
                    .probe_node(&uuid_str, &client, node, PROBE_TIMEOUT_SECS)
                    .await
                {
                    let mut guard = best_ref.lock().await;
                    let adjusted_score = score_snapshot - (latency as f64 / 1000.0);
                    if guard.as_ref().is_none_or(|(_, best_lat, best_score)| {
                        adjusted_score > *best_score
                            || ((adjusted_score - *best_score).abs() < f64::EPSILON
                                && latency < *best_lat)
                    }) {
                        *guard = Some((updated, latency, adjusted_score));
                    }
                }
            });
        }

        while head_tasks.join_next().await.is_some() {}

        if best.lock().await.is_none() && !tail_nodes.is_empty() {
            println!(
                "[QilinNodeCache] Stage D — Tournament head failed, probing {} fallback candidates...",
                tail_nodes.len()
            );
            let mut tail_tasks = tokio::task::JoinSet::new();
            for node in tail_nodes {
                let client = client.clone();
                let best_ref = best.clone();
                let cache = cache_ref.clone();
                let uuid_str = uuid_owned.clone();
                let score_snapshot = node.tournament_score(now);

                tail_tasks.spawn(async move {
                    if let Some((updated, latency)) = cache
                        .probe_node(&uuid_str, &client, node, PROBE_TIMEOUT_SECS)
                        .await
                    {
                        let mut guard = best_ref.lock().await;
                        let adjusted_score = score_snapshot - (latency as f64 / 1000.0);
                        if guard.as_ref().is_none_or(|(_, best_lat, best_score)| {
                            adjusted_score > *best_score
                                || ((adjusted_score - *best_score).abs() < f64::EPSILON
                                    && latency < *best_lat)
                        }) {
                            *guard = Some((updated, latency, adjusted_score));
                        }
                    }
                });
            }

            while tail_tasks.join_next().await.is_some() {}
        }

        let result = best.lock().await.clone().map(|(node, _, _)| node);
        if let Some(ref winner) = result {
            println!(
                "[QilinNodeCache] ✅ Best node: {} ({}ms avg, {} hits, {:.1}% success)",
                winner.host,
                winner.avg_latency_ms,
                winner.hit_count,
                winner.success_ratio() * 100.0
            );
        }

        result
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::StorageNode;

    #[test]
    fn cooled_node_scores_below_healthy_node() {
        let now = 1_700_000_000;
        let healthy = StorageNode {
            url: "http://a.onion/test/".to_string(),
            host: "a.onion".to_string(),
            last_seen: now,
            avg_latency_ms: 1_500,
            hit_count: 4,
            success_count: 4,
            failure_count: 1,
            failure_streak: 0,
            cooldown_until: 0,
        };
        let cooled = StorageNode {
            url: "http://b.onion/test/".to_string(),
            host: "b.onion".to_string(),
            last_seen: now,
            avg_latency_ms: 1_000,
            hit_count: 6,
            success_count: 2,
            failure_count: 6,
            failure_streak: 3,
            cooldown_until: now + 120,
        };

        assert!(healthy.tournament_score(now) > cooled.tournament_score(now));
    }

    #[test]
    fn success_resets_failure_streak() {
        let mut node = StorageNode {
            url: "http://a.onion/test/".to_string(),
            host: "a.onion".to_string(),
            last_seen: 0,
            avg_latency_ms: 0,
            hit_count: 0,
            success_count: 0,
            failure_count: 0,
            failure_streak: 0,
            cooldown_until: 0,
        };

        node.record_failure();
        assert!(node.failure_streak > 0);
        assert!(node.cooldown_until > 0);

        node.record_success(1_200);
        assert_eq!(node.failure_streak, 0);
        assert_eq!(node.cooldown_until, 0);
        assert_eq!(node.success_count, 1);
        assert_eq!(node.hit_count, 1);
    }
}
