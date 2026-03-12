use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::Manager;
use tokio::sync::Mutex;

const NODE_TTL_SECS: u64 = 604_800; // 7 days
                                    // Phase 76D: Aligned with arti connect_timeout=30s for v3 HS rendezvous cold-start
const PROBE_TIMEOUT_SECS: u64 = 35;
const STAGE_HTTP_TIMEOUT_SECS: u64 = 35;
const BASE_NODE_COOLDOWN_SECS: u64 = 45;
const MAX_NODE_COOLDOWN_SECS: u64 = 15 * 60;
const CONNECT_ERROR_COOLDOWN_SECS: u64 = 15;
const HTTP_404_COOLDOWN_SECS: u64 = 120;
const FRESH_REDIRECT_TTL_SECS: u64 = 15 * 60;
const WINNER_LEASE_TTL_SECS: u64 = 10 * 60;
const HINT_PROBE_TIMEOUT_SECS: u64 = 12;
const FAST_PATH_PROBE_LIMIT: usize = 2;
const PRIORITIZED_MIRROR_LIMIT: usize = 5;
const DEGRADED_PRIORITIZED_MIRROR_LIMIT: usize = 12;
const PROBE_WAVE_SIZE: usize = 2;
const DEGRADED_PROBE_WAVE_SIZE: usize = 3;
const REDIRECT_RING_LIMIT: usize = 4;
const STAGE_A_SAMPLE_ATTEMPTS: usize = 3;
const STAGE_A_TARGET_UNIQUE_HOSTS: usize = 2;
const STAGE_A_INTER_ATTEMPT_DELAY_MS: u64 = 350;
const STICKY_WINNER_MIN_QUALITY_SCORE: f64 = 18.0;
const STICKY_WINNER_PROBE_MARGIN: f64 = 12.0;

/// Emit a discovery progress event to the Tauri UI (if app handle is provided)
fn emit_discovery_progress(app: Option<&tauri::AppHandle>, stage: &str, detail: &str) {
    use tauri::Emitter;
    let msg = format!("[Qilin Discovery] {} — {}", stage, detail);
    println!("{}", msg);
    if let Some(app) = app {
        let _ = app.emit("crawl_log", msg);
    }
}

fn with_runtime_telemetry<F>(app: Option<&tauri::AppHandle>, f: F)
where
    F: FnOnce(&crate::runtime_metrics::RuntimeTelemetry),
{
    if let Some(app) = app {
        if let Some(state) = app.try_state::<crate::AppState>() {
            f(&state.telemetry);
        }
    }
}

fn record_discovery_success(app: Option<&tauri::AppHandle>) {
    with_runtime_telemetry(app, |telemetry| {
        telemetry.record_discovery_request_success()
    });
}

fn record_discovery_failure(app: Option<&tauri::AppHandle>) {
    with_runtime_telemetry(app, |telemetry| {
        telemetry.record_discovery_request_failure()
    });
}

/// A discovered QData storage node for a specific victim UUID.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
    /// Completed full crawls where this host was the durable winner
    #[serde(default)]
    pub winner_run_count: u32,
    /// Sum of full-crawl completion times while this host was the durable winner
    #[serde(default)]
    pub winner_total_completion_ms: u64,
    /// Sum of effective entries produced while this host was the durable winner
    #[serde(default)]
    pub winner_total_effective_entries: u64,
    /// Full-crawl failovers observed while this host was the durable winner
    #[serde(default)]
    pub winner_failover_count: u32,
    /// Late throttle-class failures observed while this host was the durable winner
    #[serde(default)]
    pub winner_late_throttle_count: u32,
    /// Last observed full-crawl completion time for this winner
    #[serde(default)]
    pub winner_last_completion_ms: u64,
    /// Last observed effective entry count for this winner
    #[serde(default)]
    pub winner_last_effective_entries: u64,
}

/// Phase 77E: A globally registered storage host, discovered from 302 redirects.
/// Unlike StorageNode (per-UUID), this is a global record tracking known hosts
/// across all victims. Used for auto-discovery seeding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalHostRecord {
    pub host: String,
    pub first_seen: u64,
    pub last_seen: u64,
    pub discovered_from: String, // UUID that first led to this host
    pub hits: u32,
    pub alive: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RedirectHintRecord {
    node: StorageNode,
    captured_at: u64,
    expires_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WinnerLeaseRecord {
    node: StorageNode,
    captured_at: u64,
    lease_until: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct RedirectRingRecord {
    hints: Vec<RedirectHintRecord>,
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
        let freshness_bonus = if (self.success_count > 0 || self.hit_count > 0)
            && now.saturating_sub(self.last_seen) <= 60 * 60
        {
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
        let winner_quality_bonus = self.winner_quality_score();

        reliability_score
            + latency_score
            + stickiness_bonus
            + freshness_bonus
            + winner_quality_bonus
            - streak_penalty
            - cooldown_penalty
    }

    pub fn winner_effective_entries_per_sec(&self) -> f64 {
        if self.winner_total_completion_ms == 0 {
            return 0.0;
        }
        let seconds = (self.winner_total_completion_ms as f64 / 1_000.0).max(1.0);
        self.winner_total_effective_entries as f64 / seconds
    }

    pub fn winner_quality_score(&self) -> f64 {
        if self.winner_run_count == 0 {
            return 0.0;
        }

        let effective_eps = self.winner_effective_entries_per_sec();
        let throughput_bonus = (effective_eps * 1.8).clamp(0.0, 42.0);
        let completion_bonus = if self.winner_last_completion_ms > 0 {
            (240_000.0 / self.winner_last_completion_ms as f64).clamp(0.0, 22.0)
        } else {
            0.0
        };
        let scale_bonus = (self.winner_last_effective_entries as f64 / 300.0).clamp(0.0, 18.0);
        let failover_penalty = self.winner_failover_count as f64 * 5.5;
        let late_throttle_penalty = self.winner_late_throttle_count as f64 * 8.0;

        throughput_bonus + completion_bonus + scale_bonus - failover_penalty - late_throttle_penalty
    }

    pub fn recommended_repin_interval(&self) -> usize {
        if self.winner_run_count == 0 {
            return 20;
        }

        let effective_eps = self.winner_effective_entries_per_sec();
        let failovers_per_run =
            self.winner_failover_count as f64 / self.winner_run_count.max(1) as f64;
        let late_throttles_per_run =
            self.winner_late_throttle_count as f64 / self.winner_run_count.max(1) as f64;

        if late_throttles_per_run >= 1.0 || failovers_per_run >= 1.0 || effective_eps < 8.0 {
            8
        } else if late_throttles_per_run >= 0.5 || failovers_per_run >= 0.5 || effective_eps < 14.0
        {
            12
        } else if effective_eps < 22.0 {
            16
        } else if effective_eps < 32.0 {
            20
        } else {
            24
        }
    }

    pub fn qualifies_for_winner_stickiness(&self) -> bool {
        self.winner_run_count > 0
            && self.winner_quality_score() >= STICKY_WINNER_MIN_QUALITY_SCORE
            && self.failure_streak <= 1
    }

    pub fn sticky_replacement_margin(&self) -> f64 {
        if !self.qualifies_for_winner_stickiness() {
            return 0.0;
        }

        if self.winner_quality_score() >= 36.0 {
            18.0
        } else {
            STICKY_WINNER_PROBE_MARGIN
        }
    }

    fn record_winner_run(
        &mut self,
        effective_entries: u64,
        completion_ms: u64,
        failovers: u32,
        late_throttles: u32,
    ) {
        self.winner_run_count = self.winner_run_count.saturating_add(1);
        self.winner_total_completion_ms = self
            .winner_total_completion_ms
            .saturating_add(completion_ms.max(1));
        self.winner_total_effective_entries = self
            .winner_total_effective_entries
            .saturating_add(effective_entries);
        self.winner_failover_count = self.winner_failover_count.saturating_add(failovers);
        self.winner_late_throttle_count = self
            .winner_late_throttle_count
            .saturating_add(late_throttles);
        self.winner_last_completion_ms = completion_ms.max(1);
        self.winner_last_effective_entries = effective_entries;
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

    fn record_failure_with_cooldown(&mut self, cooldown_secs: u64) {
        self.failure_count = self.failure_count.saturating_add(1);
        self.failure_streak = self.failure_streak.saturating_add(1);
        self.cooldown_until = now_unix().saturating_add(cooldown_secs);
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
    fn fast_path_source_bonus(priority: u8) -> f64 {
        match priority {
            0 => 72.0, // winner lease
            1 => 44.0, // redirect hint
            _ => 18.0, // redirect ring
        }
    }

    fn rank_fast_path_candidates(
        winner_lease: Option<StorageNode>,
        redirect_hint: Option<StorageNode>,
        redirect_ring: Vec<StorageNode>,
    ) -> Vec<StorageNode> {
        let now = now_unix();
        let mut candidates = Vec::new();
        if let Some(node) = winner_lease {
            candidates.push((0u8, node));
        }
        if let Some(node) = redirect_hint {
            candidates.push((1u8, node));
        }
        for node in redirect_ring {
            candidates.push((2u8, node));
        }
        candidates.sort_by(|left, right| {
            let left_score = Self::fast_path_source_bonus(left.0) + left.1.tournament_score(now);
            let right_score = Self::fast_path_source_bonus(right.0) + right.1.tournament_score(now);
            right_score
                .partial_cmp(&left_score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| left.1.avg_latency_ms.cmp(&right.1.avg_latency_ms))
        });

        let mut ranked = Vec::new();
        let mut seen_hosts = std::collections::HashSet::new();

        for (_, candidate) in candidates {
            if seen_hosts.insert(candidate.host.clone()) {
                ranked.push(candidate);
            }
            if ranked.len() >= FAST_PATH_PROBE_LIMIT {
                break;
            }
        }

        ranked
    }

    fn rank_stage_a_candidates_with_sticky_winner(
        sticky_candidate: Option<StorageNode>,
        fresh_candidates: Vec<StorageNode>,
        now: u64,
    ) -> Vec<StorageNode> {
        let mut ranked = Vec::new();
        let mut seen_hosts = std::collections::HashSet::new();
        let best_fresh_score = fresh_candidates
            .iter()
            .map(|node| node.tournament_score(now))
            .fold(f64::NEG_INFINITY, f64::max);

        let sticky_candidate =
            sticky_candidate.filter(|node| node.qualifies_for_winner_stickiness());
        let sticky_leads = sticky_candidate.as_ref().is_some_and(|node| {
            best_fresh_score.is_infinite()
                || node.tournament_score(now) + node.sticky_replacement_margin() >= best_fresh_score
        });

        if sticky_leads {
            if let Some(sticky) = sticky_candidate.as_ref() {
                if seen_hosts.insert(sticky.host.clone()) {
                    ranked.push(sticky.clone());
                }
            }
        }

        for candidate in fresh_candidates {
            if seen_hosts.insert(candidate.host.clone()) {
                ranked.push(candidate);
            }
        }

        if !sticky_leads {
            if let Some(sticky) = sticky_candidate {
                if seen_hosts.insert(sticky.host.clone()) {
                    ranked.push(sticky);
                }
            }
        }

        ranked
    }

    fn node_key(uuid: &str, host: &str) -> String {
        format!("node:{}:{}", uuid, host)
    }

    fn redirect_hint_key(uuid: &str) -> String {
        format!("redirect_hint:{}", uuid)
    }

    fn winner_lease_key(uuid: &str) -> String {
        format!("winner_lease:{}", uuid)
    }

    fn redirect_ring_key(uuid: &str) -> String {
        format!("redirect_ring:{}", uuid)
    }

    fn seed_url_priority(url: &str, uuid: &str) -> u8 {
        if url.contains("/site/") {
            0
        } else if url.ends_with(&format!("/{}/", uuid)) {
            1
        } else {
            2
        }
    }

    fn is_site_candidate_url(url: &str) -> bool {
        url.contains("/site/")
    }

    pub(crate) fn is_host_only_seed_url(uuid: &str, url: &str) -> bool {
        !url.is_empty()
            && !Self::is_site_candidate_url(url)
            && url.ends_with(&format!("/{}/", uuid))
    }

    pub(crate) fn has_specific_listing_url(uuid: &str, url: &str) -> bool {
        !url.is_empty()
            && !Self::is_site_candidate_url(url)
            && !Self::is_host_only_seed_url(uuid, url)
    }

    pub(crate) fn is_retryable_listing_node(uuid: &str, node: &StorageNode) -> bool {
        Self::has_specific_listing_url(uuid, &node.url)
    }

    pub(crate) fn is_host_only_seed_node(uuid: &str, node: &StorageNode) -> bool {
        Self::is_host_only_seed_url(uuid, &node.url)
    }

    fn merged_seed_node(
        uuid: &str,
        seed: StorageNode,
        existing: Option<StorageNode>,
    ) -> StorageNode {
        match existing {
            Some(mut current) => {
                if Self::seed_url_priority(&seed.url, uuid)
                    > Self::seed_url_priority(&current.url, uuid)
                {
                    current.url = seed.url;
                }
                if current.host.is_empty() {
                    current.host = seed.host;
                }
                current
            }
            None => seed,
        }
    }

    pub(crate) fn looks_like_live_qdata_listing(body: &str) -> bool {
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

    async fn cache_redirect_candidate(
        &self,
        uuid: &str,
        target_url: &str,
        app: Option<&tauri::AppHandle>,
    ) -> Option<StorageNode> {
        let parsed_target = reqwest::Url::parse(target_url).ok()?;
        let host = parsed_target.host_str()?.to_string();
        let is_new = self.register_discovered_host(&host, uuid).await;
        if is_new {
            emit_discovery_progress(
                app,
                "Stage A",
                &format!("🆕 New storage node discovered: {}", host),
            );
        }
        self.emit_node_inventory(app).await;

        let node = StorageNode {
            url: target_url.to_string(),
            host,
            last_seen: now_unix(),
            avg_latency_ms: 0,
            hit_count: 0,
            success_count: 0,
            failure_count: 0,
            failure_streak: 0,
            cooldown_until: 0,
            ..StorageNode::default()
        };

        let _ = self
            .save_seed_nodes_batch(uuid, std::slice::from_ref(&node))
            .await;
        let _ = self.save_redirect_hint(uuid, &node).await;
        let _ = self.save_redirect_ring_sample(uuid, &node).await;
        Some(node)
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
        let mut node = self.get_node(uuid, &host).await.unwrap_or(StorageNode {
            url: final_url.to_string(),
            host: host.clone(),
            last_seen: now_unix(),
            avg_latency_ms: 0,
            hit_count: 0,
            success_count: 0,
            failure_count: 0,
            failure_streak: 0,
            cooldown_until: 0,
            ..StorageNode::default()
        });
        node.url = final_url.to_string();
        node.last_seen = now_unix();
        node.hit_count = node.hit_count.max(1);
        node.success_count = node.success_count.max(1);
        node.failure_streak = 0;
        node.cooldown_until = 0;

        let _ = self.save_node(uuid, &node).await;
        let _ = self.save_redirect_hint(uuid, &node).await;
        let _ = self.save_redirect_ring_sample(uuid, &node).await;
        println!(
            "[QilinNodeCache] Stage A — Valid live listing discovered & cached: {}",
            host
        );
        Some(node)
    }

    async fn probe_candidate_listing(
        &self,
        uuid: &str,
        client: crate::arti_client::ArtiClient,
        mut node: StorageNode,
        timeout_secs: u64,
        record_failures: bool,
        app: Option<tauri::AppHandle>,
    ) -> Option<StorageNode> {
        if node.url.trim().is_empty() {
            println!(
                "[QilinNodeCache] Skipping candidate {} because no listing URL is available",
                node.host
            );
            if !record_failures {
                self.invalidate_cached_hints_for_host(uuid, &node.host).await;
            }
            return None;
        }

        let started = std::time::Instant::now();
        match tokio::time::timeout(
            Duration::from_secs(timeout_secs),
            client.get(&node.url).send(),
        )
        .await
        {
            Ok(Ok(resp)) => {
                let status = resp.status();
                let final_url = resp.url().as_str().to_string();
                if status.is_success() || status.as_u16() == 301 || status.as_u16() == 302 {
                    record_discovery_success(app.as_ref());
                    let body = resp.text().await.unwrap_or_default();
                    if Self::looks_like_live_qdata_listing(&body) {
                        let latency = started.elapsed().as_millis() as u64;
                        node.url = final_url;
                        let updated = self.record_probe_success(uuid, &node, latency).await;
                        println!(
                            "[QilinNodeCache] ✅ Prioritized winner: {} ({}ms)",
                            updated.host, latency
                        );
                        return Some(updated);
                    }
                    if record_failures {
                        self.record_probe_failure_with_cooldown(
                            uuid,
                            &node,
                            "non-listing response",
                            Some(status.as_u16()),
                        )
                        .await;
                    } else {
                        self.invalidate_cached_hints_for_host(uuid, &node.host)
                            .await;
                    }
                } else if record_failures {
                    record_discovery_failure(app.as_ref());
                    self.record_probe_failure_with_cooldown(
                        uuid,
                        &node,
                        &format!("status {}", status),
                        Some(status.as_u16()),
                    )
                    .await;
                } else {
                    record_discovery_failure(app.as_ref());
                    self.invalidate_cached_hints_for_host(uuid, &node.host)
                        .await;
                }
            }
            Ok(Err(err)) => {
                record_discovery_failure(app.as_ref());
                if record_failures {
                    self.record_probe_failure_with_cooldown(uuid, &node, &err.to_string(), None)
                        .await;
                } else {
                    self.invalidate_cached_hints_for_host(uuid, &node.host)
                        .await;
                }
            }
            Err(_) => {
                record_discovery_failure(app.as_ref());
                if record_failures {
                    self.record_probe_failure(uuid, &node, "timeout").await;
                } else {
                    self.invalidate_cached_hints_for_host(uuid, &node.host)
                        .await;
                }
            }
        }

        None
    }

    async fn follow_watch_data_target(
        &self,
        client: &crate::arti_client::ArtiClient,
        cms_url: &str,
        uuid: &str,
        raw_target: &str,
        app: Option<&tauri::AppHandle>,
    ) -> Option<StorageNode> {
        let resolved = Self::resolve_candidate_url(cms_url, raw_target)?;
        println!(
            "[QilinNodeCache] Stage B — Following Watch data candidate: {}",
            resolved
        );

        for attempt in 1..=2 {
            match tokio::time::timeout(
                Duration::from_secs(STAGE_HTTP_TIMEOUT_SECS),
                client
                    .new_isolated()
                    .get(&resolved)
                    .header("Referer", cms_url)
                    .send_capturing_redirect(),
            )
            .await
            {
                Ok(Ok((resp, redirect_url))) => {
                    let status = resp.status();
                    if status.is_success() || status.is_redirection() {
                        record_discovery_success(app);
                    } else {
                        record_discovery_failure(app);
                    }

                    if let Some(target_url) = redirect_url {
                        println!(
                            "[QilinNodeCache] Stage B — Captured Watch data redirect: {}",
                            target_url
                        );
                        if let Some(node) =
                            self.cache_redirect_candidate(uuid, &target_url, app).await
                        {
                            if let Some(winner) = self
                                .probe_candidate_listing(
                                    uuid,
                                    client.new_isolated(),
                                    node,
                                    HINT_PROBE_TIMEOUT_SECS,
                                    false,
                                    app.cloned(),
                                )
                                .await
                            {
                                return Some(winner);
                            }
                        }
                    }

                    let final_url = resp.url().as_str().to_string();
                    println!(
                        "[QilinNodeCache] Stage B — Watch data candidate status={}, final={}",
                        status, final_url
                    );

                    if status.is_success() || status.as_u16() == 301 || status.as_u16() == 302 {
                        let body = resp.text().await.ok()?;
                        if let Some(validated) = self
                            .validate_stage_a_candidate(uuid, &final_url, &body)
                            .await
                        {
                            return Some(validated);
                        }
                    }
                }
                Ok(Err(_)) | Err(_) => {
                    record_discovery_failure(app);
                }
            }

            if attempt < 2 {
                tokio::time::sleep(Duration::from_millis(STAGE_A_INTER_ATTEMPT_DELAY_MS)).await;
            }
        }

        None
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
            let key = Self::node_key(uuid, &node.host);
            let val = serde_json::to_vec(node)?;
            db.insert(key.as_bytes(), val)?;
            db.flush_async().await?;
        }
        Ok(())
    }

    async fn get_node(&self, uuid: &str, host: &str) -> Option<StorageNode> {
        let guard = self.db.lock().await;
        let db = guard.as_ref()?;
        let key = Self::node_key(uuid, host);
        let raw = db.get(key.as_bytes()).ok()??;
        serde_json::from_slice(&raw).ok()
    }

    async fn save_seed_nodes_batch(&self, uuid: &str, nodes: &[StorageNode]) -> anyhow::Result<()> {
        let guard = self.db.lock().await;
        if let Some(db) = guard.as_ref() {
            for seed in nodes {
                let key = Self::node_key(uuid, &seed.host);
                let existing = db
                    .get(key.as_bytes())?
                    .and_then(|raw| serde_json::from_slice::<StorageNode>(&raw).ok());
                let merged = Self::merged_seed_node(uuid, seed.clone(), existing);
                db.insert(key.as_bytes(), serde_json::to_vec(&merged)?)?;
            }
            db.flush_async().await?;
        }
        Ok(())
    }

    async fn save_redirect_hint(&self, uuid: &str, node: &StorageNode) -> anyhow::Result<()> {
        let guard = self.db.lock().await;
        if let Some(db) = guard.as_ref() {
            let record = RedirectHintRecord {
                node: node.clone(),
                captured_at: now_unix(),
                expires_at: now_unix().saturating_add(FRESH_REDIRECT_TTL_SECS),
            };
            db.insert(
                Self::redirect_hint_key(uuid).as_bytes(),
                serde_json::to_vec(&record)?,
            )?;
            db.flush_async().await?;
        }
        Ok(())
    }

    async fn save_winner_lease(&self, uuid: &str, node: &StorageNode) -> anyhow::Result<()> {
        let guard = self.db.lock().await;
        if let Some(db) = guard.as_ref() {
            let record = WinnerLeaseRecord {
                node: node.clone(),
                captured_at: now_unix(),
                lease_until: now_unix().saturating_add(WINNER_LEASE_TTL_SECS),
            };
            db.insert(
                Self::winner_lease_key(uuid).as_bytes(),
                serde_json::to_vec(&record)?,
            )?;
            db.flush_async().await?;
        }
        Ok(())
    }

    async fn save_redirect_ring_sample(
        &self,
        uuid: &str,
        node: &StorageNode,
    ) -> anyhow::Result<()> {
        let guard = self.db.lock().await;
        if let Some(db) = guard.as_ref() {
            let now = now_unix();
            let key = Self::redirect_ring_key(uuid);
            let mut record = db
                .get(key.as_bytes())?
                .and_then(|raw| serde_json::from_slice::<RedirectRingRecord>(&raw).ok())
                .unwrap_or_default();

            record.hints.retain(|hint| {
                hint.expires_at > now && hint.node.host != node.host && hint.node.url != node.url
            });
            record.hints.insert(
                0,
                RedirectHintRecord {
                    node: node.clone(),
                    captured_at: now,
                    expires_at: now.saturating_add(FRESH_REDIRECT_TTL_SECS),
                },
            );
            record.hints.truncate(REDIRECT_RING_LIMIT);

            db.insert(key.as_bytes(), serde_json::to_vec(&record)?)?;
            db.flush_async().await?;
        }
        Ok(())
    }

    async fn get_redirect_hint(&self, uuid: &str) -> Option<StorageNode> {
        let now = now_unix();
        let key = Self::redirect_hint_key(uuid);
        let guard = self.db.lock().await;
        let db = guard.as_ref()?;
        let raw = db.get(key.as_bytes()).ok()??;
        let record: RedirectHintRecord = serde_json::from_slice(&raw).ok()?;
        if record.expires_at <= now {
            let _ = db.remove(key.as_bytes());
            return None;
        }
        let current = db
            .get(Self::node_key(uuid, &record.node.host).as_bytes())
            .ok()
            .flatten()
            .and_then(|bytes| serde_json::from_slice::<StorageNode>(&bytes).ok());
        Some(Self::merged_seed_node(uuid, record.node, current))
    }

    async fn get_winner_lease(&self, uuid: &str) -> Option<StorageNode> {
        let now = now_unix();
        let key = Self::winner_lease_key(uuid);
        let guard = self.db.lock().await;
        let db = guard.as_ref()?;
        let raw = db.get(key.as_bytes()).ok()??;
        let record: WinnerLeaseRecord = serde_json::from_slice(&raw).ok()?;
        if record.lease_until <= now {
            let _ = db.remove(key.as_bytes());
            return None;
        }
        let current = db
            .get(Self::node_key(uuid, &record.node.host).as_bytes())
            .ok()
            .flatten()
            .and_then(|bytes| serde_json::from_slice::<StorageNode>(&bytes).ok());
        let merged = Self::merged_seed_node(uuid, record.node, current);
        (!merged.is_cooling_down(now)).then_some(merged)
    }

    async fn get_redirect_ring(&self, uuid: &str) -> Vec<StorageNode> {
        let now = now_unix();
        let key = Self::redirect_ring_key(uuid);
        let guard = self.db.lock().await;
        let Some(db) = guard.as_ref() else {
            return Vec::new();
        };

        let Some(raw) = db.get(key.as_bytes()).ok().flatten() else {
            return Vec::new();
        };
        let Some(mut record) = serde_json::from_slice::<RedirectRingRecord>(&raw).ok() else {
            return Vec::new();
        };

        record.hints.retain(|hint| hint.expires_at > now);
        let mut hosts = std::collections::HashSet::new();
        let nodes: Vec<StorageNode> = record
            .hints
            .iter()
            .filter_map(|hint| {
                let current = db
                    .get(Self::node_key(uuid, &hint.node.host).as_bytes())
                    .ok()
                    .flatten()
                    .and_then(|bytes| serde_json::from_slice::<StorageNode>(&bytes).ok());
                let merged = Self::merged_seed_node(uuid, hint.node.clone(), current);
                if merged.is_cooling_down(now) || !hosts.insert(merged.host.clone()) {
                    None
                } else {
                    Some(merged)
                }
            })
            .collect();
        let mut nodes = nodes;
        nodes.sort_by(|left, right| {
            right
                .tournament_score(now)
                .partial_cmp(&left.tournament_score(now))
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        drop(guard);

        if nodes.len() != record.hints.len() {
            let _ = self.persist_redirect_ring(uuid, record).await;
        }

        nodes
    }

    async fn persist_redirect_ring(
        &self,
        uuid: &str,
        record: RedirectRingRecord,
    ) -> anyhow::Result<()> {
        let guard = self.db.lock().await;
        if let Some(db) = guard.as_ref() {
            let key = Self::redirect_ring_key(uuid);
            if record.hints.is_empty() {
                db.remove(key.as_bytes())?;
            } else {
                db.insert(key.as_bytes(), serde_json::to_vec(&record)?)?;
            }
            db.flush_async().await?;
        }
        Ok(())
    }

    async fn invalidate_cached_hints_for_host(&self, uuid: &str, host: &str) {
        let guard = self.db.lock().await;
        let Some(db) = guard.as_ref() else {
            return;
        };

        for key in [Self::redirect_hint_key(uuid), Self::winner_lease_key(uuid)] {
            if let Ok(Some(raw)) = db.get(key.as_bytes()) {
                let matches_host = serde_json::from_slice::<RedirectHintRecord>(&raw)
                    .ok()
                    .map(|record| record.node.host == host)
                    .or_else(|| {
                        serde_json::from_slice::<WinnerLeaseRecord>(&raw)
                            .ok()
                            .map(|record| record.node.host == host)
                    })
                    .unwrap_or(false);
                if matches_host {
                    let _ = db.remove(key.as_bytes());
                }
            }
        }

        let ring_key = Self::redirect_ring_key(uuid);
        if let Ok(Some(raw)) = db.get(ring_key.as_bytes()) {
            if let Ok(mut record) = serde_json::from_slice::<RedirectRingRecord>(&raw) {
                let now = now_unix();
                record
                    .hints
                    .retain(|hint| hint.expires_at > now && hint.node.host != host);
                if record.hints.is_empty() {
                    let _ = db.remove(ring_key.as_bytes());
                } else if let Ok(encoded) = serde_json::to_vec(&record) {
                    let _ = db.insert(ring_key.as_bytes(), encoded);
                }
            }
        }
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
            ..StorageNode::default()
        };
        let _ = self.save_seed_nodes_batch(uuid, &[node]).await;
        println!("[QilinNodeCache] Seeded: {} -> {}", uuid, url);
    }

    /// Phase 116: Reset all cooldowns and failure streaks for a UUID.
    /// Called during patient retry mode so that after a 15-minute wait,
    /// every cached node gets a fresh chance to be probed.
    pub async fn reset_all_cooldowns(&self, uuid: &str) -> usize {
        let guard = self.db.lock().await;
        let mut reset_count = 0usize;
        if let Some(db) = guard.as_ref() {
            let prefix = format!("node:{}:", uuid);
            for item in db.scan_prefix(prefix.as_bytes()).flatten() {
                if let Ok(mut node) = serde_json::from_slice::<StorageNode>(&item.1) {
                    if node.cooldown_until > 0 || node.failure_streak > 0 {
                        node.cooldown_until = 0;
                        node.failure_streak = 0;
                        node.last_seen = now_unix();
                        if let Ok(serialized) = serde_json::to_vec(&node) {
                            let _ = db.insert(&item.0, serialized);
                            reset_count += 1;
                        }
                    }
                }
            }
            if reset_count > 0 {
                let _ = db.flush_async().await;
            }
        }
        println!(
            "[QilinNodeCache] Phase 116 — Reset cooldowns on {} nodes for UUID {}",
            reset_count, uuid
        );
        reset_count
    }

    async fn persist_node_state(&self, uuid: &str, node: StorageNode) {
        let _ = self.save_node(uuid, &node).await;
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Phase 77E: Global Storage Node Auto-Discovery Registry
    // ─────────────────────────────────────────────────────────────────────────
    // Every time a 302 redirect reveals a new storage .onion, we record it
    // globally (not per-UUID). This means if victim A discovers node X, that
    // node is automatically seeded for victim B's crawl.

    /// Register a newly discovered storage host in the global registry.
    /// Returns true if this is a NEW host (not previously known).
    pub async fn register_discovered_host(&self, host: &str, discovered_from_uuid: &str) -> bool {
        let guard = self.db.lock().await;
        if let Some(db) = guard.as_ref() {
            let key = format!("global_host:{}", host);
            let now = now_unix();

            if db.contains_key(key.as_bytes()).unwrap_or(false) {
                // Already known — update last_seen
                if let Ok(Some(existing)) = db.get(key.as_bytes()) {
                    if let Ok(mut record) = serde_json::from_slice::<GlobalHostRecord>(&existing) {
                        record.last_seen = now;
                        record.hits += 1;
                        if let Ok(val) = serde_json::to_vec(&record) {
                            let _ = db.insert(key.as_bytes(), val);
                        }
                    }
                }
                false
            } else {
                // Brand new host!
                let record = GlobalHostRecord {
                    host: host.to_string(),
                    first_seen: now,
                    last_seen: now,
                    discovered_from: discovered_from_uuid.to_string(),
                    hits: 1,
                    alive: true,
                };
                if let Ok(val) = serde_json::to_vec(&record) {
                    let _ = db.insert(key.as_bytes(), val);
                    let _ = db.flush_async().await;
                }
                println!(
                    "[QilinNodeCache] 🆕 NEW storage host auto-registered: {} (discovered via UUID {})",
                    host, discovered_from_uuid
                );
                true
            }
        } else {
            false
        }
    }

    /// Get all globally known storage hosts.
    pub async fn get_all_global_hosts(&self) -> Vec<GlobalHostRecord> {
        let guard = self.db.lock().await;
        let mut hosts = Vec::new();
        if let Some(db) = guard.as_ref() {
            for item in db.scan_prefix(b"global_host:").flatten() {
                if let Ok(record) = serde_json::from_slice::<GlobalHostRecord>(&item.1) {
                    hosts.push(record);
                }
            }
        }
        hosts.sort_by(|a, b| b.last_seen.cmp(&a.last_seen));
        hosts
    }

    /// Get the count of globally known storage hosts (for UI display).
    pub async fn global_host_count(&self) -> usize {
        let guard = self.db.lock().await;
        if let Some(db) = guard.as_ref() {
            db.scan_prefix(b"global_host:").count()
        } else {
            0
        }
    }

    /// Emit a Tauri event with the current node inventory for UI display.
    pub async fn emit_node_inventory(&self, app: Option<&tauri::AppHandle>) {
        if let Some(app) = app {
            use tauri::Emitter;
            let hosts = self.get_all_global_hosts().await;
            let count = hosts.len();
            let host_names: Vec<String> = hosts.iter().map(|h| h.host.clone()).collect();
            let _ = app.emit(
                "qilin_nodes_updated",
                serde_json::json!({
                    "total_nodes": count,
                    "hosts": host_names,
                }),
            );
        }
    }

    async fn record_probe_failure(&self, uuid: &str, node: &StorageNode, reason: &str) {
        let mut updated = node.clone();
        updated.record_failure();
        self.persist_node_state(uuid, updated.clone()).await;
        self.invalidate_cached_hints_for_host(uuid, &updated.host)
            .await;
        println!(
            "[QilinNodeCache] Demoted node {} after {} (cooldown until {})",
            updated.host, reason, updated.cooldown_until
        );
    }

    async fn record_probe_failure_with_cooldown(
        &self,
        uuid: &str,
        node: &StorageNode,
        reason: &str,
        status_code: Option<u16>,
    ) {
        let mut updated = node.clone();
        let reason_lower = reason.to_ascii_lowercase();
        if status_code == Some(404) {
            updated.record_failure_with_cooldown(HTTP_404_COOLDOWN_SECS);
            println!(
                "[QilinNodeCache] Demoted node {} after {} (404 cooldown={}s until {})",
                updated.host, reason, HTTP_404_COOLDOWN_SECS, updated.cooldown_until
            );
        } else if reason_lower.contains("connect") {
            updated.record_failure_with_cooldown(CONNECT_ERROR_COOLDOWN_SECS);
            println!(
                "[QilinNodeCache] Demoted node {} after {} (connect cooldown={}s until {})",
                updated.host, reason, CONNECT_ERROR_COOLDOWN_SECS, updated.cooldown_until
            );
        } else {
            updated.record_failure();
            println!(
                "[QilinNodeCache] Demoted node {} after {} (cooldown until {})",
                updated.host, reason, updated.cooldown_until
            );
        }
        self.persist_node_state(uuid, updated.clone()).await;
        self.invalidate_cached_hints_for_host(uuid, &updated.host)
            .await;
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
        let _ = self.save_redirect_hint(uuid, &updated).await;
        let _ = self.save_redirect_ring_sample(uuid, &updated).await;
        updated
    }

    pub async fn confirm_listing_root(
        &self,
        uuid: &str,
        listing_url: &str,
        latency_ms: u64,
    ) -> Option<StorageNode> {
        let parsed = reqwest::Url::parse(listing_url).ok()?;
        let host = parsed.host_str()?.to_string();
        let mut updated = self.get_node(uuid, &host).await.unwrap_or(StorageNode {
            url: listing_url.to_string(),
            host,
            last_seen: now_unix(),
            avg_latency_ms: 0,
            hit_count: 0,
            success_count: 0,
            failure_count: 0,
            failure_streak: 0,
            cooldown_until: 0,
            ..StorageNode::default()
        });
        updated.url = listing_url.to_string();
        updated.record_success(latency_ms.max(1));
        self.persist_node_state(uuid, updated.clone()).await;
        let _ = self.save_redirect_hint(uuid, &updated).await;
        let _ = self.save_redirect_ring_sample(uuid, &updated).await;
        let _ = self.save_winner_lease(uuid, &updated).await;
        println!(
            "[QilinNodeCache] Durable winner confirmed for {} via {} ({}ms)",
            uuid, updated.host, latency_ms
        );
        Some(updated)
    }

    pub async fn record_winner_run_outcome(
        &self,
        uuid: &str,
        listing_url: &str,
        effective_entries: usize,
        completion_ms: u64,
        failovers: usize,
        late_throttles: usize,
    ) -> Option<StorageNode> {
        let parsed = reqwest::Url::parse(listing_url).ok()?;
        let host = parsed.host_str()?.to_string();
        let mut updated = self.get_node(uuid, &host).await.unwrap_or(StorageNode {
            url: listing_url.to_string(),
            host,
            last_seen: now_unix(),
            avg_latency_ms: 0,
            hit_count: 0,
            success_count: 0,
            failure_count: 0,
            failure_streak: 0,
            cooldown_until: 0,
            ..StorageNode::default()
        });
        updated.url = listing_url.to_string();
        updated.last_seen = now_unix();
        updated.record_winner_run(
            effective_entries as u64,
            completion_ms,
            failovers as u32,
            late_throttles as u32,
        );
        self.persist_node_state(uuid, updated.clone()).await;
        let _ = self.save_redirect_hint(uuid, &updated).await;
        let _ = self.save_redirect_ring_sample(uuid, &updated).await;
        let _ = self.save_winner_lease(uuid, &updated).await;
        Some(updated)
    }

    pub async fn probe_until_first_valid_listing(
        &self,
        uuid: &str,
        client: &crate::arti_client::ArtiClient,
        candidates: Vec<StorageNode>,
        wave_size: usize,
        app: Option<&tauri::AppHandle>,
    ) -> Option<StorageNode> {
        let mut seen_hosts = std::collections::HashSet::new();
        let mut filtered = Vec::new();
        for node in candidates {
            if seen_hosts.insert(node.host.clone()) {
                filtered.push(node);
            }
        }

        if filtered.is_empty() {
            emit_discovery_progress(
                app,
                "Stage D",
                "No retryable listing candidates remain after filtering.",
            );
            return None;
        }

        emit_discovery_progress(
            app,
            "Stage D",
            &format!(
                "Prioritized probing {} candidates (redirect > known-good > top-ranked)",
                filtered.len()
            ),
        );

        let wave_size = wave_size.max(1);
        let total_candidates = filtered.len();
        let total_waves = total_candidates.div_ceil(wave_size);

        for (wave_idx, chunk) in filtered.chunks(wave_size).enumerate() {
            let wave_nodes: Vec<StorageNode> = chunk.to_vec();
            emit_discovery_progress(
                app,
                "Stage D",
                &format!(
                    "Wave {}/{} probing {} candidates",
                    wave_idx + 1,
                    total_waves.max(1),
                    wave_nodes.len()
                ),
            );

            let mut wave = tokio::task::JoinSet::new();
            for node in wave_nodes {
                let cache = self.clone();
                let uuid_owned = uuid.to_string();
                let probe_client = client.new_isolated();
                wave.spawn(async move {
                    cache
                        .probe_candidate_listing(
                            &uuid_owned,
                            probe_client,
                            node,
                            PROBE_TIMEOUT_SECS,
                            true,
                            None,
                        )
                        .await
                });
            }

            // Phase 126C: Hard outer deadline to handle Arti cancellation-safety issue
            let wave_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(40);
            loop {
                match tokio::time::timeout_at(wave_deadline, wave.join_next()).await {
                    Ok(Some(Ok(Some(winner)))) => {
                        wave.abort_all();
                        return Some(winner);
                    }
                    Ok(Some(_)) => continue, // Task returned None or JoinError
                    Ok(None) => break,       // All tasks completed
                    Err(_) => {
                        println!("[QilinNodeCache] Wave probe deadline exceeded (40s) — aborting remaining");
                        wave.abort_all();
                        break;
                    }
                }
            }
        }

        None
    }

    pub async fn try_fast_cached_route(
        &self,
        uuid: &str,
        client: &crate::arti_client::ArtiClient,
        app: Option<&tauri::AppHandle>,
    ) -> Option<StorageNode> {
        let winner_lease = self.get_winner_lease(uuid).await;
        let redirect_hint = self.get_redirect_hint(uuid).await;
        let redirect_ring = self.get_redirect_ring(uuid).await;
        let candidates =
            Self::rank_fast_path_candidates(winner_lease, redirect_hint, redirect_ring);

        if candidates.len() >= FAST_PATH_PROBE_LIMIT {
            emit_discovery_progress(
                app,
                "Fast Path",
                &format!(
                    "Probe budget reached ({} candidates). Falling back to fresh Stage A after these checks.",
                    FAST_PATH_PROBE_LIMIT
                ),
            );
        }

        for (idx, candidate) in candidates.into_iter().enumerate() {
            let detail = match idx {
                0 => format!("Probing cached winner candidate: {}", candidate.host),
                _ => format!("Probing freshest cached alternate: {}", candidate.host),
            };
            emit_discovery_progress(app, "Fast Path", &detail);
            if let Some(winner) = self
                .probe_candidate_listing(
                    uuid,
                    client.new_isolated(),
                    candidate,
                    HINT_PROBE_TIMEOUT_SECS,
                    false,
                    app.cloned(),
                )
                .await
            {
                with_runtime_telemetry(app, |telemetry| telemetry.record_cached_route_hit());
                return Some(winner);
            }
        }

        None
    }

    /// Pre-seed the cache with all known Qilin QData storage domains.
    /// These are the storage hosts that host the actual file data.
    /// Each gets paired with the UUID to form a probable URL.
    /// Phase 42: Expanded with all historically discovered nodes so they
    /// are always checked as fallback candidates during Stage D probing.
    pub async fn seed_known_mirrors(&self, uuid: &str) {
        let known_storage_hosts = vec![
            // === Phase 77D: pandora42btu REMOVED — it's the Pandora RaaS platform ===
            // === (separate ransomware group), NOT a Qilin storage node.             ===
            //
            // IMPORTANT: The CMS remaps victim UUIDs. The /<cms_uuid>/ path may not exist
            // on storage nodes. Stage A captures the real redirect URL with the correct
            // storage UUID. These hosts are probed during Stage D to find alive nodes.
            //
            // === Phase 77D: Newly discovered active nodes (2026-03-09 via 302 capture) ===
            "onlta6cik443t67n5zqlbbcmhepazzlonhsx27qidmpf6zha6bxsjcid.onion",
            "42hfjtvbstk472gbv42sxqqabupa5d2ow2mahc6zq4orpe4bpo63gcyd.onion",
            "5nqgp7hmstqsvlqu3wr6o5mg6twxpz3fvyiqyctxyx4hfoynqbm74qyd.onion",
            // === Previously known QData storage nodes ===
            "szgkpzhcrnshftjb5mtvd6bc5oep5yabmgfmwt7u3tiqzfikoew27hqd.onion",
            "25j35d6uf37tvfqt5pmz457yicgu35yhizojqxbfzv33dni2d73q3oad.onion",
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

        // Phase 77E: Register all hardcoded hosts into the global registry
        for host in &known_storage_hosts {
            self.register_discovered_host(host, "hardcoded").await;
        }

        // Phase 77E: Merge in dynamically discovered hosts from prior sessions
        let global_hosts = self.get_all_global_hosts().await;
        let mut all_hosts: Vec<String> =
            known_storage_hosts.iter().map(|s| s.to_string()).collect();
        for global in &global_hosts {
            if !all_hosts.contains(&global.host) {
                println!(
                    "[QilinNodeCache] Phase 77E — Auto-seeding previously discovered host: {} (first seen from UUID {})",
                    global.host, global.discovered_from
                );
                all_hosts.push(global.host.clone());
            }
        }

        let total = all_hosts.len();
        let seeded_nodes: Vec<StorageNode> = all_hosts
            .iter()
            .map(|host| StorageNode {
                url: format!("http://{}/{}/", host, uuid),
                host: host.clone(),
                last_seen: now_unix(),
                avg_latency_ms: 0,
                hit_count: 0,
                success_count: 0,
                failure_count: 0,
                failure_streak: 0,
                cooldown_until: 0,
                ..StorageNode::default()
            })
            .collect();
        let _ = self.save_seed_nodes_batch(uuid, &seeded_nodes).await;
        for node in &seeded_nodes {
            println!("[QilinNodeCache] Seeded: {} -> {}", uuid, node.url);
        }

        println!(
            "[QilinNodeCache] Phase 77E — Seeded {} hosts ({} hardcoded + {} auto-discovered)",
            total,
            known_storage_hosts.len(),
            total - known_storage_hosts.len()
        );
    }

    /// Phase 76D: Seed the CMS host into the storage pool as a last-resort fallback.
    /// Called after Stage B confirms the CMS is reachable. If all dedicated storage
    /// nodes are down, the CMS host might still serve autoindex at /<uuid>/.
    pub async fn seed_cms_host(&self, uuid: &str, cms_url: &str) {
        if let Ok(parsed) = reqwest::Url::parse(cms_url) {
            if let Some(host) = parsed.host_str() {
                let url = format!("http://{}/{}/", host, uuid);
                println!(
                    "[QilinNodeCache] Phase 76D — Seeding CMS host as fallback: {}",
                    host
                );
                self.seed_node(uuid, &url, host).await;
            }
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
        app: Option<&tauri::AppHandle>,
    ) -> Option<StorageNode> {
        println!(
            "[QilinNodeCache] Starting multi-path discovery for UUID: {}",
            uuid
        );
        emit_discovery_progress(
            app,
            "Init",
            &format!("Starting 4-stage discovery for {}", uuid),
        );

        let parsed = reqwest::Url::parse(cms_url).ok()?;
        let base = format!("{}://{}", parsed.scheme(), parsed.host_str()?);
        let mut stage_a_redirect_candidates = Vec::new();
        let mut stage_a_hosts = std::collections::HashSet::new();

        if let Some(winner) = self.try_fast_cached_route(uuid, client, app).await {
            return Some(winner);
        }

        let now = now_unix();
        let mut cached_nodes = self.get_nodes(uuid).await;
        let winner_lease = self.get_winner_lease(uuid).await;
        let redirect_hint = self.get_redirect_hint(uuid).await;
        let redirect_ring = self.get_redirect_ring(uuid).await;
        let last_known_good = cached_nodes
            .iter()
            .filter(|node| node.success_count > 0 || node.hit_count > 0)
            .max_by(|left, right| {
                left.tournament_score(now)
                    .partial_cmp(&right.tournament_score(now))
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| left.last_seen.cmp(&right.last_seen))
            })
            .cloned();
        let sticky_winner_candidate = winner_lease
            .clone()
            .filter(|node| node.qualifies_for_winner_stickiness())
            .or_else(|| {
                last_known_good
                    .clone()
                    .filter(|node| node.qualifies_for_winner_stickiness())
            });

        // Stage A: Capture the 302 redirect Location from /site/data WITHOUT following it
        // Phase 77D: The CMS remaps victim UUIDs — the storage path uses a DIFFERENT UUID
        // than the CMS. We must capture the raw redirect URL to get the correct path.
        let data_url = format!("{}/site/data?uuid={}", base, uuid);
        emit_discovery_progress(
            app,
            "Stage A",
            &format!("Capturing 302 redirect from {}", data_url),
        );

        for attempt in 1..=STAGE_A_SAMPLE_ATTEMPTS {
            match tokio::time::timeout(
                Duration::from_secs(STAGE_HTTP_TIMEOUT_SECS),
                client
                    .new_isolated()
                    .get(&data_url)
                    .header("Referer", &base)
                    .send_capturing_redirect(),
            )
            .await
            {
                Ok(Ok((resp, redirect_url))) => {
                    let status = resp.status();
                    if status.is_success() || status.is_redirection() {
                        record_discovery_success(app);
                    } else {
                        record_discovery_failure(app);
                    }
                    println!(
                        "[QilinNodeCache] Stage A — Status={}, RedirectTarget={:?}",
                        status, redirect_url
                    );

                    if let Some(ref target_url) = redirect_url {
                        // We have the actual storage URL with the correct remapped UUID!
                        println!(
                            "[QilinNodeCache] Stage A — 🎯 Captured storage redirect: {}",
                            target_url
                        );
                        if let Some(node) =
                            self.cache_redirect_candidate(uuid, target_url, app).await
                        {
                            if stage_a_hosts.insert(node.host.clone()) {
                                stage_a_redirect_candidates.push(node);
                            }
                        }
                    } else if status.as_u16() == 404 {
                        println!(
                            "[QilinNodeCache] Stage A — CMS returned 404 (victim may be delisted)"
                        );
                    } else {
                        println!(
                            "[QilinNodeCache] Stage A — No redirect received (status={})",
                            status
                        );
                    }
                    if stage_a_hosts.len() >= STAGE_A_TARGET_UNIQUE_HOSTS || status.as_u16() == 404
                    {
                        break;
                    }
                }
                Ok(Err(e)) => {
                    record_discovery_failure(app);
                    println!(
                        "[QilinNodeCache] Stage A — attempt {} failed: {}",
                        attempt, e
                    );
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
                Err(_) => {
                    record_discovery_failure(app);
                    println!(
                        "[QilinNodeCache] Stage A — attempt {} timed out ({}s)",
                        attempt, STAGE_HTTP_TIMEOUT_SECS
                    );
                }
            }

            if attempt < STAGE_A_SAMPLE_ATTEMPTS {
                tokio::time::sleep(Duration::from_millis(STAGE_A_INTER_ATTEMPT_DELAY_MS)).await;
            }
        }

        if !stage_a_redirect_candidates.is_empty() {
            let prioritized_stage_a = Self::rank_stage_a_candidates_with_sticky_winner(
                sticky_winner_candidate.clone(),
                stage_a_redirect_candidates.clone(),
                now,
            );
            emit_discovery_progress(
                app,
                "Stage A",
                &format!(
                    "Captured {} fresh redirect candidates. Validating them against the best sticky winner first when it has proven productive.",
                    stage_a_redirect_candidates.len()
                ),
            );
            if let Some(winner) = self
                .probe_until_first_valid_listing(
                    uuid,
                    client,
                    prioritized_stage_a,
                    PROBE_WAVE_SIZE,
                    app,
                )
                .await
            {
                return Some(winner);
            }
        }

        // Stage B: Scrape the view page for the QData storage reference
        let view_url = format!("{}/site/view?uuid={}", base, uuid);
        emit_discovery_progress(app, "Stage B", &format!("Scraping view page: {}", view_url));

        if let Ok(Ok(resp)) = tokio::time::timeout(
            Duration::from_secs(STAGE_HTTP_TIMEOUT_SECS),
            client.get(&view_url).send(),
        )
        .await
        {
            if resp.status().is_success() {
                record_discovery_success(app);
            } else {
                record_discovery_failure(app);
            }
            if let Ok(body) = resp.text().await {
                for target in crate::adapters::qilin::extract_watch_data_targets(&base, &body) {
                    if let Some(node) = self
                        .follow_watch_data_target(client, &view_url, uuid, &target, app)
                        .await
                    {
                        return Some(node);
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
                        // Phase 77D: Skip known non-QData .onion addresses that appear
                        // in CMS footer/affiliate links (e.g. Pandora RaaS, WikiLeaks mirrors)
                        const BLOCKLIST: &[&str] =
                            &["pandora42btuwlldza4uthk4bssbtsv47y4t5at5mo4ke3h4nqveobyd.onion"];
                        if BLOCKLIST.contains(&onion_host.as_str()) {
                            println!(
                                "[QilinNodeCache] Stage B — Skipping known non-QData host: {}",
                                onion_host
                            );
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
                            ..StorageNode::default()
                        };
                        let _ = self.save_seed_nodes_batch(uuid, &[node]).await;
                    }
                }

                // Also check for the data link which may contain a different onion ref
                if body.contains("site/data") {
                    println!("[QilinNodeCache] Stage B — View page has 'Watch data' link (data available)");
                    // Phase 76D: CMS is confirmed reachable — seed it as fallback storage
                    self.seed_cms_host(uuid, &view_url).await;
                }
            }
        } else {
            record_discovery_failure(app);
        }

        // Stage C: Load all cached nodes
        if cached_nodes.len() < PRIORITIZED_MIRROR_LIMIT {
            emit_discovery_progress(
                app,
                "Stage C",
                &format!(
                    "Only {} cached nodes available. Lazy-seeding fallback mirrors.",
                    cached_nodes.len()
                ),
            );
            self.seed_known_mirrors(uuid).await;
            cached_nodes = self.get_nodes(uuid).await;
        }
        emit_discovery_progress(
            app,
            "Stage C",
            &format!("{} cached nodes loaded", cached_nodes.len()),
        );

        if cached_nodes.is_empty() {
            println!("[QilinNodeCache] No nodes discovered for UUID {}", uuid);
            return None;
        }

        let fresh_redirect_candidates = stage_a_redirect_candidates.len();
        let stale_host_only_candidates = cached_nodes
            .iter()
            .filter(|node| Self::is_host_only_seed_node(uuid, node))
            .count();
        let retryable_cached_candidates = cached_nodes
            .iter()
            .filter(|node| Self::is_retryable_listing_node(uuid, node))
            .count();

        with_runtime_telemetry(app, |telemetry| {
            telemetry.set_qilin_discovery_candidate_mix(
                fresh_redirect_candidates,
                stale_host_only_candidates,
            )
        });
        emit_discovery_progress(
            app,
            "Stage C",
            &format!(
                "Candidate mix: fresh_redirect={} retryable_cached={} stale_host_only={}",
                fresh_redirect_candidates,
                retryable_cached_candidates,
                stale_host_only_candidates
            ),
        );

        let degraded_stage_d = fresh_redirect_candidates == 0;
        if degraded_stage_d {
            emit_discovery_progress(
                app,
                "Stage D",
                &format!(
                    "No fresh redirect captured. Entering degraded Stage D with wider probing across {} cached listing URLs.",
                    retryable_cached_candidates
                ),
            );
            with_runtime_telemetry(app, |telemetry| {
                telemetry.record_qilin_degraded_stage_d_activation()
            });
        }

        let ranked_limit = if degraded_stage_d {
            DEGRADED_PRIORITIZED_MIRROR_LIMIT
        } else {
            PRIORITIZED_MIRROR_LIMIT
        };
        let probe_wave_size = if degraded_stage_d {
            DEGRADED_PROBE_WAVE_SIZE
        } else {
            PROBE_WAVE_SIZE
        };

        let first_stable_candidate = winner_lease
            .clone()
            .or_else(|| sticky_winner_candidate.clone())
            .or_else(|| last_known_good.clone())
            .or_else(|| redirect_hint.clone())
            .or_else(|| redirect_ring.first().cloned());

        let mut prioritized = Vec::new();
        let mut ordered_stage_a = Self::rank_stage_a_candidates_with_sticky_winner(
            sticky_winner_candidate.clone(),
            stage_a_redirect_candidates.clone(),
            now,
        );
        if let Some(first_redirect) = ordered_stage_a.first().cloned() {
            if Self::is_retryable_listing_node(uuid, &first_redirect) {
                prioritized.push(first_redirect);
            }
        }
        if let Some(stable_candidate) = first_stable_candidate {
            if Self::is_retryable_listing_node(uuid, &stable_candidate) {
                prioritized.push(stable_candidate);
            }
        }
        if ordered_stage_a.len() > 1 {
            prioritized.extend(
                ordered_stage_a
                    .drain(1..)
                    .filter(|node| Self::is_retryable_listing_node(uuid, node)),
            );
        }
        if let Some(candidate) = winner_lease {
            if Self::is_retryable_listing_node(uuid, &candidate) {
                prioritized.push(candidate);
            }
        }
        if let Some(candidate) = last_known_good {
            if Self::is_retryable_listing_node(uuid, &candidate) {
                prioritized.push(candidate);
            }
        }
        if let Some(candidate) = redirect_hint {
            if Self::is_retryable_listing_node(uuid, &candidate) {
                prioritized.push(candidate);
            }
        }
        prioritized.extend(
            redirect_ring
                .into_iter()
                .filter(|node| Self::is_retryable_listing_node(uuid, node)),
        );

        let mut ranked: Vec<StorageNode> = cached_nodes
            .iter()
            .filter(|node| !node.is_cooling_down(now))
            .filter(|node| Self::is_retryable_listing_node(uuid, node))
            .cloned()
            .collect();
        ranked.sort_by(|a, b| {
            b.tournament_score(now)
                .partial_cmp(&a.tournament_score(now))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        prioritized.extend(ranked.into_iter().take(ranked_limit));

        if prioritized.is_empty() && stale_host_only_candidates > 0 {
            emit_discovery_progress(
                app,
                "Stage D",
                &format!(
                    "Skipping {} stale host-only candidates because /<cms_uuid>/ retries are banned.",
                    stale_host_only_candidates
                ),
            );
        }

        self.probe_until_first_valid_listing(uuid, client, prioritized, probe_wave_size, app)
            .await
    }

    pub async fn discover_and_resolve_prioritized(
        &self,
        cms_url: &str,
        uuid: &str,
        client: &crate::arti_client::ArtiClient,
        app: Option<&tauri::AppHandle>,
    ) -> Option<StorageNode> {
        self.discover_and_resolve(cms_url, uuid, client, app).await
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
    use super::{QilinNodeCache, StorageNode};

    async fn test_cache() -> QilinNodeCache {
        let cache = QilinNodeCache::default();
        let unique = format!(
            "qilin_nodes_test_{}_{}",
            std::process::id(),
            super::now_unix()
        );
        let path = std::env::temp_dir().join(unique);
        let db = sled::open(path).expect("open sled test db");
        let mut guard = cache.db.lock().await;
        *guard = Some(db);
        drop(guard);
        cache
    }

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
            ..StorageNode::default()
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
            ..StorageNode::default()
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
            ..StorageNode::default()
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

    #[test]
    fn merged_seed_node_preserves_history_and_prefers_specific_url() {
        let existing = StorageNode {
            url: "http://a.onion/site/view?uuid=test".to_string(),
            host: "a.onion".to_string(),
            last_seen: 55,
            avg_latency_ms: 1_500,
            hit_count: 3,
            success_count: 2,
            failure_count: 1,
            failure_streak: 0,
            cooldown_until: 0,
            ..StorageNode::default()
        };
        let seed = StorageNode {
            url: "http://a.onion/remapped-uuid/".to_string(),
            host: "a.onion".to_string(),
            last_seen: 100,
            avg_latency_ms: 0,
            hit_count: 0,
            success_count: 0,
            failure_count: 0,
            failure_streak: 0,
            cooldown_until: 0,
            ..StorageNode::default()
        };

        let merged = super::QilinNodeCache::merged_seed_node("test", seed, Some(existing.clone()));

        assert_eq!(merged.url, "http://a.onion/remapped-uuid/");
        assert_eq!(merged.avg_latency_ms, existing.avg_latency_ms);
        assert_eq!(merged.hit_count, existing.hit_count);
        assert_eq!(merged.success_count, existing.success_count);
        assert_eq!(merged.failure_count, existing.failure_count);
    }

    #[test]
    fn fast_path_candidates_dedupe_and_respect_probe_budget() {
        let winner = StorageNode {
            url: "http://winner.onion/uuid/".to_string(),
            host: "winner.onion".to_string(),
            last_seen: 10,
            avg_latency_ms: 800,
            hit_count: 4,
            success_count: 4,
            failure_count: 0,
            failure_streak: 0,
            cooldown_until: 0,
            ..StorageNode::default()
        };
        let duplicate_hint = StorageNode {
            url: "http://winner.onion/uuid/".to_string(),
            host: "winner.onion".to_string(),
            last_seen: 9,
            avg_latency_ms: 900,
            hit_count: 2,
            success_count: 2,
            failure_count: 0,
            failure_streak: 0,
            cooldown_until: 0,
            ..StorageNode::default()
        };
        let ring = vec![
            StorageNode {
                url: "http://fresh-alt.onion/uuid/".to_string(),
                host: "fresh-alt.onion".to_string(),
                last_seen: 11,
                avg_latency_ms: 1_100,
                hit_count: 1,
                success_count: 1,
                failure_count: 0,
                failure_streak: 0,
                cooldown_until: 0,
                ..StorageNode::default()
            },
            StorageNode {
                url: "http://older-alt.onion/uuid/".to_string(),
                host: "older-alt.onion".to_string(),
                last_seen: 8,
                avg_latency_ms: 1_400,
                hit_count: 1,
                success_count: 1,
                failure_count: 0,
                failure_streak: 0,
                cooldown_until: 0,
                ..StorageNode::default()
            },
        ];

        let ranked =
            QilinNodeCache::rank_fast_path_candidates(Some(winner), Some(duplicate_hint), ring);

        assert_eq!(ranked.len(), super::FAST_PATH_PROBE_LIMIT);
        assert_eq!(ranked[0].host, "winner.onion");
        assert_eq!(ranked[1].host, "fresh-alt.onion");
    }

    #[test]
    fn listing_url_classification_distinguishes_remapped_and_host_only_paths() {
        assert!(QilinNodeCache::has_specific_listing_url(
            "cms-uuid",
            "http://winner.onion/remapped-storage-uuid/"
        ));
        assert!(!QilinNodeCache::has_specific_listing_url(
            "cms-uuid",
            "http://winner.onion/cms-uuid/"
        ));
        assert!(QilinNodeCache::is_host_only_seed_url(
            "cms-uuid",
            "http://winner.onion/cms-uuid/"
        ));
        assert!(!QilinNodeCache::is_host_only_seed_url(
            "cms-uuid",
            "http://winner.onion/remapped-storage-uuid/"
        ));
    }

    #[tokio::test]
    async fn winner_lease_requires_durable_root_confirmation() {
        let cache = test_cache().await;
        cache
            .seed_node(
                "uuid-test",
                "http://stablehostabcdefghijklmnopqrstuvwxyabcdefghijklmnop.onion/uuid-test/",
                "stablehostabcdefghijklmnopqrstuvwxyabcdefghijklmnop.onion",
            )
            .await;

        let probed = cache
            .record_probe_success(
                "uuid-test",
                &StorageNode {
                    url: "http://stablehostabcdefghijklmnopqrstuvwxyabcdefghijklmnop.onion/uuid-test/"
                        .to_string(),
                    host: "stablehostabcdefghijklmnopqrstuvwxyabcdefghijklmnop.onion".to_string(),
                    last_seen: 0,
                    avg_latency_ms: 0,
                    hit_count: 0,
                    success_count: 0,
                    failure_count: 0,
                    failure_streak: 0,
                    cooldown_until: 0,
                    ..StorageNode::default()
                },
                1_200,
            )
            .await;

        assert_eq!(
            probed.host,
            "stablehostabcdefghijklmnopqrstuvwxyabcdefghijklmnop.onion"
        );
        assert!(cache.get_redirect_hint("uuid-test").await.is_some());
        assert!(cache.get_winner_lease("uuid-test").await.is_none());

        let confirmed = cache
            .confirm_listing_root(
                "uuid-test",
                "http://stablehostabcdefghijklmnopqrstuvwxyabcdefghijklmnop.onion/uuid-test/",
                950,
            )
            .await
            .expect("winner confirmation");

        assert_eq!(
            confirmed.host,
            "stablehostabcdefghijklmnopqrstuvwxyabcdefghijklmnop.onion"
        );
        assert!(cache.get_winner_lease("uuid-test").await.is_some());
    }

    #[test]
    fn winner_quality_biases_ranking_and_repin_interval() {
        let now = 1_700_000_000;
        let mut productive = StorageNode {
            url: "http://productive.onion/test/".to_string(),
            host: "productive.onion".to_string(),
            last_seen: now,
            avg_latency_ms: 1_400,
            hit_count: 4,
            success_count: 4,
            failure_count: 0,
            failure_streak: 0,
            cooldown_until: 0,
            ..StorageNode::default()
        };
        productive.record_winner_run(3_180, 140_000, 0, 0);

        let mut unstable = StorageNode {
            url: "http://unstable.onion/test/".to_string(),
            host: "unstable.onion".to_string(),
            last_seen: now,
            avg_latency_ms: 1_000,
            hit_count: 6,
            success_count: 6,
            failure_count: 0,
            failure_streak: 0,
            cooldown_until: 0,
            ..StorageNode::default()
        };
        unstable.record_winner_run(900, 410_000, 3, 2);

        assert!(productive.tournament_score(now) > unstable.tournament_score(now));
        assert!(productive.recommended_repin_interval() > unstable.recommended_repin_interval());
    }

    #[test]
    fn sticky_winner_probes_before_fresh_stage_a_candidate() {
        let now = 1_700_000_000;
        let mut sticky = StorageNode {
            url: "http://sticky.onion/root/".to_string(),
            host: "sticky.onion".to_string(),
            last_seen: now,
            avg_latency_ms: 1_250,
            hit_count: 5,
            success_count: 5,
            failure_count: 0,
            failure_streak: 0,
            cooldown_until: 0,
            ..StorageNode::default()
        };
        sticky.record_winner_run(3_180, 145_000, 0, 0);

        let fresh = StorageNode {
            url: "http://fresh.onion/root/".to_string(),
            host: "fresh.onion".to_string(),
            last_seen: now,
            avg_latency_ms: 0,
            hit_count: 0,
            success_count: 0,
            failure_count: 0,
            failure_streak: 0,
            cooldown_until: 0,
            ..StorageNode::default()
        };

        let ranked = QilinNodeCache::rank_stage_a_candidates_with_sticky_winner(
            Some(sticky.clone()),
            vec![fresh.clone()],
            now,
        );

        assert_eq!(
            ranked.first().map(|node| node.host.as_str()),
            Some("sticky.onion")
        );
        assert_eq!(
            ranked.get(1).map(|node| node.host.as_str()),
            Some("fresh.onion")
        );
    }
}
