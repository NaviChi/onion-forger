use crate::adapters::qilin_nodes::{QilinNodeCache, StorageNode};
use crate::adapters::{CrawlerAdapter, EntryType, FileEntry, SiteFingerprint};
use crate::frontier::CrawlerFrontier;
use crate::path_utils;
use crate::runtime_metrics::RuntimeTelemetry;
use crate::subtree_heatmap::{HeatFailureKind, SubtreeHeatmap};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{mpsc, Mutex};

// Phase 73: MacOS Darwin kqueue non-blocking spinlocks
#[inline(always)]
async fn darwin_kqueue_spinlock(ms: u64) {
    #[cfg(target_os = "macos")]
    {
        for _ in 0..ms {
            tokio::task::yield_now().await;
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
    }
}
#[derive(Default)]
pub struct QilinAdapter;

#[derive(Clone, Copy)]
enum CrawlFailureKind {
    Timeout,
    Circuit,
    Throttle,
    Http,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RetryLane {
    Primary,
    Degraded,
}

#[derive(Clone)]
struct RetryPayload {
    url: String,
    attempt: u8,
    unlock_timestamp: std::time::Instant,
}

struct DegradedLanePermit {
    in_flight: Arc<AtomicUsize>,
}

impl Drop for DegradedLanePermit {
    fn drop(&mut self) {
        self.in_flight.fetch_sub(1, Ordering::Release);
    }
}

fn env_usize(name: &str) -> Option<usize> {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
}

#[allow(dead_code)] // Phase 76: kept as utility for adapters that opt out of default-true
fn env_bool(name: &str) -> bool {
    matches!(
        std::env::var(name).ok().as_deref(),
        Some("1" | "true" | "TRUE" | "yes" | "YES")
    )
}

/// Phase 76: env_bool variant that defaults to true unless explicitly disabled.
fn env_bool_default_true(name: &str) -> bool {
    match std::env::var(name).ok().as_deref() {
        Some("0" | "false" | "FALSE" | "no" | "NO") => false,
        _ => true, // Default: enabled
    }
}

fn is_dumb_mode_enabled() -> bool {
    matches!(
        std::env::var("CRAWLI_DUMB_MODE").ok().as_deref(),
        Some("1" | "true" | "TRUE" | "yes" | "YES")
    )
}

fn legacy_cms_bypass_enabled() -> bool {
    env_bool("CRAWLI_QILIN_LEGACY_CMS_BYPASS")
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum QilinIngressKind {
    CmsLauncher,
    DirectListing,
    Unknown,
}

fn classify_qilin_ingress(url: &str) -> (Option<String>, QilinIngressKind) {
    if url.contains("/site/view") || url.contains("/site/data") {
        let uuid = url.find("uuid=").map(|start| {
            url[start + 5..]
                .split('&')
                .next()
                .unwrap_or("")
                .trim_end_matches('/')
                .to_string()
        });
        return (
            uuid.filter(|uuid| !uuid.is_empty()),
            QilinIngressKind::CmsLauncher,
        );
    }

    let uuid = url
        .trim_end_matches('/')
        .split('/')
        .next_back()
        .filter(|segment| segment.len() == 36 && segment.chars().filter(|c| *c == '-').count() == 4)
        .map(|segment| segment.to_string());

    if uuid.is_some() {
        (uuid, QilinIngressKind::DirectListing)
    } else {
        (None, QilinIngressKind::Unknown)
    }
}

#[derive(Clone)]
struct QilinRoutePlan {
    active_seed_url: Arc<tokio::sync::RwLock<String>>,
    standby_seed_urls: Arc<Vec<String>>,
    next_failover_idx: Arc<AtomicUsize>,
    subtree_route_health: Arc<std::sync::Mutex<HashMap<String, SubtreeRouteHealth>>>,
    subtree_summary_path: Option<PathBuf>,
    telemetry: Option<RuntimeTelemetry>,
}

#[derive(Clone, Default)]
struct SubtreeRouteHealth {
    preferred_seed_url: Option<String>,
    host_health: HashMap<String, SubtreeHostHealth>,
}

#[derive(Clone, Default)]
struct SubtreeHostHealth {
    success_count: u32,
    failure_count: u32,
    consecutive_failures: u32,
    quarantine_until: u64,
    last_success_epoch: u64,
    last_failure_epoch: u64,
}

#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedSubtreeRouteSummary {
    updated_at_epoch: u64,
    winner_host: Option<String>,
    entries: Vec<PersistedSubtreeRouteEntry>,
}

#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedSubtreeRouteEntry {
    subtree_key: String,
    preferred_host: String,
    success_count: u32,
    last_success_epoch: u64,
}

const SUBTREE_STANDBY_QUARANTINE_BASE_SECS: u64 = 90;
const SUBTREE_STANDBY_QUARANTINE_MAX_SECS: u64 = 15 * 60;

impl QilinRoutePlan {
    fn new(
        primary_seed_url: String,
        standby_seed_urls: Vec<String>,
        telemetry: Option<RuntimeTelemetry>,
        subtree_summary_path: Option<PathBuf>,
    ) -> Self {
        Self {
            active_seed_url: Arc::new(tokio::sync::RwLock::new(primary_seed_url)),
            standby_seed_urls: Arc::new(standby_seed_urls),
            next_failover_idx: Arc::new(AtomicUsize::new(0)),
            subtree_route_health: Arc::new(std::sync::Mutex::new(HashMap::new())),
            subtree_summary_path,
            telemetry,
        }
    }

    fn worker_node_url(&self, worker_idx: usize) -> String {
        let standbys = self.standby_seed_urls.clone();
        let mut all = vec![self.current_seed_url_sync()];
        all.extend(standbys.iter().cloned());
        if all.is_empty() {
            return String::new();
        }
        let node_idx = worker_idx % all.len();
        all[node_idx].clone()
    }

    async fn current_seed_url(&self) -> String {
        self.active_seed_url.read().await.clone()
    }

    fn current_seed_url_sync(&self) -> String {
        // Use try_read to avoid panicking inside async runtime
        match self.active_seed_url.try_read() {
            Ok(guard) => guard.clone(),
            Err(_) => String::new(), // fallback: caller will retry
        }
    }

    fn record_subtree_quarantine_hit(&self) {
        if let Some(telemetry) = &self.telemetry {
            telemetry.record_subtree_quarantine_hit();
        }
    }

    fn record_subtree_reroute(&self) {
        if let Some(telemetry) = &self.telemetry {
            telemetry.record_subtree_reroute();
        }
    }

    fn record_request_route(&self, active_seed_url: &str, request_url: &str) {
        let Some((request_seed, subtree_key)) = split_qilin_seed_and_relative_path(request_url)
        else {
            return;
        };
        if subtree_key.is_empty() || request_seed == active_seed_url {
            return;
        }
        if let Some(telemetry) = &self.telemetry {
            telemetry.record_off_winner_child_request();
        }
    }

    fn seed_urls_by_host(&self) -> HashMap<String, String> {
        let mut routes = HashMap::new();
        let active_seed = self.current_seed_url_sync();
        if let Some(host) = seed_host(&active_seed) {
            routes.entry(host).or_insert(active_seed);
        }
        for seed in self.standby_seed_urls.iter() {
            if let Some(host) = seed_host(seed) {
                routes.entry(host).or_insert_with(|| seed.clone());
            }
        }
        routes
    }

    fn load_persisted_subtree_preferences(&self) -> anyhow::Result<usize> {
        let Some(path) = self.subtree_summary_path.as_ref() else {
            return Ok(0);
        };
        if !path.exists() {
            return Ok(0);
        }

        let data = std::fs::read(path)?;
        let summary = serde_json::from_slice::<PersistedSubtreeRouteSummary>(&data)?;
        let routes_by_host = self.seed_urls_by_host();
        if routes_by_host.is_empty() {
            return Ok(0);
        }

        let mut loaded = 0usize;
        let mut state_guard = match self.subtree_route_health.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        for entry in summary.entries {
            if entry.subtree_key.is_empty() || entry.preferred_host.is_empty() {
                continue;
            }
            let Some(seed_url) = routes_by_host.get(&entry.preferred_host).cloned() else {
                continue;
            };
            let state = state_guard.entry(entry.subtree_key).or_default();
            state.preferred_seed_url = Some(seed_url.clone());
            let host_state = state.host_health.entry(seed_url).or_default();
            host_state.success_count = host_state.success_count.max(entry.success_count);
            host_state.last_success_epoch =
                host_state.last_success_epoch.max(entry.last_success_epoch);
            loaded = loaded.saturating_add(1);
        }
        Ok(loaded)
    }

    fn persist_subtree_preferences(&self) -> anyhow::Result<usize> {
        let Some(path) = self.subtree_summary_path.as_ref() else {
            return Ok(0);
        };

        let state_guard = match self.subtree_route_health.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        let mut entries = Vec::new();
        for (subtree_key, state) in state_guard.iter() {
            let Some(seed_url) = state.preferred_seed_url.as_ref() else {
                continue;
            };
            let Some(host_state) = state.host_health.get(seed_url) else {
                continue;
            };
            if host_state.success_count == 0 {
                continue;
            }
            let Some(host) = seed_host(seed_url) else {
                continue;
            };
            entries.push(PersistedSubtreeRouteEntry {
                subtree_key: subtree_key.clone(),
                preferred_host: host,
                success_count: host_state.success_count,
                last_success_epoch: host_state.last_success_epoch,
            });
        }
        drop(state_guard);

        if entries.is_empty() {
            return Ok(0);
        }

        entries.sort_by(|left, right| left.subtree_key.cmp(&right.subtree_key));
        let summary = PersistedSubtreeRouteSummary {
            updated_at_epoch: now_unix_secs(),
            winner_host: seed_host(&self.current_seed_url_sync()),
            entries,
        };
        let payload = serde_json::to_vec_pretty(&summary)?;
        std::fs::write(path, payload)?;
        Ok(summary.entries.len())
    }

    fn preferred_seed_for_request(
        &self,
        worker_idx: usize,
        request_url: &str,
        attempt: u8,
    ) -> Option<String> {
        let (current_seed, subtree_key) = split_qilin_seed_and_relative_path(request_url)?;
        if subtree_key.is_empty() || attempt <= 2 {
            return None;
        }

        let now = now_unix_secs();
        let active_seed = self.current_seed_url_sync();
        let state_guard = match self.subtree_route_health.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        let state = state_guard.get(&subtree_key)?;

        if let Some(preferred_seed) = state.preferred_seed_url.as_ref() {
            if preferred_seed != &current_seed {
                if state.seed_is_quarantined(preferred_seed, now) {
                    self.record_subtree_quarantine_hit();
                } else {
                    return Some(preferred_seed.clone());
                }
            }
        }

        let mut candidates = Vec::new();
        for seed in self.ranked_subtree_candidates(
            &state,
            &active_seed,
            &current_seed,
            now,
            state.preferred_seed_url.as_deref(),
        ) {
            if seed != current_seed {
                candidates.push(seed);
            }
        }

        if candidates.is_empty() {
            return None;
        }

        Some(candidates[worker_idx % candidates.len()].clone())
    }

    fn record_request_success(&self, request_url: &str) {
        let Some((seed_url, subtree_key)) = split_qilin_seed_and_relative_path(request_url) else {
            return;
        };
        if subtree_key.is_empty() {
            return;
        }

        let now = now_unix_secs();
        let mut state_guard = match self.subtree_route_health.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        let state = state_guard.entry(subtree_key).or_default();
        let host_state = state.host_health.entry(seed_url.clone()).or_default();
        host_state.success_count = host_state.success_count.saturating_add(1);
        host_state.consecutive_failures = 0;
        host_state.quarantine_until = 0;
        host_state.last_success_epoch = now;
        state.preferred_seed_url = Some(seed_url);
    }

    async fn retry_url_for_failure(
        &self,
        failed_url: &str,
        failure_kind: CrawlFailureKind,
        attempt: u8,
        app: Option<&AppHandle>,
    ) -> Option<String> {
        let Some((failed_seed, subtree_key)) = split_qilin_seed_and_relative_path(failed_url)
        else {
            return None;
        };
        let required_attempts = match failure_kind {
            CrawlFailureKind::Throttle => 2, // Fast failover for 403/400 DDoS protection
            CrawlFailureKind::Timeout | CrawlFailureKind::Circuit => 4,
            CrawlFailureKind::Http => 5, // Fallback even for HTTP errors eventually
        };

        if attempt < required_attempts {
            return None;
        }

        let current_root_seed = self.current_seed_url().await;
        let is_root_failure =
            subtree_key.is_empty() || is_root_retry_url(&current_root_seed, failed_url);
        if is_root_failure {
            return self
                .global_failover_url(failed_url, failure_kind, attempt, app)
                .await;
        }

        let active_seed = self.current_seed_url_sync();
        let now = now_unix_secs();
        let mut state_guard = match self.subtree_route_health.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        let state = state_guard.entry(subtree_key.clone()).or_default();
        let current_health = state.host_health.entry(failed_seed.clone()).or_default();
        current_health.failure_count = current_health.failure_count.saturating_add(1);
        current_health.consecutive_failures = current_health.consecutive_failures.saturating_add(1);
        current_health.last_failure_epoch = now;
        if failed_seed != active_seed {
            current_health.quarantine_until = now.saturating_add(subtree_standby_quarantine_secs(
                failure_kind,
                current_health.consecutive_failures,
            ));
            if state.preferred_seed_url.as_deref() == Some(failed_seed.as_str()) {
                state.preferred_seed_url = None;
            }
        }

        let next_seed = self
            .ranked_subtree_candidates(state, &active_seed, &failed_seed, now, None)
            .into_iter()
            .find(|seed| seed != &failed_seed)?;

        if next_seed == failed_seed {
            return None;
        }

        if let Ok(parsed) = reqwest::Url::parse(&next_seed) {
            if let Some(host) = parsed.host_str() {
                if let Some(telemetry) = &self.telemetry {
                    telemetry.record_failover(host.to_string());
                }
            }
        }

        let remapped = remap_seed_url(failed_url, &failed_seed, &next_seed);
        if let Some(app) = app {
            let _ = app.emit(
                "log",
                format!(
                    "[Qilin] Subtree reroute engaged after {} on attempt {}. Re-routing subtree {} from {} -> {}",
                    failure_kind_label(failure_kind),
                    attempt,
                    subtree_key,
                    failed_seed,
                    next_seed
                ),
            );
        }
        self.record_subtree_reroute();
        Some(remapped)
    }

    async fn global_failover_url(
        &self,
        failed_url: &str,
        failure_kind: CrawlFailureKind,
        attempt: u8,
        app: Option<&AppHandle>,
    ) -> Option<String> {
        let current_seed = self.current_seed_url().await;
        if !failed_url.starts_with(&current_seed) {
            return None;
        }

        let next_idx = self.next_failover_idx.fetch_add(1, Ordering::Relaxed);
        let next_seed = self.standby_seed_urls.get(next_idx)?.clone();
        if next_seed == current_seed {
            return None;
        }

        {
            let mut guard = self.active_seed_url.write().await;
            *guard = next_seed.clone();
        }

        if let Ok(parsed) = reqwest::Url::parse(&next_seed) {
            if let Some(host) = parsed.host_str() {
                if let Some(telemetry) = &self.telemetry {
                    telemetry.record_failover(host.to_string());
                }
            }
        }

        let remapped = remap_seed_url(failed_url, &current_seed, &next_seed);
        if let Some(app) = app {
            let _ = app.emit(
                "log",
                format!(
                    "[Qilin] Storage failover engaged after {} on attempt {}. Re-routing {} -> {}",
                    failure_kind_label(failure_kind),
                    attempt,
                    current_seed,
                    next_seed
                ),
            );
        }
        Some(remapped)
    }

    fn ranked_subtree_candidates(
        &self,
        state: &SubtreeRouteHealth,
        active_seed: &str,
        current_seed: &str,
        now: u64,
        excluded_seed: Option<&str>,
    ) -> Vec<String> {
        let mut candidates = Vec::new();
        if let Some(preferred_seed) = state.preferred_seed_url.as_ref() {
            if Some(preferred_seed.as_str()) != excluded_seed && preferred_seed != current_seed {
                if state.seed_is_quarantined(preferred_seed, now) {
                    self.record_subtree_quarantine_hit();
                } else {
                    candidates.push(preferred_seed.clone());
                }
            }
        }
        if Some(active_seed) != excluded_seed && active_seed != current_seed {
            if state.seed_is_quarantined(active_seed, now) {
                self.record_subtree_quarantine_hit();
            } else {
                candidates.push(active_seed.to_string());
            }
        }
        for seed in self.standby_seed_urls.iter() {
            if Some(seed.as_str()) == excluded_seed || seed == current_seed {
                continue;
            }
            if state.seed_is_quarantined(seed, now) {
                self.record_subtree_quarantine_hit();
                continue;
            }
            candidates.push(seed.clone());
        }

        candidates.sort_by(|left, right| {
            state
                .seed_score(right, active_seed, now)
                .cmp(&state.seed_score(left, active_seed, now))
                .then_with(|| left.cmp(right))
        });
        candidates.dedup();
        candidates
    }
}

impl SubtreeRouteHealth {
    fn seed_is_quarantined(&self, seed_url: &str, now: u64) -> bool {
        self.host_health
            .get(seed_url)
            .map(|health| health.quarantine_until > now)
            .unwrap_or(false)
    }

    fn seed_score(&self, seed_url: &str, active_seed: &str, now: u64) -> i64 {
        let Some(health) = self.host_health.get(seed_url) else {
            return if seed_url == active_seed { 50 } else { 25 };
        };
        let mut score = (health.success_count as i64 * 12)
            - (health.failure_count as i64 * 5)
            - (health.consecutive_failures as i64 * 14);
        if seed_url == active_seed {
            score += 20;
        }
        if self.preferred_seed_url.as_deref() == Some(seed_url) {
            score += 15;
        }
        if health.last_success_epoch > 0 && now.saturating_sub(health.last_success_epoch) <= 600 {
            score += 8;
        }
        score
    }
}

fn now_unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn subtree_standby_quarantine_secs(kind: CrawlFailureKind, consecutive_failures: u32) -> u64 {
    let base = match kind {
        CrawlFailureKind::Throttle => SUBTREE_STANDBY_QUARANTINE_BASE_SECS.saturating_mul(3),
        CrawlFailureKind::Timeout | CrawlFailureKind::Circuit => {
            SUBTREE_STANDBY_QUARANTINE_BASE_SECS.saturating_mul(2)
        }
        CrawlFailureKind::Http => SUBTREE_STANDBY_QUARANTINE_BASE_SECS,
    };
    base.saturating_mul(1_u64 << consecutive_failures.min(3))
        .min(SUBTREE_STANDBY_QUARANTINE_MAX_SECS)
}

fn split_qilin_seed_and_relative_path(url: &str) -> Option<(String, String)> {
    let parsed = reqwest::Url::parse(url).ok()?;
    let segments: Vec<_> = parsed.path_segments()?.collect();
    let uuid_idx = segments.iter().position(|segment| {
        segment.len() == 36 && segment.chars().filter(|c| *c == '-').count() == 4
    })?;

    let mut seed_path = String::new();
    for segment in segments.iter().take(uuid_idx + 1) {
        seed_path.push('/');
        seed_path.push_str(segment);
    }
    seed_path.push('/');

    let seed_url = format!("{}://{}{}", parsed.scheme(), parsed.host_str()?, seed_path);
    let relative_path = segments
        .iter()
        .skip(uuid_idx + 1)
        .filter(|segment| !segment.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join("/");
    Some((seed_url, relative_path))
}

fn seed_host(seed_url: &str) -> Option<String> {
    Some(
        reqwest::Url::parse(seed_url)
            .ok()?
            .host_str()?
            .trim()
            .to_string(),
    )
}

fn failure_kind_label(kind: CrawlFailureKind) -> &'static str {
    match kind {
        CrawlFailureKind::Timeout => "timeout",
        CrawlFailureKind::Circuit => "circuit",
        CrawlFailureKind::Throttle => "throttle",
        CrawlFailureKind::Http => "http",
    }
}

fn classify_http_status_failure(status: reqwest::StatusCode) -> CrawlFailureKind {
    if matches!(
        status,
        reqwest::StatusCode::TOO_MANY_REQUESTS
            | reqwest::StatusCode::SERVICE_UNAVAILABLE
            | reqwest::StatusCode::FORBIDDEN
            | reqwest::StatusCode::BAD_REQUEST
    ) {
        CrawlFailureKind::Throttle
    } else {
        CrawlFailureKind::Http
    }
}

fn heat_failure_kind(kind: CrawlFailureKind) -> HeatFailureKind {
    match kind {
        CrawlFailureKind::Timeout => HeatFailureKind::Timeout,
        CrawlFailureKind::Circuit => HeatFailureKind::Circuit,
        CrawlFailureKind::Throttle => HeatFailureKind::Throttle,
        CrawlFailureKind::Http => HeatFailureKind::Http,
    }
}

fn retry_lane_label(lane: RetryLane) -> &'static str {
    match lane {
        RetryLane::Primary => "primary",
        RetryLane::Degraded => "degraded",
    }
}

fn degraded_lane_limit(max_concurrent: usize) -> usize {
    env_usize("CRAWLI_QILIN_DEGRADED_LANE_MAX")
        .unwrap_or(2)
        .clamp(1, max_concurrent.max(1).min(4))
}

fn degraded_lane_interval() -> usize {
    env_usize("CRAWLI_QILIN_DEGRADED_LANE_INTERVAL")
        .unwrap_or(6)
        .max(1)
}

fn governor_rebalance_interval() -> Duration {
    Duration::from_millis(
        env_usize("CRAWLI_QILIN_GOVERNOR_INTERVAL_MS")
            .unwrap_or(2_000)
            .clamp(500, 10_000) as u64,
    )
}

fn retry_lane_for_failure(kind: CrawlFailureKind, attempt: u8) -> RetryLane {
    if matches!(kind, CrawlFailureKind::Timeout | CrawlFailureKind::Circuit) && attempt >= 1 {
        RetryLane::Degraded
    } else if matches!(kind, CrawlFailureKind::Throttle) && attempt >= 2 {
        RetryLane::Degraded
    } else {
        RetryLane::Primary
    }
}

fn retry_backoff(attempt: u8, lane: RetryLane) -> Duration {
    let capped_attempt = attempt.min(5);
    let base = 1u64 << capped_attempt;
    match lane {
        RetryLane::Primary => Duration::from_secs(base.min(8)),
        RetryLane::Degraded => Duration::from_secs((base.saturating_mul(2)).clamp(4, 20)),
    }
}

fn take_due_retry(queue: &crossbeam_queue::SegQueue<RetryPayload>) -> Option<RetryPayload> {
    let len = queue.len();
    for _ in 0..len {
        if let Some(payload) = queue.pop() {
            if std::time::Instant::now() >= payload.unlock_timestamp {
                return Some(payload);
            }
            queue.push(payload);
        }
    }
    None
}

fn try_acquire_degraded_lane(
    in_flight: &Arc<AtomicUsize>,
    limit: usize,
) -> Option<DegradedLanePermit> {
    loop {
        let current = in_flight.load(Ordering::Acquire);
        if current >= limit {
            return None;
        }
        if in_flight
            .compare_exchange(current, current + 1, Ordering::AcqRel, Ordering::Relaxed)
            .is_ok()
        {
            return Some(DegradedLanePermit {
                in_flight: Arc::clone(in_flight),
            });
        }
    }
}

fn standby_seed_urls(
    primary_url: &str,
    ranked_nodes: &[crate::adapters::qilin_nodes::StorageNode],
    limit: usize,
) -> Vec<String> {
    ranked_nodes
        .iter()
        .filter(|node| node.url != primary_url)
        .take(limit)
        .map(|node| node.url.clone())
        .collect()
}

fn remap_seed_url(failed_url: &str, current_seed: &str, next_seed: &str) -> String {
    failed_url.replacen(current_seed, next_seed, 1)
}

fn is_root_retry_url(active_seed: &str, candidate_url: &str) -> bool {
    let normalized_active = active_seed.trim_end_matches('/');
    let normalized_candidate = candidate_url.trim_end_matches('/');
    normalized_candidate == normalized_active || url_depth_relative(active_seed, candidate_url) == 0
}

fn should_keep_child_retry_on_active_seed(
    active_seed: &str,
    candidate_url: &str,
    attempt: u8,
) -> bool {
    attempt > 1
        && attempt <= 2
        && candidate_url.starts_with(active_seed)
        && !is_root_retry_url(active_seed, candidate_url)
}

fn escalate_reconciliation_attempt(previous: Option<u8>) -> u8 {
    match previous {
        Some(attempt) => attempt.saturating_add(2).min(15),
        None => 8,
    }
}

fn emit_root_listing_diagnostics(app: &AppHandle, next_url: &str, html: &str) {
    let link_cells = html.matches("<td class=\"link\">").count();
    let href_count = html.matches("href=\"").count();
    let has_qdata = html.contains("QData");
    let has_data_browser = html.contains("Data browser");
    let has_table = html.contains("<table id=\"list\">");
    let has_file_name = html.contains("File Name");
    let has_file_size = html.contains("File Size");
    let _ = app.emit(
        "log",
        format!(
            "[Qilin] Root listing diagnostics: url={} | bytes={} | qdata={} | data_browser={} | table={} | file_name={} | file_size={} | td.link={} | hrefs={}",
            next_url,
            html.len(),
            has_qdata,
            has_data_browser,
            has_table,
            has_file_name,
            has_file_size,
            link_cells,
            href_count
        ),
    );
    println!(
        "[Qilin Root Diagnostics] url={} | bytes={} | qdata={} | data_browser={} | table={} | file_name={} | file_size={} | td.link={} | hrefs={}",
        next_url,
        html.len(),
        has_qdata,
        has_data_browser,
        has_table,
        has_file_name,
        has_file_size,
        link_cells,
        href_count
    );
}

async fn confirm_qilin_root_winner(
    node_cache: Option<&QilinNodeCache>,
    uuid: Option<&str>,
    effective_url: &str,
    html: &str,
    latency_ms: u64,
    telemetry: Option<&RuntimeTelemetry>,
    app: &AppHandle,
) -> Option<StorageNode> {
    let (Some(node_cache), Some(uuid)) = (node_cache, uuid) else {
        return None;
    };
    if !QilinNodeCache::looks_like_live_qdata_listing(html) {
        return None;
    }
    if let Some(node) = node_cache
        .confirm_listing_root(uuid, effective_url, latency_ms)
        .await
    {
        if let Some(telemetry) = telemetry {
            telemetry.set_current_node_host(node.host.clone());
            telemetry.set_winner_host(node.host.clone());
        }
        let _ = app.emit(
            "log",
            format!("[Qilin] Durable storage winner confirmed: {}", node.host),
        );
        return Some(node);
    }
    None
}

fn record_late_throttle_if_durable(
    telemetry: Option<&RuntimeTelemetry>,
    durable_root_confirmed: &AtomicBool,
) {
    if durable_root_confirmed.load(Ordering::Relaxed) {
        if let Some(telemetry) = telemetry {
            telemetry.record_late_throttle();
        }
    }
}

fn shorten_tail_host(host: &str) -> String {
    let trimmed = host.trim();
    if trimmed.is_empty() {
        "-".to_string()
    } else if trimmed.len() <= 24 {
        trimmed.to_string()
    } else {
        format!("{}...{}", &trimmed[..12], &trimmed[trimmed.len() - 8..])
    }
}

const CHILD_DIAGNOSTIC_LIMIT: usize = 64;

fn emit_limited_child_log(app: &AppHandle, counter: &AtomicUsize, stage: &str, message: String) {
    let idx = counter.fetch_add(1, Ordering::Relaxed);
    if idx < CHILD_DIAGNOSTIC_LIMIT {
        let line = format!("[Qilin Child {}] {}", stage, message);
        println!("{}", line);
        let _ = app.emit("log", line);
    }
}

fn sync_qilin_frontier_progress(
    frontier: &CrawlerFrontier,
    telemetry: Option<&RuntimeTelemetry>,
    pending: &AtomicUsize,
    worker_target: usize,
) {
    frontier.set_adapter_pending_requests(pending.load(Ordering::SeqCst));
    frontier.set_adapter_worker_target(worker_target.max(1));
    if let Some(telemetry) = telemetry {
        telemetry.set_worker_metrics(
            frontier.active_workers(),
            frontier.worker_target().max(worker_target.max(1)),
        );
    }
}

fn increment_qilin_pending(
    frontier: &CrawlerFrontier,
    telemetry: Option<&RuntimeTelemetry>,
    pending: &AtomicUsize,
    worker_target: usize,
) {
    pending.fetch_add(1, Ordering::SeqCst);
    sync_qilin_frontier_progress(frontier, telemetry, pending, worker_target);
}

struct QilinPendingGuard {
    frontier: Arc<CrawlerFrontier>,
    telemetry: Option<RuntimeTelemetry>,
    pending: Arc<AtomicUsize>,
    worker_target: usize,
}

impl Drop for QilinPendingGuard {
    fn drop(&mut self) {
        let _ = self
            .pending
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
                Some(current.saturating_sub(1))
            });
        sync_qilin_frontier_progress(
            self.frontier.as_ref(),
            self.telemetry.as_ref(),
            self.pending.as_ref(),
            self.worker_target,
        );
    }
}

struct QilinActiveRequestGuard {
    frontier: Arc<CrawlerFrontier>,
    telemetry: Option<RuntimeTelemetry>,
    worker_target: usize,
}

impl QilinActiveRequestGuard {
    fn new(
        frontier: Arc<CrawlerFrontier>,
        telemetry: Option<RuntimeTelemetry>,
        worker_target: usize,
    ) -> Self {
        frontier.set_adapter_worker_target(worker_target.max(1));
        frontier.begin_adapter_request();
        if let Some(telemetry_ref) = telemetry.as_ref() {
            telemetry_ref.set_worker_metrics(
                frontier.active_workers(),
                frontier.worker_target().max(worker_target.max(1)),
            );
        }
        Self {
            frontier,
            telemetry,
            worker_target: worker_target.max(1),
        }
    }
}

impl Drop for QilinActiveRequestGuard {
    fn drop(&mut self) {
        self.frontier.finish_adapter_request();
        if let Some(telemetry_ref) = self.telemetry.as_ref() {
            telemetry_ref.set_worker_metrics(
                self.frontier.active_workers(),
                self.frontier.worker_target().max(self.worker_target),
            );
        }
    }
}

fn listing_entry_name(href: &str) -> String {
    let trimmed = href.trim();
    let without_query = trimmed.split(['?', '#']).next().unwrap_or(trimmed);
    let decoded = path_utils::url_decode(without_query);
    let normalized = decoded.trim_end_matches('/');
    normalized
        .rsplit('/')
        .find(|segment| !segment.is_empty())
        .unwrap_or(normalized)
        .to_string()
}

fn resolve_listing_child_url(base_url: &str, href: &str, is_dir: bool) -> String {
    let mut resolved = reqwest::Url::parse(base_url)
        .ok()
        .and_then(|base| base.join(href).ok())
        .map(|url| url.to_string())
        .or_else(|| reqwest::Url::parse(href).ok().map(|url| url.to_string()))
        .unwrap_or_else(|| {
            format!(
                "{}/{}",
                base_url.trim_end_matches('/'),
                path_utils::url_encode(href.trim_start_matches('/'))
            )
        });

    if is_dir && !resolved.ends_with('/') {
        resolved.push('/');
    }

    resolved
}

pub(crate) fn extract_watch_data_targets(base_domain: &str, html: &str) -> Vec<String> {
    static WATCH_DATA_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(
            r#"(?is)<a[^>]+href="([^"]*(?:/site/data\?uuid=[^"]+|http://[a-z2-7]{56}\.onion/[^"]+))"[^>]*>\s*watch data\s*</a>"#,
        )
        .unwrap()
    });

    let mut seen = std::collections::HashSet::new();
    WATCH_DATA_RE
        .captures_iter(html)
        .filter_map(|cap| cap.get(1).map(|m| m.as_str().trim().to_string()))
        .filter_map(|raw| {
            reqwest::Url::parse(&raw)
                .ok()
                .map(|url| url.to_string())
                .or_else(|| {
                    reqwest::Url::parse(base_domain)
                        .ok()
                        .and_then(|base| base.join(&raw).ok())
                        .map(|url| url.to_string())
                })
        })
        .filter(|resolved| seen.insert(resolved.clone()))
        .collect()
}

/// Phase 76C: Traffic class for circuit partitioning.
/// Crawl (listing) and download traffic use separate circuit ranges
/// to prevent download saturation from starving page workers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
enum TrafficClass {
    Listing,
    Download,
}

/// Phase 76C: Compute URL depth relative to a seed URL.
/// Returns the number of path segments beyond the seed.
/// e.g. seed = "http://x.onion/root/", url = "http://x.onion/root/A/B/C/" → depth 3
fn url_depth_relative(seed_url: &str, target_url: &str) -> usize {
    if !target_url.starts_with(seed_url) {
        // Fall back to absolute path segment count
        return target_url
            .split('/')
            .filter(|s| !s.is_empty() && !s.contains(':'))
            .count();
    }
    let relative = &target_url[seed_url.len()..];
    relative.split('/').filter(|s| !s.is_empty()).count()
}

struct QilinCrawlGovernor {
    desired_active: AtomicUsize,
    in_flight: AtomicUsize,
    min_active: usize,
    max_active: usize,
    available_clients: usize,
    permit_budget: usize,
    reserve_for_downloads: bool,
    successes: AtomicUsize,
    failures: AtomicUsize,
    timeouts: AtomicUsize,
    circuit_failures: AtomicUsize,
    pub throttles: Arc<AtomicUsize>,
    http_failures: AtomicUsize,
    // Phase 67D: epoch millis of most recent throttle for reactive scale-down
    pub last_throttle_epoch_ms: Arc<AtomicU64>,
    pub throttles_in_window: Arc<AtomicUsize>,
    pub adaptive_ramp_ceiling: Arc<AtomicUsize>,
    // Phase 67F: Per-circuit latency profiling (supports up to 16 circuits)
    circuit_latency_sum_ms: [AtomicU64; 16],
    circuit_request_count: [AtomicU64; 16],
    // Phase 67L: Per-circuit error count for health scoring
    circuit_error_count: [AtomicUsize; 16],
    // Phase 89: Cooldown guard for governor-driven latency outlier isolation
    last_outlier_isolation_epoch_ms: [AtomicU64; 16],
    repin_interval_hint: AtomicUsize,
    // Phase 74B: EWMA error-rate decay tracking
    last_ewma_decay_epoch_ms: AtomicU64,
    // Phase 74B: Ceiling change tracking for frontend logging
    last_ceiling_change_epoch_ms: AtomicU64,
    prev_ceiling_value: AtomicUsize,
    telemetry: Option<RuntimeTelemetry>,
    // Phase 76C: Circuit partitioning for crawl/download traffic separation
    listing_circuit_start: usize,
    listing_circuit_end: usize, // exclusive
    download_circuit_start: usize,
    download_circuit_end: usize, // exclusive
    listing_workers_active: AtomicUsize,
    download_workers_active: AtomicUsize,
}

impl QilinCrawlGovernor {
    fn new(
        available_clients: usize,
        reserve_for_downloads: bool,
        telemetry: Option<RuntimeTelemetry>,
    ) -> Self {
        let available_clients = available_clients.max(1);
        let min_active = env_usize("CRAWLI_QILIN_PAGE_WORKERS_MIN")
            .unwrap_or(4)
            .max(1);
        let multiplex_factor = env_usize("CRAWLI_QILIN_CLIENT_MULTIPLEX_FACTOR")
            .unwrap_or(1)
            .clamp(1, 4);
        let effective_budget = available_clients
            .saturating_mul(multiplex_factor)
            .max(min_active);
        let profile_budget = crate::resource_governor::recommend_listing_budget(
            available_clients,
            effective_budget,
            true,
            reserve_for_downloads,
            telemetry.as_ref(),
        );
        let default_max = if reserve_for_downloads { 10 } else { 16 };
        let explicit_qilin_workers = env_usize("CRAWLI_QILIN_WORKERS");

        let mut max_active = env_usize(if reserve_for_downloads {
            "CRAWLI_QILIN_PAGE_WORKERS_DOWNLOAD_MAX"
        } else {
            "CRAWLI_QILIN_PAGE_WORKERS_MAX"
        })
        .unwrap_or(default_max)
        .clamp(min_active, effective_budget);

        if let Some(explicit) = explicit_qilin_workers {
            max_active = explicit.max(min_active).min(128);
        } else {
            max_active = max_active.min(profile_budget.worker_cap.max(min_active));
        }

        let mut desired_active = env_usize(if reserve_for_downloads {
            "CRAWLI_QILIN_PAGE_WORKERS_DOWNLOAD_START"
        } else {
            "CRAWLI_QILIN_PAGE_WORKERS_START"
        })
        .unwrap_or(if reserve_for_downloads { 4 } else { 6 })
        .clamp(min_active, max_active);

        if let Some(explicit) = explicit_qilin_workers {
            desired_active = (explicit / 2).clamp(min_active, max_active);
        } else {
            desired_active =
                desired_active.min(profile_budget.worker_cap.clamp(min_active, max_active));
        }

        Self {
            desired_active: AtomicUsize::new(desired_active),
            in_flight: AtomicUsize::new(0),
            min_active,
            max_active,
            available_clients,
            permit_budget: effective_budget,
            reserve_for_downloads,
            successes: AtomicUsize::new(0),
            failures: AtomicUsize::new(0),
            timeouts: AtomicUsize::new(0),
            circuit_failures: AtomicUsize::new(0),
            throttles: Arc::new(AtomicUsize::new(0)),
            http_failures: AtomicUsize::new(0),
            last_throttle_epoch_ms: Arc::new(AtomicU64::new(0)),
            throttles_in_window: Arc::new(AtomicUsize::new(0)),
            adaptive_ramp_ceiling: Arc::new(AtomicUsize::new(max_active)),
            circuit_latency_sum_ms: std::array::from_fn(|_| AtomicU64::new(0)),
            circuit_request_count: std::array::from_fn(|_| AtomicU64::new(0)),
            circuit_error_count: std::array::from_fn(|_| AtomicUsize::new(0)),
            last_outlier_isolation_epoch_ms: std::array::from_fn(|_| AtomicU64::new(0)),
            repin_interval_hint: AtomicUsize::new(20),
            last_ewma_decay_epoch_ms: AtomicU64::new(0),
            last_ceiling_change_epoch_ms: AtomicU64::new(0),
            prev_ceiling_value: AtomicUsize::new(max_active),
            telemetry,
            // Phase 76C: Circuit partitioning — reserve 2/3 for listing, 1/3 for downloads
            // When not reserve_for_downloads, all circuits serve listing.
            listing_circuit_start: 0,
            listing_circuit_end: if reserve_for_downloads {
                (available_clients * 2 / 3).max(1)
            } else {
                available_clients
            },
            download_circuit_start: if reserve_for_downloads {
                (available_clients * 2 / 3).max(1)
            } else {
                0
            },
            download_circuit_end: available_clients,
            listing_workers_active: AtomicUsize::new(0),
            download_workers_active: AtomicUsize::new(0),
        }
    }

    fn current_target(&self) -> usize {
        self.desired_active.load(Ordering::Relaxed)
    }

    /// Phase 76C: Select a circuit from the appropriate partition based on traffic class.
    /// Uses the circuit scorer within the constrained range for optimal selection.
    fn circuit_for_class(
        &self,
        class: TrafficClass,
        scorer: &crate::scorer::CircuitScorer,
    ) -> usize {
        let (start, end) = match class {
            TrafficClass::Listing => (self.listing_circuit_start, self.listing_circuit_end),
            TrafficClass::Download => (self.download_circuit_start, self.download_circuit_end),
        };
        if start >= end {
            // Fallback: return any circuit via scorer
            return scorer.best_circuit_for_url(self.available_clients);
        }
        // Score only within the partition range
        let range_size = end - start;
        let best_in_range = scorer.best_circuit_for_url(range_size);
        start + best_in_range
    }

    /// Phase 76C: Summary string for traffic separation visibility in governor logs.
    fn traffic_summary(&self) -> String {
        if self.reserve_for_downloads {
            format!(
                "listing_cids=[{}-{}] download_cids=[{}-{}] listing_w={} download_w={}",
                self.listing_circuit_start,
                self.listing_circuit_end.saturating_sub(1),
                self.download_circuit_start,
                self.download_circuit_end.saturating_sub(1),
                self.listing_workers_active.load(Ordering::Relaxed),
                self.download_workers_active.load(Ordering::Relaxed),
            )
        } else {
            "traffic=unified (no partition)".to_string()
        }
    }

    /// Phase 67F: Record success with per-circuit latency tracking
    fn record_success_with_latency(&self, cid: usize, elapsed_ms: u64) {
        self.successes.fetch_add(1, Ordering::Relaxed);
        if cid < 16 {
            self.circuit_latency_sum_ms[cid].fetch_add(elapsed_ms, Ordering::Relaxed);
            self.circuit_request_count[cid].fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Phase 67F: Format per-circuit avg latency for logging
    fn circuit_latency_summary(&self) -> String {
        let mut parts = Vec::new();
        for i in 0..16 {
            let count = self.circuit_request_count[i].load(Ordering::Relaxed);
            if count > 0 {
                let sum = self.circuit_latency_sum_ms[i].load(Ordering::Relaxed);
                let avg = sum / count;
                parts.push(format!("c{}:{}ms", i, avg));
            }
        }
        if parts.is_empty() {
            "none".to_string()
        } else {
            parts.join(" ")
        }
    }

    pub fn record_throttle(&self) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or(std::time::Duration::ZERO)
            .as_millis() as u64;

        let last = self.last_throttle_epoch_ms.swap(now, Ordering::Relaxed);

        if now.saturating_sub(last) > 10000 {
            // Reset sliding window if it's been > 10s since the last throttle
            self.throttles_in_window.store(1, Ordering::Relaxed);
        } else {
            self.throttles_in_window.fetch_add(1, Ordering::Relaxed);
        }

        self.throttles.fetch_add(1, Ordering::Relaxed);
    }
    pub fn should_halt_ramp(&self, current_active_workers: usize) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or(std::time::Duration::ZERO)
            .as_millis() as u64;

        let last_throttle = self.last_throttle_epoch_ms.load(Ordering::Relaxed);
        let window_throttles = self.throttles_in_window.load(Ordering::Relaxed);
        let current_ceiling = self.adaptive_ramp_ceiling.load(Ordering::Relaxed);

        if current_active_workers > current_ceiling {
            return true;
        }

        // Halt ramp and set adaptive ceiling if we've seen > 2 throttles in the last 10 seconds
        if window_throttles > 2 && (now.saturating_sub(last_throttle) < 10000) {
            // Freeze ceiling at the current active worker count (or slightly less to relieve pressure)
            let safe_ceiling = current_active_workers.saturating_sub(1).max(1);

            // Only update if we're lowering the ceiling further (don't accidentally raise it)
            if safe_ceiling < current_ceiling {
                self.adaptive_ramp_ceiling
                    .store(safe_ceiling, Ordering::Relaxed);
            }
            return true;
        }

        false
    }

    /// Phase 67I: Returns (best_cid, best_avg_ms) for the fastest circuit
    fn best_latency_circuit(&self) -> Option<(usize, u64)> {
        let mut best: Option<(usize, u64)> = None;
        for i in 0..16 {
            let count = self.circuit_request_count[i].load(Ordering::Relaxed);
            if count >= 5 {
                let sum = self.circuit_latency_sum_ms[i].load(Ordering::Relaxed);
                let avg = sum / count;
                if best.is_none() || avg < best.unwrap().1 {
                    best = Some((i, avg));
                }
            }
        }
        best
    }

    fn slowest_latency_circuit(&self) -> Option<(usize, u64)> {
        let mut slowest: Option<(usize, u64)> = None;
        for i in 0..16 {
            let count = self.circuit_request_count[i].load(Ordering::Relaxed);
            if count >= 5 {
                let sum = self.circuit_latency_sum_ms[i].load(Ordering::Relaxed);
                let avg = sum / count;
                if slowest.is_none() || avg > slowest.unwrap().1 {
                    slowest = Some((i, avg));
                }
            }
        }
        slowest
    }

    fn slowest_latency_summary(&self) -> Option<String> {
        let (cid, avg_ms) = self.slowest_latency_circuit()?;
        Some(format!("c{cid}:{avg_ms}ms"))
    }

    fn median_latency_ms(&self) -> Option<u64> {
        let mut avgs = Vec::new();
        for i in 0..16 {
            let count = self.circuit_request_count[i].load(Ordering::Relaxed);
            if count >= 5 {
                let sum = self.circuit_latency_sum_ms[i].load(Ordering::Relaxed);
                avgs.push(sum / count);
            }
        }
        if avgs.is_empty() {
            return None;
        }
        avgs.sort_unstable();
        Some(avgs[avgs.len() / 2])
    }

    fn latency_outlier_candidate(&self) -> Option<(usize, u64, u64)> {
        const MIN_SAMPLES: u64 = 8;
        const MIN_OUTLIER_MS: u64 = 3_500;

        let (_, best_avg) = self.best_latency_circuit()?;
        let baseline = self.median_latency_ms().unwrap_or(best_avg).max(best_avg);
        let mut worst: Option<(usize, u64, u64)> = None;

        for cid in 0..16 {
            let count = self.circuit_request_count[cid].load(Ordering::Relaxed);
            if count < MIN_SAMPLES {
                continue;
            }
            let sum = self.circuit_latency_sum_ms[cid].load(Ordering::Relaxed);
            let avg = sum / count;
            if avg < MIN_OUTLIER_MS || avg <= baseline.saturating_mul(3) {
                continue;
            }

            if worst.is_none() || avg > worst.unwrap().1 {
                worst = Some((cid, avg, baseline));
            }
        }

        worst
    }

    fn mark_latency_outlier_isolation(&self, cid: usize) -> bool {
        const OUTLIER_ISOLATION_COOLDOWN_MS: u64 = 30_000;
        if cid >= 16 {
            return false;
        }

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let slot = &self.last_outlier_isolation_epoch_ms[cid];
        let last = slot.load(Ordering::Relaxed);
        if now_ms.saturating_sub(last) < OUTLIER_ISOLATION_COOLDOWN_MS {
            return false;
        }

        slot.compare_exchange(last, now_ms, Ordering::AcqRel, Ordering::Relaxed)
            .is_ok()
    }

    fn set_repin_interval_hint(&self, interval: usize) {
        self.repin_interval_hint
            .store(interval.clamp(6, 32), Ordering::Relaxed);
    }

    fn current_repin_interval(&self) -> usize {
        let hinted = self
            .repin_interval_hint
            .load(Ordering::Relaxed)
            .clamp(6, 32);
        let window_throttles = self.throttles_in_window.load(Ordering::Relaxed);
        if window_throttles >= 3 {
            hinted.min(8)
        } else if window_throttles > 0 {
            hinted.min(12)
        } else {
            hinted
        }
    }

    /// Phase 67I+L: Returns true if the worker should re-pin to a faster/healthier circuit
    fn should_repin(&self, current_cid: usize) -> bool {
        if current_cid >= 16 {
            return false;
        }
        let my_count = self.circuit_request_count[current_cid].load(Ordering::Relaxed);
        if my_count < 10 {
            return false;
        } // Need enough samples

        // Phase 67L: Check error rate — re-pin if >30% error rate
        let my_errors = self.circuit_error_count[current_cid].load(Ordering::Relaxed) as u64;
        let my_total = my_count + my_errors;
        if my_total >= 10 && my_errors * 100 / my_total.max(1) > 30 {
            return true;
        }

        // Phase 67I: Check latency — re-pin if >1.8× slower than best
        let my_sum = self.circuit_latency_sum_ms[current_cid].load(Ordering::Relaxed);
        let my_avg = my_sum / my_count;
        if let Some((_, best_avg)) = self.best_latency_circuit() {
            my_avg > best_avg.saturating_mul(18) / 10
        } else {
            false
        }
    }

    /// Phase 74: Thompson Sampling circuit selection for smart work distribution.
    /// Uses Box-Muller transform over per-circuit latency statistics to balance
    /// exploitation (fast circuits) with exploration (untested circuits).
    /// Returns (best_cid, thompson_score) — higher score = preferred circuit.
    fn thompson_sample_circuit(&self) -> Option<(usize, f64)> {
        let mut best: Option<(usize, f64)> = None;
        let now_nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();

        for i in 0..16 {
            let count = self.circuit_request_count[i].load(Ordering::Relaxed);
            if count == 0 {
                // Untested circuits get infinite score (explore first)
                return Some((i, f64::MAX));
            }

            let sum = self.circuit_latency_sum_ms[i].load(Ordering::Relaxed);
            let errors = self.circuit_error_count[i].load(Ordering::Relaxed) as f64;
            let avg_speed = count as f64 / (sum.max(1) as f64); // requests per ms (higher=faster)

            // Penalize high-error circuits
            let error_penalty = if count > 5 {
                1.0 - (errors / (count as f64 + errors)).min(0.8)
            } else {
                1.0
            };

            // Box-Muller transform for Thompson Sampling: N(mean, variance)
            let variance = if count >= 5 {
                let mean_ms = sum as f64 / count as f64;
                // Estimate variance from mean (heuristic for streaming data)
                (mean_ms * 0.3).max(10.0)
            } else {
                500.0 // High variance = explore
            };

            let std_dev = variance.sqrt();
            let u1 = (((now_nanos ^ (now_nanos >> 12) ^ (i as u128 * 7919)) % 10000) as f64
                / 10000.0)
                .max(0.0001);
            let u2 = (((now_nanos ^ (now_nanos >> 20) ^ (i as u128 * 6271)) % 10000) as f64
                / 10000.0)
                .max(0.0001);
            let z0 = (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos();

            let score = (avg_speed + z0 * std_dev * 0.001) * error_penalty;

            if best.is_none() || score > best.unwrap().1 {
                best = Some((i, score));
            }
        }
        best
    }

    /// Phase 74B: EWMA error-rate decay — reduces per-circuit error counts by 25%
    /// every 30s so previously-bad circuits get second chances.
    fn apply_ewma_error_decay(&self) {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let last_decay = self.last_ewma_decay_epoch_ms.load(Ordering::Relaxed);

        // Decay every 30 seconds
        if now_ms.saturating_sub(last_decay) < 30_000 {
            return;
        }

        // CAS to prevent multiple concurrent decays
        if self
            .last_ewma_decay_epoch_ms
            .compare_exchange(last_decay, now_ms, Ordering::AcqRel, Ordering::Relaxed)
            .is_err()
        {
            return;
        }

        // Apply 0.75 decay factor to all circuit error counts
        for i in 0..16 {
            let current = self.circuit_error_count[i].load(Ordering::Relaxed);
            if current > 0 {
                let decayed = (current * 3) / 4; // 75% retention = 25% decay
                self.circuit_error_count[i].store(decayed, Ordering::Relaxed);
            }
        }
    }

    /// Phase 74B: Adaptive circuit ceiling based on throttle rate.
    /// Ramps down from max→half when 503 count exceeds threshold,
    /// recovers aggressively (+4) after 60s cooldown.
    /// Returns (new_ceiling, changed:bool) for frontend event logging.
    fn adaptive_ceiling_update(&self) -> usize {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let last_throttle = self.last_throttle_epoch_ms.load(Ordering::Relaxed);
        let window_throttles = self.throttles_in_window.load(Ordering::Relaxed);
        let current_ceiling = self.adaptive_ramp_ceiling.load(Ordering::Relaxed);

        // Heavy throttle burst: >3 in 10s window → halve the ceiling
        if window_throttles > 3 && now_ms.saturating_sub(last_throttle) < 10_000 {
            let halved = (current_ceiling / 2).max(self.min_active);
            if halved < current_ceiling {
                self.adaptive_ramp_ceiling.store(halved, Ordering::Relaxed);
                self.last_ceiling_change_epoch_ms
                    .store(now_ms, Ordering::Relaxed);
                self.prev_ceiling_value
                    .store(current_ceiling, Ordering::Relaxed);
            }
            return halved;
        }

        // Phase 74B: Aggressive recovery — +4 per 60s since last throttle
        if last_throttle > 0 && now_ms.saturating_sub(last_throttle) > 60_000 {
            let recovered = (current_ceiling + 4).min(self.max_active);
            if recovered > current_ceiling {
                self.adaptive_ramp_ceiling
                    .store(recovered, Ordering::Relaxed);
                self.last_ceiling_change_epoch_ms
                    .store(now_ms, Ordering::Relaxed);
                self.prev_ceiling_value
                    .store(current_ceiling, Ordering::Relaxed);
            }
            return recovered;
        }

        current_ceiling
    }

    /// Phase 74B: Check if ceiling changed since last query and return change info
    fn ceiling_change_info(&self) -> Option<(usize, usize)> {
        let prev = self.prev_ceiling_value.load(Ordering::Relaxed);
        let current = self.adaptive_ramp_ceiling.load(Ordering::Relaxed);
        if prev != current {
            // Acknowledge the change
            self.prev_ceiling_value.store(current, Ordering::Relaxed);
            Some((prev, current))
        } else {
            None
        }
    }

    /// Phase 67K: Compute adaptive timeout from observed latency data
    /// Returns timeout in seconds: max(8, median_avg_latency × 4), clamped [8, 45]
    fn adaptive_timeout_secs(&self) -> u64 {
        let mut avgs: Vec<u64> = Vec::new();
        for i in 0..16 {
            let count = self.circuit_request_count[i].load(Ordering::Relaxed);
            if count >= 3 {
                let sum = self.circuit_latency_sum_ms[i].load(Ordering::Relaxed);
                avgs.push(sum / count);
            }
        }
        if avgs.is_empty() {
            return 25; // No data yet — use default
        }
        avgs.sort();
        let median_ms = avgs[avgs.len() / 2];
        // Timeout = 4× median latency, clamped to [8, 45] seconds
        (median_ms.saturating_mul(4) / 1000).clamp(8, 45)
    }

    fn record_failure(&self, kind: CrawlFailureKind) {
        self.failures.fetch_add(1, Ordering::Relaxed);
        match kind {
            CrawlFailureKind::Timeout => {
                self.timeouts.fetch_add(1, Ordering::Relaxed);
                if let Some(telemetry) = &self.telemetry {
                    telemetry.record_timeout();
                }
            }
            CrawlFailureKind::Circuit => {
                self.circuit_failures.fetch_add(1, Ordering::Relaxed);
            }
            CrawlFailureKind::Throttle => {
                self.record_throttle();
                if let Some(telemetry) = &self.telemetry {
                    telemetry.record_throttle();
                }
            }
            CrawlFailureKind::Http => {
                self.http_failures.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Phase 67L: Record a failure with circuit tracking for health scoring
    fn record_failure_for_circuit(&self, kind: CrawlFailureKind, cid: usize) {
        self.failures.fetch_add(1, Ordering::Relaxed);
        if cid < 16 {
            self.circuit_error_count[cid].fetch_add(1, Ordering::Relaxed);
        }
        match kind {
            CrawlFailureKind::Timeout => {
                self.timeouts.fetch_add(1, Ordering::Relaxed);
                if let Some(telemetry) = &self.telemetry {
                    telemetry.record_timeout();
                }
            }
            CrawlFailureKind::Throttle => {
                self.record_throttle();
                if let Some(telemetry) = &self.telemetry {
                    telemetry.record_throttle();
                }
            }
            CrawlFailureKind::Circuit => {
                self.circuit_failures.fetch_add(1, Ordering::Relaxed);
            }
            CrawlFailureKind::Http => {
                self.http_failures.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    async fn acquire_slot(self: &Arc<Self>) -> QilinCrawlPermit {
        loop {
            let desired = self.desired_active.load(Ordering::Relaxed).max(1);
            // Phase 67D: Reactive throttle — if a throttle happened within 5s,
            // cut effective desired in half to prevent hammering
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            let last_throttle = self.last_throttle_epoch_ms.load(Ordering::Relaxed);
            let throttle_active = last_throttle > 0 && now_ms.saturating_sub(last_throttle) < 5_000;
            let effective_desired = if throttle_active {
                (desired / 2).max(self.min_active)
            } else {
                desired
            };
            let current = self.in_flight.load(Ordering::Relaxed);
            if current < effective_desired
                && self
                    .in_flight
                    .compare_exchange(current, current + 1, Ordering::AcqRel, Ordering::Relaxed)
                    .is_ok()
            {
                return QilinCrawlPermit {
                    governor: Arc::clone(self),
                };
            }

            let wait_ms = if current >= desired { 50 } else { 25 };
            tokio::time::sleep(Duration::from_millis(wait_ms)).await;
        }
    }

    fn rebalance(&self, pending: usize) -> Option<(usize, f64)> {
        let successes = self.successes.swap(0, Ordering::Relaxed);
        let failures = self.failures.swap(0, Ordering::Relaxed);
        let timeouts = self.timeouts.swap(0, Ordering::Relaxed);
        let circuit_failures = self.circuit_failures.swap(0, Ordering::Relaxed);
        let throttles = self.throttles.swap(0, Ordering::Relaxed);
        let http_failures = self.http_failures.swap(0, Ordering::Relaxed);

        let current = self.desired_active.load(Ordering::Relaxed).max(1);
        let total = successes + failures;
        let success_ratio = if total > 0 {
            successes as f64 / total as f64
        } else {
            1.0
        };
        let pressure_budget = crate::resource_governor::recommend_listing_budget(
            self.available_clients,
            self.permit_budget,
            true,
            self.reserve_for_downloads,
            self.telemetry.as_ref(),
        );
        let pressure_cap = pressure_budget
            .worker_cap
            .clamp(self.min_active, self.max_active);
        let pressure = pressure_budget.pressure.total_pressure;

        // Phase 74: Apply adaptive ceiling decay/recovery
        let adaptive_cap = self.adaptive_ceiling_update();
        let effective_cap = pressure_cap.min(adaptive_cap);

        let mut next = current.min(effective_cap).max(self.min_active);
        if pressure >= 0.85 {
            next = ((current * 2) / 3).max(self.min_active).min(effective_cap);
        } else if pressure >= 0.70 {
            next = ((current * 4) / 5).max(self.min_active).min(effective_cap);
        } else if throttles > 0 || circuit_failures >= 2 {
            next = ((current * 2) / 3).max(self.min_active);
        } else if timeouts >= 3 && timeouts >= successes.max(1) {
            next = ((current * 3) / 4).max(self.min_active);
        } else if http_failures >= 4 && http_failures > successes {
            next = ((current * 4) / 5).max(self.min_active);
        } else if pending > current * 3 && success_ratio > 0.90 && total >= current.min(6) {
            // Phase 67D: Graduated re-escalation — limit scale-up after recent throttles
            let throttle_cooldown_active = {
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;
                let last = self.last_throttle_epoch_ms.load(Ordering::Relaxed);
                last > 0 && now_ms.saturating_sub(last) < 30_000
            };
            let step = if throttle_cooldown_active { 1 } else { 4 };
            next = (current + step).min(pressure_cap);
        } else if pending > current && success_ratio > 0.75 && total >= 4 {
            let throttle_cooldown_active = {
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;
                let last = self.last_throttle_epoch_ms.load(Ordering::Relaxed);
                last > 0 && now_ms.saturating_sub(last) < 30_000
            };
            let step = if throttle_cooldown_active { 1 } else { 2 };
            next = (current + step).min(pressure_cap);
        } else if total >= 6 && success_ratio < 0.50 {
            next = ((current * 3) / 4).max(self.min_active);
        }
        next = next.clamp(self.min_active, pressure_cap);

        if next != current {
            self.desired_active.store(next, Ordering::Relaxed);
            Some((next, pressure))
        } else {
            None
        }
    }
}

struct QilinCrawlPermit {
    governor: Arc<QilinCrawlGovernor>,
}

impl Drop for QilinCrawlPermit {
    fn drop(&mut self) {
        let _ = self
            .governor
            .in_flight
            .fetch_sub(1, Ordering::Release)
            .saturating_sub(1);
    }
}

fn classify_request_error(err: &anyhow::Error) -> CrawlFailureKind {
    let err_str = err.to_string().to_lowercase();
    if err_str.contains("failed to obtain hidden service circuit")
        || err_str.contains("hidden service circuit")
        || err_str.contains("un-retried transient failure")
        || err_str.contains("connection reset")
        || err_str.contains("broken pipe")
        || err_str.contains("eos")
        || err_str.contains("eof")
    {
        CrawlFailureKind::Circuit
    } else if err_str.contains("timed out") || err_str.contains("timeout") {
        CrawlFailureKind::Timeout
    } else {
        CrawlFailureKind::Http
    }
}

#[async_trait::async_trait]
impl CrawlerAdapter for QilinAdapter {
    async fn can_handle(&self, fingerprint: &SiteFingerprint) -> bool {
        // URL-based matching: Qilin CMS URL patterns are unique identifiers
        // (reqwest follows 302 redirects, so body won't contain CMS markers
        //  when the URL is /site/view or /site/data — the body will be nginx autoindex)
        if fingerprint.url.contains("/site/view") || fingerprint.url.contains("/site/data") {
            return true;
        }
        // Body-based matching: CMS page markers (when redirect doesn't happen)
        fingerprint
            .body
            .contains("<div class=\"page-header-title\">QData</div>")
            || fingerprint.body.contains("Data browser")
            || fingerprint.body.contains("_csrf-blog")
            || fingerprint.body.contains("item_box_photos")
            // Nginx autoindex markers (storage node pages after redirect)
            || (fingerprint.body.contains("<table id=\"list\">")
                && fingerprint.body.contains("<td class=\"link\">"))
            || regex::Regex::new(r#"value="[a-z2-7]{56}\.onion""#)
                .unwrap()
                .is_match(&fingerprint.body)
    }

    async fn crawl(
        &self,
        current_url: &str,
        frontier: Arc<CrawlerFrontier>,
        app: AppHandle,
    ) -> anyhow::Result<Vec<FileEntry>> {
        use tauri::Emitter;

        let state = app.state::<crate::AppState>();
        let telemetry = Some(state.telemetry.clone());
        let vfs = Some(state.vfs.clone());
        let collect_results_locally = false;
        let is_dumb_mode = is_dumb_mode_enabled();
        // Phase 76 PR-HEATMAP-DEFAULT-001: Subtree shaping enabled by default
        let subtree_shaping_enabled =
            !is_dumb_mode && env_bool_default_true("CRAWLI_QILIN_SUBTREE_SHAPING");
        let subtree_heatmap_enabled =
            subtree_shaping_enabled && env_bool_default_true("CRAWLI_QILIN_SUBTREE_HEATMAP");
        let current_target_dir = state.current_target_dir.lock().await.clone();
        let current_target_key = state.current_target_key.lock().await.clone();
        let heatmap_path = current_target_dir
            .as_ref()
            .map(|dir| dir.join("qilin_bad_subtrees.json"));
        let heatmap = Arc::new(tokio::sync::Mutex::new(
            match (
                subtree_heatmap_enabled,
                &heatmap_path,
                current_target_key.as_deref(),
            ) {
                (true, Some(path), Some(target_key)) => SubtreeHeatmap::load(path, target_key)
                    .unwrap_or_else(|_| SubtreeHeatmap {
                        target_key: target_key.to_string(),
                        ..Default::default()
                    }),
                _ => SubtreeHeatmap::default(),
            },
        ));
        if subtree_heatmap_enabled {
            // Phase 75: Merge Sled VFS heatmap records into the in-memory heatmap
            if let Some(vfs_ref) = &vfs {
                if let Ok(sled_records) = vfs_ref.load_heatmap_records().await {
                    if !sled_records.is_empty() {
                        let mut hm = heatmap.lock().await;
                        let mut merged = 0usize;
                        for (key, sled_record) in &sled_records {
                            let should_insert = match hm.entries.get(key) {
                                Some(existing) => {
                                    // Use Sled record only if it's more recent
                                    let sled_last = sled_record
                                        .last_failure_epoch
                                        .max(sled_record.last_success_epoch);
                                    let json_last = existing
                                        .last_failure_epoch
                                        .max(existing.last_success_epoch);
                                    sled_last > json_last
                                }
                                None => true,
                            };
                            if should_insert {
                                hm.entries.insert(key.clone(), sled_record.clone());
                                merged += 1;
                            }
                        }
                        if merged > 0 {
                            println!(
                                "[Qilin] Merged {} heatmap records from Sled VFS ({} total in Sled)",
                                merged,
                                sled_records.len()
                            );
                        }
                    }
                }
            }
            if let Some(path) = &heatmap_path {
                let loaded_entries = heatmap.lock().await.entries.len();
                if loaded_entries > 0 {
                    let _ = app.emit(
                        "log",
                        format!(
                            "[Qilin] Loaded subtree heatmap: {} clustered prefixes from {} + Sled VFS",
                            loaded_entries,
                            path.display()
                        ),
                    );
                }
            }
        }
        let _ = app.emit(
            "log",
            format!(
                "[Qilin] Subtree shaping policy: live={} persistent={}",
                subtree_shaping_enabled, subtree_heatmap_enabled
            ),
        );
        if is_dumb_mode {
            let _ = app.emit(
                "log",
                "[Qilin] CRAWLI_DUMB_MODE=1 active: intelligence pipeline disabled (naive fast-path benchmark mode)."
                    .to_string(),
            );
        }

        let queue = Arc::new(crossbeam_queue::SegQueue::new());
        let retry_queue = Arc::new(crossbeam_queue::SegQueue::<RetryPayload>::new());
        let degraded_retry_queue = Arc::new(crossbeam_queue::SegQueue::<RetryPayload>::new());
        let collected_entries = Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let root_fetch_logged = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let root_parse_logged = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let child_queue_logged = Arc::new(AtomicUsize::new(0));
        let child_fetch_logged = Arc::new(AtomicUsize::new(0));
        let child_parse_logged = Arc::new(AtomicUsize::new(0));
        let child_failure_logged = Arc::new(AtomicUsize::new(0));
        let child_retry_lane_logged = Arc::new(AtomicUsize::new(0));

        // Phase 44: Absolute Directory Verification
        let discovered_folders =
            Arc::new(tokio::sync::Mutex::new(std::collections::HashSet::new()));
        let visited_folders = Arc::new(tokio::sync::Mutex::new(std::collections::HashSet::new()));

        let mut actual_seed_url = current_url.to_string();
        let mut standby_routes = Vec::new();
        let mut goto_unleash = false;
        let mut json_bypass_entries = Vec::new();
        let mut initial_repin_interval = 20usize;

        // Phase 30: Multi-Node Storage Discovery with Persistent Cache
        let (extracted_uuid, ingress_kind) = classify_qilin_ingress(current_url);
        let cms_launcher_url = matches!(ingress_kind, QilinIngressKind::CmsLauncher);
        let direct_listing_url = matches!(ingress_kind, QilinIngressKind::DirectListing);
        let node_cache = if extracted_uuid.is_some() {
            let cache = QilinNodeCache::default();
            if let Err(e) = cache.initialize().await {
                eprintln!("[Qilin] Failed to init node cache: {}", e);
            }
            Some(cache)
        } else {
            None
        };

        // Phase 77: Military-grade CMS Data Bypass
        // Strategy 1: Try JSON from /site/data (may redirect to dead storage node)
        // Strategy 2: Try HTML scrape from /site/view (confirmed reachable via Stage B)
        // Both bypass the need for storage node discovery.
        if !is_dumb_mode && cms_launcher_url && legacy_cms_bypass_enabled() {
            if let Some(ref uuid) = extracted_uuid {
                let base_domain = if let Ok(parsed) = reqwest::Url::parse(current_url) {
                    format!("{}://{}", parsed.scheme(), parsed.host_str().unwrap_or(""))
                } else {
                    current_url.split("/site/").next().unwrap_or("").to_string()
                };
                let data_json_url = format!("{}/site/data?uuid={}", base_domain, uuid);

                let timer = crate::timer::CrawlTimer::new(app.clone());
                timer.emit_log(&format!(
                    "[Qilin] Phase 77: Attempting CMS bypass via {}",
                    data_json_url
                ));
                let _ = app.emit(
                    "log",
                    format!(
                        "[Qilin] Phase 77: Attempting CMS bypass via {}",
                        data_json_url
                    ),
                );
                println!(
                    "[Qilin Phase 77] Attempting CMS bypass via {}",
                    data_json_url
                );

                let (_, client) = frontier.get_client();

                // Strategy 1: Try JSON from /site/data
                match tokio::time::timeout(
                    std::time::Duration::from_secs(45),
                    client
                        .get(&data_json_url)
                        .header("X-Requested-With", "XMLHttpRequest")
                        .header("Accept", "application/json")
                        .send(),
                )
                .await
                {
                    Ok(Ok(resp)) if resp.status().is_success() => {
                        let final_url = resp.url().as_str().to_string();
                        if let Ok(body) = resp.text().await {
                            // Check if response is JSON
                            if body.trim().starts_with('[') || body.trim().starts_with('{') {
                                let bypass_entries = parse_qilin_json(&body, &base_domain);
                                if !bypass_entries.is_empty() {
                                    println!("[Qilin] Phase 77: ✅ JSON bypass successful, discovered {} entries.", bypass_entries.len());
                                    timer.emit_log(&format!(
                                        "[Qilin] ✅ JSON bypass successful. Discovered {} entries.",
                                        bypass_entries.len()
                                    ));
                                    let _ = app.emit("log", format!("[Qilin] ✅ JSON bypass successful. Discovered {} entries.", bypass_entries.len()));
                                    actual_seed_url = final_url.clone();
                                    json_bypass_entries = bypass_entries;
                                    goto_unleash = true;
                                }
                            }
                            // If redirected to a storage node autoindex, parse that HTML
                            if !goto_unleash
                                && (body.contains("<table id=\"list\">")
                                    || body.contains("<td class=\"link\">")
                                    || body.contains("Index of"))
                            {
                                println!("[Qilin Phase 77] Redirect landed on live autoindex. Using as seed.");
                                actual_seed_url = final_url;
                                // Don't set goto_unleash — let the normal crawl loop parse this URL
                            }
                        }
                    }
                    _ => {
                        println!("[Qilin] Phase 77: Strategy 1 (JSON/data) unavailable.");
                    }
                }

                // Strategy 2: If Strategy 1 failed, try /site/view HTML scraping
                if !goto_unleash {
                    let view_url = format!("{}/site/view?uuid={}", base_domain, uuid);
                    println!(
                        "[Qilin Phase 77] Strategy 2: Scraping CMS view page for listing data: {}",
                        view_url
                    );

                    let (_, client2) = frontier.get_client();
                    match tokio::time::timeout(
                        std::time::Duration::from_secs(45),
                        client2.get(&view_url).send(),
                    )
                    .await
                    {
                        Ok(Ok(resp)) if resp.status().is_success() => {
                            if let Ok(body) = resp.text().await {
                                // Parse the CMS view page for file listing tables
                                // The view page HTML contains item_box_photos, data tables, etc.
                                let view_entries = parse_qilin_view_page(&body, &base_domain, uuid);
                                if !view_entries.is_empty() {
                                    println!("[Qilin] Phase 77: ✅ View page bypass successful, discovered {} entries.", view_entries.len());
                                    timer.emit_log(&format!(
                                        "[Qilin] ✅ View page bypass discovered {} entries.",
                                        view_entries.len()
                                    ));
                                    let _ = app.emit(
                                        "log",
                                        format!(
                                            "[Qilin] ✅ View page bypass discovered {} entries.",
                                            view_entries.len()
                                        ),
                                    );
                                    actual_seed_url = view_url;
                                    json_bypass_entries = view_entries;
                                    goto_unleash = true;
                                } else {
                                    let watch_targets =
                                        extract_watch_data_targets(&base_domain, &body);
                                    if let Some(primary_watch_target) = watch_targets.first() {
                                        println!(
                                            "[Qilin Phase 77] View page exposed Watch data target. Promoting {} as active seed.",
                                            primary_watch_target
                                        );
                                        timer.emit_log(&format!(
                                            "[Qilin] View page exposed Watch data target. Promoting {}",
                                            primary_watch_target
                                        ));
                                        let _ = app.emit(
                                            "log",
                                            format!(
                                                "[Qilin] View page exposed Watch data target. Promoting {}",
                                                primary_watch_target
                                            ),
                                        );
                                        actual_seed_url = primary_watch_target.clone();
                                        for standby in watch_targets.into_iter().skip(1) {
                                            if !standby_routes.contains(&standby) {
                                                standby_routes.push(standby);
                                            }
                                        }
                                    }
                                    // Diagnostic: dump HTML for analysis
                                    let _ = std::fs::write("/tmp/qilin_view_page.html", &body);
                                    println!("[Qilin Phase 77] View page scraped ({} bytes) but no file entries found. HTML saved to /tmp/qilin_view_page.html", body.len());
                                    // Print first 500 chars for quick log diagnosis
                                    let preview = &body[..body.len().min(500)];
                                    println!(
                                        "[Qilin Phase 77] HTML preview: {}",
                                        preview.replace('\n', " ")
                                    );
                                }
                            }
                        }
                        _ => {
                            println!("[Qilin] Phase 77: Strategy 2 (view page) unavailable. Falling back to Phase 30.");
                        }
                    }
                }
            }
        } else if !is_dumb_mode && cms_launcher_url {
            let _ = app.emit(
                "log",
                "[Qilin] CMS launcher URL detected; skipping legacy Phase 77 bypass and using prioritized storage discovery."
                    .to_string(),
            );
        } else if !is_dumb_mode && direct_listing_url {
            let _ = app.emit(
                "log",
                "[Qilin] Direct storage listing URL detected; skipping CMS discovery path."
                    .to_string(),
            );
        } else if let Some(ref uuid) = extracted_uuid {
            // True naive path for benchmark mode: skip all CMS bypass heuristics and
            // go directly to the canonical storage listing URL.
            let base_domain = if let Ok(parsed) = reqwest::Url::parse(current_url) {
                format!("{}://{}", parsed.scheme(), parsed.host_str().unwrap_or(""))
            } else {
                current_url.split("/site/").next().unwrap_or("").to_string()
            };
            actual_seed_url = format!("{}/{}/", base_domain, uuid);
            let _ = app.emit(
                "log",
                format!(
                    "[Qilin] DUMB_MODE canonical seed selected: {}",
                    actual_seed_url
                ),
            );
        }

        if !goto_unleash && !is_dumb_mode && cms_launcher_url {
            if let Some(ref uuid_string) = extracted_uuid {
                let uuid = uuid_string.as_str();

                let timer = crate::timer::CrawlTimer::new(app.clone());
                timer.emit_log(&format!(
                    "[Qilin] Phase 30: Multi-node discovery for UUID: {}",
                    uuid
                ));
                let _ = app.emit(
                    "log",
                    format!("[Qilin] Phase 30: Multi-node discovery for UUID: {}", uuid),
                );
                println!(
                    "[Qilin Phase 30] Starting multi-node discovery for UUID: {}",
                    uuid
                );
                let Some(node_cache) = node_cache.clone() else {
                    eprintln!("[Qilin Phase 30] Node cache unavailable for UUID {}", uuid);
                    return Err(anyhow::anyhow!("Qilin node cache unavailable"));
                };

                let (_, client) = frontier.get_client();
                let fast_cached_node = tokio::time::timeout(
                    std::time::Duration::from_secs(15),
                    node_cache.try_fast_cached_route(uuid, &client, Some(&app)),
                )
                .await
                .ok()
                .flatten();

                let resolved_node = if let Some(node) = fast_cached_node {
                    timer.emit_log(&format!("[Qilin] Fast cached route hit: {}", node.host));
                    Some(node)
                } else {
                    // Run prioritized discovery with strict ordering:
                    // redirect target -> last known good -> top-ranked mirrors.
                    let discovery_result = tokio::time::timeout(
                        std::time::Duration::from_secs(90),
                        node_cache.discover_and_resolve_prioritized(
                            current_url,
                            uuid,
                            &client,
                            Some(&app),
                        ),
                    )
                    .await;
                    match discovery_result {
                        Ok(node) => node,
                        Err(_) => {
                            timer.emit_log("[Qilin] ⚠ Storage discovery timeout (90s). Falling back to direct mirrors...");
                            println!("[Qilin Phase 30] ⚠ Global discovery timeout after 90s");
                            let _ = app.emit("log", "[Qilin] Storage discovery timed out after 90s. Trying direct mirrors...".to_string());
                            None
                        }
                    }
                };
                if let Some(best_node) = resolved_node {
                    actual_seed_url = best_node.url.clone();
                    initial_repin_interval = best_node.recommended_repin_interval();
                    if let Some(telemetry) = &telemetry {
                        telemetry.set_current_node_host(best_node.host.clone());
                    }
                    standby_routes =
                        standby_seed_urls(&best_node.url, &node_cache.get_nodes(uuid).await, 4);
                    println!(
                        "[Qilin Phase 30] ✅ Resolved to storage node: {} ({}ms, {} hits)",
                        best_node.host, best_node.avg_latency_ms, best_node.hit_count
                    );
                    timer.emit_log(&format!(
                        "[Qilin] Storage Node Resolved: {} ({}ms avg latency)",
                        best_node.host, best_node.avg_latency_ms
                    ));
                    let _ = app.emit(
                        "log",
                        format!(
                            "[Qilin] Storage Node Resolved: {} ({}ms avg latency)",
                            best_node.host, best_node.avg_latency_ms
                        ),
                    );
                    if !standby_routes.is_empty() {
                        let _ = app.emit(
                            "log",
                            format!(
                                "[Qilin] Standby storage routes primed: {}",
                                standby_routes.join(" | ")
                            ),
                        );
                    }
                } else {
                    // Phase 42 Fix 4: Direct UUID retry with NEWNYM rotation
                    timer.emit_log("[Qilin] All storage nodes dead. Trying direct UUID construction with fresh circuits...");
                    println!("[Qilin Phase 42] ⚠ All storage nodes dead. Attempting direct UUID retry with NEWNYM...");
                    let _ = app.emit("log", "[Qilin] All storage nodes dead. Trying direct UUID construction with fresh circuits...".to_string());

                    // Blast NEWNYM to all active managed Tor daemons to get fresh circuits
                    let current_ports = crate::tor::detect_active_managed_tor_ports();
                    for port in current_ports {
                        let _ = crate::tor::request_newnym(port).await;
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

                    let mut retry_candidates: Vec<(String, String)> = node_cache
                        .get_nodes(uuid)
                        .await
                        .into_iter()
                        .take(6)
                        .map(|node| {
                            let url = if node.url.is_empty() {
                                format!("http://{}/{}/", node.host, uuid)
                            } else {
                                node.url
                            };
                            (node.host, url)
                        })
                        .collect();

                    if retry_candidates.is_empty() {
                        retry_candidates.extend([
                            (
                                "7mnkv5nvnjyifezlfyba6gek7aeimg5eghej5vp65qxnb2hjbtlttlyd.onion"
                                    .to_string(),
                                format!(
                                    "http://{}/{}/",
                                    "7mnkv5nvnjyifezlfyba6gek7aeimg5eghej5vp65qxnb2hjbtlttlyd.onion",
                                    uuid
                                ),
                            ),
                            (
                                "25mjg55vcbjzwykz2uqsvaw7hcevm4pqxl42o324zr6qf5zgddmghkqd.onion"
                                    .to_string(),
                                format!(
                                    "http://{}/{}/",
                                    "25mjg55vcbjzwykz2uqsvaw7hcevm4pqxl42o324zr6qf5zgddmghkqd.onion",
                                    uuid
                                ),
                            ),
                            (
                                "arrfcpipltlfgxc6hvjylixc6c5hrummwctz4wqysk3h56ntqz5scnad.onion"
                                    .to_string(),
                                format!(
                                    "http://{}/{}/",
                                    "arrfcpipltlfgxc6hvjylixc6c5hrummwctz4wqysk3h56ntqz5scnad.onion",
                                    uuid
                                ),
                            ),
                        ]);
                    }

                    let retry_wave_size = 2;
                    let total_retry_waves = retry_candidates.len().div_ceil(retry_wave_size);
                    let mut found_alive_node: Option<(String, String)> = None;

                    for (wave_idx, chunk) in retry_candidates.chunks(retry_wave_size).enumerate() {
                        let _ = app.emit(
                            "log",
                            format!(
                                "[Qilin] Direct mirror retry wave {}/{} with {} candidates",
                                wave_idx + 1,
                                total_retry_waves.max(1),
                                chunk.len()
                            ),
                        );

                        let mut mirror_tasks = tokio::task::JoinSet::new();
                        for (mirror_host, mirror_url) in chunk.iter().cloned() {
                            let frontier_ref = frontier.clone();
                            mirror_tasks.spawn(async move {
                                let (_, client) = frontier_ref.get_client();
                                let isolated = client.new_isolated();
                                println!("[Qilin Phase 42] Probing direct mirror: {}", mirror_url);
                                match tokio::time::timeout(
                                    // Phase 76D: Aligned with arti connect_timeout=30s for HS cold-start
                                    std::time::Duration::from_secs(30),
                                    isolated.get(&mirror_url).send(),
                                )
                                .await
                                {
                                    Ok(Ok(resp))
                                        if resp.status().is_success()
                                            || resp.status().as_u16() == 301
                                            || resp.status().as_u16() == 302 =>
                                    {
                                        let final_url = resp.url().as_str().to_string();
                                        if let Ok(body) = resp.text().await {
                                            if body.contains("<table id=\"list\">")
                                                || body.contains("Index of")
                                                || body.contains("<td class=\"link\">")
                                                || body.contains("QData")
                                                || body.contains("Data browser")
                                            {
                                                return Some((
                                                    mirror_host,
                                                    if final_url != mirror_url {
                                                        final_url
                                                    } else {
                                                        mirror_url
                                                    },
                                                ));
                                            }
                                        }
                                    }
                                    Ok(Ok(resp)) => {
                                        println!(
                                            "[Qilin Phase 42] Mirror {} responded with {}",
                                            mirror_host,
                                            resp.status()
                                        );
                                    }
                                    Ok(Err(e)) => {
                                        println!(
                                            "[Qilin Phase 42] Mirror {} unreachable: {}",
                                            mirror_host, e
                                        );
                                    }
                                    Err(_) => {
                                        println!(
                                            "[Qilin Phase 42] Mirror {} timed out",
                                            mirror_host
                                        );
                                    }
                                }
                                None
                            });
                        }

                        while let Some(joined) = mirror_tasks.join_next().await {
                            if let Ok(Some(node)) = joined {
                                mirror_tasks.abort_all();
                                found_alive_node = Some(node);
                                break;
                            }
                        }

                        if found_alive_node.is_some() {
                            break;
                        }
                    }

                    let found_alive = if let Some((mirror_host, mirror_url)) = found_alive_node {
                        println!("[Qilin Phase 42] ✅ Direct mirror alive: {}", mirror_host);
                        timer.emit_log(&format!("[Qilin] ✅ Direct mirror alive: {}", mirror_host));
                        let _ = app.emit(
                            "log",
                            format!("[Qilin] ✅ Direct mirror alive: {}", mirror_host),
                        );
                        actual_seed_url = mirror_url;
                        node_cache
                            .seed_node(uuid, &actual_seed_url, &mirror_host)
                            .await;
                        if let Some(telemetry) = &telemetry {
                            telemetry.set_current_node_host(mirror_host);
                        }
                        true
                    } else {
                        false
                    };

                    if !found_alive {
                        println!(
                            "[Qilin Phase 42] ⚠ No alive mirrors found. Attempting Phase 77 CMS view page bypass."
                        );
                        timer.emit_log("[Qilin] No alive storage nodes. Attempting Phase 77 CMS view page bypass...");
                        let _ = app.emit("log", "[Qilin] No alive storage nodes. Attempting Phase 77 CMS view page bypass...".to_string());

                        // Phase 77 Post-Discovery: View page scraping as last resort
                        // Circuits are now warm after Stage B, so the CMS is reliably reachable
                        let view_url = format!(
                            "{}/site/view?uuid={}",
                            current_url.split("/site/").next().unwrap_or("").to_string(),
                            uuid
                        );
                        let (_, bypass_client) = frontier.get_client();
                        match tokio::time::timeout(
                            std::time::Duration::from_secs(60),
                            bypass_client.get(&view_url).send(),
                        )
                        .await
                        {
                            Ok(Ok(resp)) if resp.status().is_success() => {
                                if let Ok(body) = resp.text().await {
                                    let base_domain = current_url
                                        .split("/site/")
                                        .next()
                                        .unwrap_or("")
                                        .to_string();
                                    let view_entries =
                                        parse_qilin_view_page(&body, &base_domain, uuid);
                                    if !view_entries.is_empty() {
                                        println!("[Qilin] Phase 77 Post-Discovery: ✅ View page bypass found {} entries!", view_entries.len());
                                        timer.emit_log(&format!(
                                            "[Qilin] ✅ Phase 77 view page discovered {} entries.",
                                            view_entries.len()
                                        ));
                                        let _ = app.emit("log", format!("[Qilin] ✅ Phase 77 view page discovered {} entries.", view_entries.len()));
                                        actual_seed_url = view_url;
                                        json_bypass_entries = view_entries;
                                        goto_unleash = true;
                                    } else {
                                        let watch_targets =
                                            extract_watch_data_targets(&base_domain, &body);
                                        if let Some(primary_watch_target) = watch_targets.first() {
                                            println!(
                                                "[Qilin Phase 77] Post-Discovery: promoting Watch data target {}",
                                                primary_watch_target
                                            );
                                            timer.emit_log(&format!(
                                                "[Qilin] Phase 77 promoted Watch data target {}",
                                                primary_watch_target
                                            ));
                                            let _ = app.emit(
                                                "log",
                                                format!(
                                                    "[Qilin] Phase 77 promoted Watch data target {}",
                                                    primary_watch_target
                                                ),
                                            );
                                            actual_seed_url = primary_watch_target.clone();
                                            for standby in watch_targets.into_iter().skip(1) {
                                                if !standby_routes.contains(&standby) {
                                                    standby_routes.push(standby);
                                                }
                                            }
                                        }
                                        let _ = std::fs::write("/tmp/qilin_view_page.html", &body);
                                        println!("[Qilin Phase 77] Post-Discovery: View page scraped ({} bytes) but no entries. HTML saved to /tmp/qilin_view_page.html", body.len());
                                        let preview = &body[..body.len().min(500)];
                                        println!(
                                            "[Qilin Phase 77] HTML preview: {}",
                                            preview.replace('\n', " ")
                                        );
                                    }
                                }
                            }
                            _ => {
                                println!("[Qilin Phase 77] Post-Discovery: View page unreachable. Falling back to CMS URL.");
                                timer.emit_log(
                                    "[Qilin] Using CMS URL directly (limited results expected).",
                                );
                                let _ = app.emit(
                                    "log",
                                    "[Qilin] Using CMS URL directly (limited results expected)."
                                        .to_string(),
                                );
                            }
                        }
                    }
                }
            }
        }

        let subtree_summary_path = frontier
            .target_paths()
            .map(|paths| paths.target_dir.join("qilin_subtree_route_summary.json"));
        let route_plan = Arc::new(QilinRoutePlan::new(
            actual_seed_url.clone(),
            standby_routes,
            telemetry.clone(),
            subtree_summary_path.clone(),
        ));
        if let Ok(loaded) = route_plan.load_persisted_subtree_preferences() {
            if loaded > 0 {
                let _ = app.emit(
                    "log",
                    format!(
                        "[Qilin] Restored {} persisted subtree host preferences{}",
                        loaded,
                        subtree_summary_path
                            .as_ref()
                            .map(|path| format!(" from {}", path.display()))
                            .unwrap_or_default()
                    ),
                );
            }
        }
        let durable_uuid = extracted_uuid.clone();
        let durable_node_cache = node_cache.clone();
        let durable_root_confirmed = Arc::new(AtomicBool::new(false));
        let durable_winner_url = Arc::new(Mutex::new(None::<String>));

        // Reverted to Strict Depth-First Search parsing (Phase 27)
        if !goto_unleash {
            queue.push(actual_seed_url.clone());
            frontier.mark_visited(&actual_seed_url);
        }

        // Phase 76: Install the global heatmap now that actual_seed_url is finalized
        if subtree_shaping_enabled {
            let hm_snapshot = heatmap.lock().await.clone();
            SubtreeHeatmap::install_global_heatmap(actual_seed_url.clone(), hm_snapshot);
        }

        let pending = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        // Phase 67E: Shared counter for real-time entry count visibility
        let discovered_entries = Arc::new(std::sync::atomic::AtomicUsize::new(0));

        let (ui_tx, mut ui_rx) = mpsc::channel::<FileEntry>(4096);

        if goto_unleash {
            discovered_entries.fetch_add(
                json_bypass_entries.len(),
                std::sync::atomic::Ordering::Relaxed,
            );
            for entry in &json_bypass_entries {
                if entry.entry_type == EntryType::Folder && frontier.mark_visited(&entry.raw_url) {
                    pending.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    queue.push(entry.raw_url.clone());
                }
                let _ = ui_tx.try_send(entry.clone());
            }
        } else {
            pending.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        let ui_app = app.clone();
        let vfs_for_batches = vfs.clone();
        let collected_entries_for_batches = collected_entries.clone();
        let ui_flush_task = tokio::spawn(async move {
            let mut batch = Vec::new();
            // Phase 78: 2000ms flush interval to drastically reduce rapid React rendering overhead on massive QData targets
            let mut interval = tokio::time::interval(std::time::Duration::from_millis(2000));
            let mut channel_closed = false;
            loop {
                tokio::select! {
                    entry = ui_rx.recv(), if !channel_closed => {
                        match entry {
                            Some(entry) => {
                                batch.push(entry);
                                // Phase 78: Zero-Copy Sled Streaming + Batched VFS Flush ceiling increased to 5000
                                if batch.len() >= 5000 {
                                    if let Some(vfs) = &vfs_for_batches {
                                        let _ = vfs.insert_entries(&batch).await;
                                    }
                                    let _ = ui_app.emit("crawl_progress", batch.clone());
                                    if collect_results_locally {
                                        let mut guard = collected_entries_for_batches.lock().await;
                                        guard.extend(batch.iter().cloned());
                                    }
                                    batch.clear();
                                }
                            }
                            None => {
                                channel_closed = true;
                            }
                        }
                    }
                    _ = interval.tick() => {
                        if !batch.is_empty() {
                            if let Some(vfs) = &vfs_for_batches {
                                let _ = vfs.insert_entries(&batch).await;
                            }
                            let _ = ui_app.emit("crawl_progress", batch.clone());
                            if collect_results_locally {
                                let mut guard = collected_entries_for_batches.lock().await;
                                guard.extend(batch.iter().cloned());
                            }
                            batch.clear();
                        }
                    }
                }
                if channel_closed && batch.is_empty() {
                    break;
                }
            }
            if !batch.is_empty() {
                if let Some(vfs) = &vfs_for_batches {
                    let _ = vfs.insert_entries(&batch).await;
                }
                let _ = ui_app.emit("crawl_progress", batch.clone());
                if collect_results_locally {
                    let mut guard = collected_entries_for_batches.lock().await;
                    guard.extend(batch);
                }
            }
        });

        let reserve_for_downloads = frontier.active_options.download;
        // Phase 74: Separate governor worker pool size from TorClient multi_clients pool.
        // Cap max_active at qilin_workers (e.g. 64) while multi_clients limits underlying Tor circuits.
        let qilin_workers = std::env::var("CRAWLI_QILIN_WORKERS")
            .ok()
            .and_then(|v| v.trim().parse::<usize>().ok())
            .unwrap_or_else(|| frontier.active_options.circuits.unwrap_or(120).min(64))
            .min(128)
            .max(1);

        let governor = Arc::new(QilinCrawlGovernor::new(
            qilin_workers,
            reserve_for_downloads,
            telemetry.clone(),
        ));
        governor.set_repin_interval_hint(initial_repin_interval);
        let max_concurrent = governor.max_active;
        sync_qilin_frontier_progress(
            frontier.as_ref(),
            telemetry.as_ref(),
            pending.as_ref(),
            max_concurrent,
        );
        let degraded_lane_limit = degraded_lane_limit(max_concurrent);
        let degraded_lane_interval = degraded_lane_interval();
        let governor_interval = governor_rebalance_interval();
        let degraded_in_flight = Arc::new(AtomicUsize::new(0));
        let degraded_dispatch_counter = Arc::new(AtomicUsize::new(0));
        let mut workers = tokio::task::JoinSet::new();

        let _ = app.emit(
            "log",
            format!(
                "[Qilin] Adaptive page governor online: target={} max={} reserve_for_downloads={} degraded_lane_max={} degraded_lane_interval={} {}",
                governor.current_target(),
                max_concurrent,
                reserve_for_downloads,
                degraded_lane_limit,
                degraded_lane_interval,
                governor.traffic_summary()
            ),
        );

        {
            let governor = governor.clone();
            let pending = pending.clone();
            let discovered_entries_gov = discovered_entries.clone();
            let cancel_flag = frontier.cancel_flag.clone();
            let frontier_for_governor = frontier.clone();
            let app = app.clone();
            let governor_interval = governor_interval;
            tokio::spawn(async move {
                let mut idle_rounds = 0u8;
                let mut last_entries = 0usize;
                let mut last_processed = 0usize;
                let mut no_progress_rounds = 0u8;
                loop {
                    tokio::time::sleep(governor_interval).await;
                    if cancel_flag.load(Ordering::Relaxed) {
                        break;
                    }

                    let pending_now = pending.load(Ordering::Relaxed);
                    let in_flight = governor.in_flight.load(Ordering::Relaxed);
                    let discovered_now = discovered_entries_gov.load(Ordering::Relaxed);
                    let processed_now = frontier_for_governor.progress_snapshot().processed;
                    if pending_now == 0 && in_flight == 0 {
                        idle_rounds = idle_rounds.saturating_add(1);
                        no_progress_rounds = 0;
                        if idle_rounds >= 2 {
                            break;
                        }
                    } else {
                        idle_rounds = 0;
                        if discovered_now == last_entries && processed_now == last_processed {
                            no_progress_rounds = no_progress_rounds.saturating_add(1);
                        } else {
                            no_progress_rounds = 0;
                        }
                    }
                    last_entries = discovered_now;
                    last_processed = processed_now;

                    if no_progress_rounds >= 2 {
                        if let Some((cid, avg_ms, baseline_ms)) =
                            governor.latency_outlier_candidate()
                        {
                            if governor.mark_latency_outlier_isolation(cid) {
                                let message = format!(
                                    "[Qilin StallGuard] No progress for {} intervals with pending={} in_flight={}; isolating c{} ({}ms avg vs {}ms baseline).",
                                    no_progress_rounds, pending_now, in_flight, cid, avg_ms, baseline_ms
                                );
                                let _ = app.emit("crawl_log", message.clone());
                                println!("{}", message);
                                frontier_for_governor.trigger_circuit_isolation(cid).await;
                                if let Some(telemetry) = &governor.telemetry {
                                    telemetry.record_outlier_isolation();
                                    telemetry
                                        .record_failover(format!("stall_guard_circuit_{}", cid));
                                }
                                no_progress_rounds = 0;
                            }
                        } else {
                            no_progress_rounds = 0;
                        }
                    }

                    if let Some(telemetry) = &governor.telemetry {
                        if let Some(summary) = governor.slowest_latency_summary() {
                            telemetry.set_slowest_circuit(summary);
                        }
                    }

                    if let Some((next, pressure)) = governor.rebalance(pending_now) {
                        // Phase 74B: Apply EWMA error decay during each rebalance
                        governor.apply_ewma_error_decay();

                        let latency = governor.circuit_latency_summary();
                        let adaptive_to = governor.adaptive_timeout_secs();
                        let ceiling = governor.adaptive_ramp_ceiling.load(Ordering::Relaxed);

                        // Phase 74B: Emit ceiling change events to Tauri frontend
                        if let Some((prev_ceil, new_ceil)) = governor.ceiling_change_info() {
                            let direction = if new_ceil < prev_ceil {
                                "DECAY"
                            } else {
                                "RECOVERY"
                            };
                            let _ =
                                app.emit(
                                    "crawl_log",
                                    format!(
                                    "[PHASE 74] Adaptive ceiling {}: {} → {} (throttle window={})",
                                    direction, prev_ceil, new_ceil,
                                    governor.throttles_in_window.load(Ordering::Relaxed)
                                ),
                                );
                            println!(
                                "[PHASE 74] Adaptive ceiling {}: {} → {} (throttle window={})",
                                direction,
                                prev_ceil,
                                new_ceil,
                                governor.throttles_in_window.load(Ordering::Relaxed)
                            );
                        }

                        let _ = app.emit(
                            "log",
                            format!(
                                "[Qilin] Governor: workers={} pending={} in_flight={} pressure={:.2} entries={} timeout={}s ceiling={} latency=[{}]",
                                next, pending_now, in_flight, pressure,
                                discovered_entries_gov.load(Ordering::Relaxed),
                                adaptive_to, ceiling, latency
                            ),
                        );
                        println!(
                            "[Qilin] Governor: workers={} pending={} in_flight={} pressure={:.2} entries={} timeout={}s ceiling={} latency=[{}] {}",
                            next, pending_now, in_flight, pressure,
                            discovered_entries_gov.load(Ordering::Relaxed),
                            adaptive_to, ceiling, latency,
                            governor.traffic_summary()
                        );
                    } else if idle_rounds == 0 {
                        // Phase 67E+F: Periodic progress with latency
                        let total = discovered_entries_gov.load(Ordering::Relaxed);
                        if total > 0 {
                            println!(
                                "[Qilin Progress] entries={} pending={} in_flight={} latency=[{}]",
                                total,
                                pending_now,
                                in_flight,
                                governor.circuit_latency_summary()
                            );
                        }
                    }
                }
            });
        }

        // Phase 76B: Periodic heatmap refresh — pushes live data to global
        // `is_subtree_penalized()` every 30s + saves to disk/Sled for durability.
        if subtree_heatmap_enabled {
            let heatmap_r = heatmap.clone();
            let heatmap_path_r = heatmap_path.clone();
            let vfs_r = vfs.clone();
            let cancel_flag_r = frontier.cancel_flag.clone();
            let active_seed_r = actual_seed_url.clone();
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(Duration::from_secs(30));
                interval.tick().await; // skip immediate first tick
                loop {
                    interval.tick().await;
                    if cancel_flag_r.load(Ordering::Relaxed) {
                        break;
                    }

                    // 1. Refresh global heatmap for cross-thread is_subtree_penalized()
                    {
                        let hm = heatmap_r.lock().await;
                        SubtreeHeatmap::refresh_global_heatmap(&hm);
                    }

                    // 2. Persist to JSON (best-effort)
                    if let Some(path) = &heatmap_path_r {
                        let hm = heatmap_r.lock().await;
                        if let Err(err) = hm.save(path) {
                            println!("[Qilin Heatmap] Periodic save failed: {}", err);
                        }
                    }

                    // 3. Persist to Sled VFS (best-effort)
                    if let Some(vfs_ref) = &vfs_r {
                        let hm = heatmap_r.lock().await;
                        let records: Vec<crate::subtree_heatmap::SubtreeHeatRecord> =
                            hm.entries.values().cloned().collect();
                        if !records.is_empty() {
                            if let Err(err) = vfs_ref.upsert_heatmap_batch(&records).await {
                                println!("[Qilin Heatmap] Periodic Sled save failed: {}", err);
                            }
                        }
                    }

                    println!(
                        "[Qilin Heatmap] Periodic refresh: {} subtrees tracked for {}",
                        heatmap_r.lock().await.entries.len(),
                        active_seed_r
                    );
                }
            });
        }
        let parsed_url = reqwest::Url::parse(current_url)?;
        let base_domain = format!(
            "{}://{}",
            parsed_url.scheme(),
            parsed_url.host_str().unwrap_or("")
        );

        // Phase 67B: Separate pool size (TorClient count) from circuits_ceiling (worker budget).
        // circuits_ceiling controls how many workers/circuits operate; pool size controls
        // how many actual TorClient instances are created. Creating 120 TorClients is catastrophic.
        let circuits_ceiling = frontier.active_options.circuits.unwrap_or(120);

        let qilin_workers = std::env::var("CRAWLI_QILIN_WORKERS")
            .ok()
            .and_then(|v| v.trim().parse::<usize>().ok())
            .unwrap_or_else(|| circuits_ceiling.min(64))
            .min(128)
            .max(1);

        let multi_clients = std::env::var("CRAWLI_MULTI_CLIENTS")
            .ok()
            .and_then(|v| v.trim().parse::<usize>().ok())
            // Phase 74: Cap at 8 TorClients normally to prevent Mac OS overload
            .unwrap_or_else(|| circuits_ceiling.min(8))
            .min(8)
            .max(1);

        let crawl_started_at = std::time::Instant::now();
        let timer = crate::timer::CrawlTimer::new(app.clone());
        timer.emit_log(&format!(
            "[Qilin] Bootstrapping MultiClientPool with {} TorClients (Multiplexing {} workers)",
            multi_clients, qilin_workers
        ));
        let _ = app.emit(
            "log",
            format!(
                "[Qilin] Bootstrapping MultiClientPool with {} TorClients (Multiplexing {} workers)",
                multi_clients, qilin_workers
            ),
        );
        let multi_pool = if frontier.is_onion {
            let seeded_clients = if let Some(guard_arc) = &frontier.swarm_guard {
                let guard = guard_arc.lock().await;
                let shared_clients = guard.get_arti_clients();
                crate::multi_client_pool::snapshot_seed_clients(&shared_clients, multi_clients)
            } else {
                Vec::new()
            };
            let pool = Arc::new(
                crate::multi_client_pool::MultiClientPool::new_seeded(
                    multi_clients,
                    seeded_clients,
                    telemetry.clone(),
                )
                .await
                .unwrap(),
            );

            if pool.borrowed_client_count() > 0 {
                let message = format!(
                    "[Qilin] Seeded MultiClientPool with {}/{} hot Arti clients from the active swarm.",
                    pool.borrowed_client_count(),
                    multi_clients
                );
                timer.emit_log(&message);
                let _ = app.emit("log", message);
            }

            timer.emit_log("[Qilin] Concurrent Pre-heating of MultiClientPool circuits to cache HS descriptors...");
            let _ = app.emit(
                "log",
                "[Qilin] Concurrent Pre-heating of MultiClientPool circuits to cache HS descriptors..."
                    .to_string(),
            );
            // Phase 67/74: Fire-and-Forget lazily-evaluated preheat.
            // Spawns tasks to get_client() concurrently so lazy-booting unblocks setup.
            // Lazy-loading 74: Only preheat the circuits we are actually about to use in max_concurrent
            // (the starting target, e.g. 6), NOT the entire ceiling (16), to prevent massive
            // upfront network/CPU stalling during the initial scan wave.
            let mut preheats = Vec::new();
            let preheat_limit = std::cmp::min(multi_clients, max_concurrent.max(2));
            for i in 0..preheat_limit {
                let multi_pool_clone = pool.clone();
                let target_heat_url = actual_seed_url.clone();
                preheats.push(tokio::spawn(async move {
                    let tor_arc = multi_pool_clone.get_client(i).await;
                    let preheat_client =
                        crate::arti_client::ArtiClient::new((*tor_arc).clone(), None);
                    let _ = tokio::time::timeout(
                        std::time::Duration::from_secs(20),
                        preheat_client.head(&target_heat_url).send(),
                    )
                    .await;
                }));
            }
            // If we already borrowed hot clients from the active swarm, don't stall crawl start.
            if pool.borrowed_client_count() > 0 {
                drop(preheats);
            } else if !preheats.is_empty() {
                // Gate: wait for ANY single client to finish preheating, then unleash workers immediately
                let (result, _index, remaining) = futures::future::select_all(preheats).await;
                let _ = result; // Ignore errors — circuit warmup is best-effort
                                // Fire-and-forget remaining preheats as background tasks
                for handle in remaining {
                    tokio::spawn(async move {
                        let _ = handle.await;
                    });
                }
            }
            Some(pool)
        } else {
            None
        };
        timer.emit_log("[Qilin] First circuit hot. Unleashing workers (remaining circuits warming in background).");
        let _ = app.emit(
            "log",
            "[Qilin] First circuit hot. Unleashing workers (remaining warming in background)."
                .to_string(),
        );

        for worker_idx in 0..max_concurrent {
            let f = frontier.clone();
            let pool_clone = multi_pool.clone();
            let q_clone = queue.clone();
            let retry_q_clone = retry_queue.clone();
            let degraded_retry_q_clone = degraded_retry_queue.clone();
            let ui_tx_clone = ui_tx.clone();
            let ui_app_clone = app.clone();
            let pending_clone = pending.clone();
            let discovered_clone = discovered_entries.clone();
            let domain_clone = base_domain.clone();
            let crawl_governor = governor.clone();
            let route_plan = route_plan.clone();
            let root_fetch_logged = root_fetch_logged.clone();
            let root_parse_logged = root_parse_logged.clone();
            let child_queue_logged = child_queue_logged.clone();
            let child_fetch_logged = child_fetch_logged.clone();
            let child_parse_logged = child_parse_logged.clone();
            let child_failure_logged = child_failure_logged.clone();
            let child_retry_lane_logged = child_retry_lane_logged.clone();
            let degraded_in_flight = degraded_in_flight.clone();
            let degraded_dispatch_counter = degraded_dispatch_counter.clone();
            let heatmap = heatmap.clone();
            let telemetry_clone = telemetry.clone();
            let durable_uuid = durable_uuid.clone();
            let durable_node_cache = durable_node_cache.clone();
            let durable_root_confirmed = durable_root_confirmed.clone();
            let durable_winner_url = durable_winner_url.clone();

            let df_clone = discovered_folders.clone();
            let vf_clone = visited_folders.clone();

            workers.spawn(async move {
                let ramp_initial = std::env::var("CRAWLI_VANGUARD_INITIAL")
                    .ok()
                    .and_then(|v| v.parse::<usize>().ok())
                    .unwrap_or(1);
                let ramp_interval_ms = std::env::var("CRAWLI_VANGUARD_RAMP_INTERVAL_MS")
                    .ok()
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(2500);

                if f.stealth_ramp_active() && worker_idx >= ramp_initial {
                    let offset_ms = (worker_idx - ramp_initial + 1) as u64 * ramp_interval_ms;
                    let mut slept = 0;
                    while slept < offset_ms {
                        if f.is_cancelled() { return; }
                        let current_workers = f.active_workers();
                        if crawl_governor.should_halt_ramp(current_workers) {
                            let _ = ui_app_clone.emit(
                                "log",
                                format!("[Vanguard] Halting worker induction at {}/{} due to server 503s.", worker_idx, max_concurrent)
                            );
                            return;
                        }
                        darwin_kqueue_spinlock(100).await;
                        slept += 100;
                    }
                    let current_workers = f.active_workers();
                    if crawl_governor.should_halt_ramp(current_workers) { return; }
                    let _ = ui_app_clone.emit("log", format!("[Qilin Vanguard] Inducting worker {}/{}", worker_idx + 1, max_concurrent));
                }

                let mut ddos = crate::adapters::qilin_ddos_guard::DdosGuard::new();
                // Phase 76B: Quarantine queue for throttled URLs (per-worker, non-blocking)
                let quarantine_queue = crossbeam_queue::SegQueue::<RetryPayload>::new();
                let mut idle_sleep_ms: u64 = 50;
                let mut worker_client: Option<(usize, crate::arti_client::ArtiClient)> = None;
                let mut worker_req_count: usize = 0; // Phase 67I: request counter for circuit re-eval
                loop {
                    if f.is_cancelled() {
                        break;
                    }

                                        let mut degraded_lane_permit = None;
                    let (mut next_url, current_attempt) = loop {
                        if f.is_cancelled() { return; }

                        // 1. Quarantine (Highest priority)
                        if !quarantine_queue.is_empty() {
                            if let Some(payload) = take_due_retry(&quarantine_queue) {
                                idle_sleep_ms = 50;
                                break (payload.url, payload.attempt);
                            }
                        }

                        // 2. Primary Retries (Inverted Priority - clear stalled tail)
                        if !retry_q_clone.is_empty() {
                            if let Some(payload) = take_due_retry(&retry_q_clone) {
                                let current_seed = route_plan.current_seed_url_sync();
                                if subtree_shaping_enabled && SubtreeHeatmap::is_subtree_penalized(&payload.url) && !payload.url.starts_with(&current_seed) {
                                    degraded_retry_q_clone.push(payload);
                                } else {
                                    idle_sleep_ms = 50;
                                    break (payload.url, payload.attempt);
                                }
                            }
                        }

                        // 3. Degraded Lane Retries
                        if !degraded_retry_q_clone.is_empty() {
                            let should_probe_degraded = degraded_dispatch_counter.fetch_add(1, Ordering::Relaxed) % degraded_lane_interval == 0;
                            if should_probe_degraded {
                                if let Some(permit) = try_acquire_degraded_lane(&degraded_in_flight, degraded_lane_limit) {
                                    if let Some(payload) = take_due_retry(&degraded_retry_q_clone) {
                                        degraded_lane_permit = Some(permit);
                                        idle_sleep_ms = 50;
                                        emit_limited_child_log(
                                            &ui_app_clone,
                                            &child_retry_lane_logged,
                                            "RetryLane",
                                            format!("dispatch lane=degraded url={} attempt={} pending={}", payload.url, payload.attempt, pending_clone.load(Ordering::SeqCst))
                                        );
                                        break (payload.url, payload.attempt);
                                    } else {
                                        drop(permit);
                                    }
                                }
                            }
                        }

                        // 4. Primary Queue (Lowest priority for fetching NEW work)
                        if let Some(url) = q_clone.pop() {
                            idle_sleep_ms = 50;
                            break (url, 1);
                        }

                        // 5. Idle Return
                        if pending_clone.load(Ordering::SeqCst) == 0 && retry_q_clone.is_empty() && degraded_retry_q_clone.is_empty() && quarantine_queue.is_empty() {
                            return; // Worker terminates naturally
                        }

                        darwin_kqueue_spinlock(idle_sleep_ms).await;
                        idle_sleep_ms = std::cmp::min(idle_sleep_ms * 2, 150);
                    };

                    // 95% fast path: zero-intelligence request execution unless
                    // we hit an anomaly (retry) or a hot subtree.
                    let active_seed = route_plan.current_seed_url_sync();
                    let use_slow_path = !is_dumb_mode
                        && (current_attempt > 1
                            || crate::subtree_heatmap::SubtreeHeatmap::is_subtree_penalized(
                                &next_url,
                            ));
                    if use_slow_path {
                        let is_root_retry = is_root_retry_url(&active_seed, &next_url);
                        let keep_active_seed = should_keep_child_retry_on_active_seed(
                            &active_seed,
                            &next_url,
                            current_attempt,
                        );
                        let pinned_seed = route_plan
                            .preferred_seed_for_request(worker_idx, &next_url, current_attempt)
                            .unwrap_or_else(|| route_plan.worker_node_url(worker_idx));
                        if !is_root_retry && !keep_active_seed && !pinned_seed.is_empty() {
                            if let Some((current_seed, _)) =
                                split_qilin_seed_and_relative_path(&next_url)
                            {
                                if pinned_seed != current_seed {
                                    next_url =
                                        remap_seed_url(&next_url, &current_seed, &pinned_seed);
                                }
                            }
                        }
                    }
                    let _active_request_guard = QilinActiveRequestGuard::new(
                        f.clone(),
                        telemetry_clone.clone(),
                        max_concurrent,
                    );
                    let _degraded_lane_permit = degraded_lane_permit;

                    let _crawl_slot = if use_slow_path {
                        Some(crawl_governor.acquire_slot().await)
                    } else {
                        None
                    };
                    let _permit = if use_slow_path {
                        f.politeness_semaphore.acquire().await.ok()
                    } else {
                        None
                    };

                    let (cid, client) = if let Some((cid, client)) = &worker_client {
                        (*cid, client.clone())
                    } else if let Some(pool) = &pool_clone {
                        let best_cid = if use_slow_path {
                            f.scorer.best_circuit_for_url(multi_clients)
                        } else {
                            worker_idx % multi_clients.max(1)
                        };
                        let tor_arc = pool.get_client(best_cid).await;
                        let cid = best_cid;
                        let client = crate::arti_client::ArtiClient::new(
                            (*tor_arc).clone(),
                            None,
                        );
                        worker_client = Some((cid, client.clone()));
                        (cid, client)
                    } else {
                        let (cid, client) = f.get_client();
                        worker_client = Some((cid, client.clone()));
                        (cid, client)
                    };

                    let delay = if use_slow_path {
                        f.scorer.yield_delay(cid)
                    } else {
                        std::time::Duration::ZERO
                    };
                    if delay > std::time::Duration::ZERO {
                        darwin_kqueue_spinlock(delay.as_millis() as u64).await;
                    }

                    let _guard = QilinPendingGuard {
                        frontier: f.clone(),
                        telemetry: telemetry_clone.clone(),
                        pending: pending_clone.clone(),
                        worker_target: max_concurrent,
                    };
                    let active_seed_url = route_plan.current_seed_url().await;
                    route_plan.record_request_route(&active_seed_url, &next_url);
                    if next_url == active_seed_url
                        && !root_fetch_logged.swap(true, std::sync::atomic::Ordering::AcqRel)
                    {
                        println!(
                            "[Qilin Root Fetch] cid={} url={} attempt={} pending={}",
                            cid,
                            next_url,
                            current_attempt,
                            pending_clone.load(std::sync::atomic::Ordering::SeqCst)
                        );
                    } else if next_url != active_seed_url {
                        emit_limited_child_log(
                            &ui_app_clone,
                            &child_fetch_logged,
                            "Fetch",
                            format!(
                                "cid={} url={} attempt={} pending={}",
                                cid,
                                next_url,
                                current_attempt,
                                pending_clone.load(std::sync::atomic::Ordering::SeqCst)
                            ),
                        );
                    }

                    let start_time = std::time::Instant::now();
                    let url_depth = url_depth_relative(&active_seed_url, &next_url);
                    let should_speculate = use_slow_path && multi_clients >= 2 && url_depth > 2;
                    let req_timeout_secs = if use_slow_path {
                        crawl_governor.adaptive_timeout_secs()
                    } else {
                        35
                    };
                    let resp_result: Result<Result<crate::arti_client::ArtiResponse, anyhow::Error>, _> = if should_speculate {
                        let adaptive_timeout = std::time::Duration::from_secs(req_timeout_secs);
                        // Phase 76C: Select speculative circuit from listing partition
                        let spec_cid = if crawl_governor.reserve_for_downloads {
                            crawl_governor.circuit_for_class(TrafficClass::Listing, &f.scorer)
                        } else {
                            (worker_idx + 1) % multi_clients
                        };

                        let p_clone = pool_clone.clone();
                        let spec_url = next_url.clone();

                        let spec_fut = tokio::time::timeout(adaptive_timeout, async move {
                            if let Some(p) = p_clone {
                                let spec_tor = p.get_client(spec_cid).await;
                                let spec_client = crate::arti_client::ArtiClient::new((*spec_tor).clone(), None);
                                spec_client.get(&spec_url).send().await
                            } else {
                                let spec_client = crate::arti_client::ArtiClient::new_clearnet();
                                spec_client.get(&spec_url).send().await
                            }
                        });

                        let primary_fut = tokio::time::timeout(
                            adaptive_timeout,
                            client.get(&next_url).send(),
                        );

                        tokio::select! {
                            biased; // Prefer primary circuit
                            r = primary_fut => r,
                            r = spec_fut => r,
                        }
                    } else {
                        tokio::time::timeout(
                            std::time::Duration::from_secs(req_timeout_secs),
                            client.get(&next_url).send()
                        ).await
                    };

                    let mut html = None;
                    let mut effective_url = next_url.clone();
                    let mut should_retry = false;
                    let mut retry_failure_kind = CrawlFailureKind::Http;
                    let mut response_latency_ms = None;

                    if let Ok(Ok(resp)) = resp_result {
                        let elapsed_ms = start_time.elapsed().as_millis() as u64;
                        response_latency_ms = Some(elapsed_ms);
                        let status = resp.status();

                        if use_slow_path {
                            match ddos.record_response(status.as_u16()) {
                                crate::adapters::qilin_ddos_guard::DdosOutcome::Proceed(Some(delay)) => {
                                    darwin_kqueue_spinlock(delay.as_millis() as u64).await;
                                }
                                crate::adapters::qilin_ddos_guard::DdosOutcome::Proceed(None) => {}
                                crate::adapters::qilin_ddos_guard::DdosOutcome::Quarantine(quarantine_duration) => {
                                    // Phase 76B: Don't sleep the worker — re-enqueue
                                    // with jittered unlock time and continue to next URL.
                                    quarantine_queue.push(RetryPayload {
                                        url: next_url.clone(),
                                        attempt: current_attempt,
                                        unlock_timestamp: std::time::Instant::now() + quarantine_duration,
                                    });
                                    // The status-based retry logic below will also fire,
                                    // but should_retry won't double-enqueue because we continue
                                    // before that point.
                                    continue;
                                }
                            }
                            // Update BBR RTT estimate on successful responses
                            if status.is_success() {
                                ddos.update_rtt(elapsed_ms);
                            }
                        }

                        effective_url = resp.url().as_str().to_string();

                        if status.is_success() {
                            // Phase 67: Offload UTF-8 conversion to spawn_blocking for large HTML pages
                            match resp.bytes().await {
                                Ok(body_bytes) => {
                                    match tokio::task::spawn_blocking(move || {
                                        String::from_utf8(body_bytes.to_vec())
                                            .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
                                    }).await {
                                        Ok(body) => {
                                            f.record_success(cid, 4096, elapsed_ms);
                                            if use_slow_path {
                                                crawl_governor.record_success_with_latency(cid, elapsed_ms);
                                                worker_req_count += 1;
                                                if worker_req_count % crawl_governor.current_repin_interval() == 0
                                                    && crawl_governor.should_repin(cid)
                                                {
                                                    if let Some((thompson_cid, _score)) = crawl_governor.thompson_sample_circuit() {
                                                        if thompson_cid != cid {
                                                            let my_count = crawl_governor.circuit_request_count[cid].load(Ordering::Relaxed);
                                                            let my_avg = if my_count > 0 {
                                                                crawl_governor.circuit_latency_sum_ms[cid].load(Ordering::Relaxed) / my_count
                                                            } else { 0 };
                                                            let ts_count = crawl_governor.circuit_request_count[thompson_cid].load(Ordering::Relaxed);
                                                            let ts_avg = if ts_count > 0 {
                                                                crawl_governor.circuit_latency_sum_ms[thompson_cid].load(Ordering::Relaxed) / ts_count
                                                            } else { 0 };
                                                            println!("[Qilin Worker {}] Thompson re-pin: c{}({}ms) → c{}({}ms)", worker_idx, cid, my_avg, thompson_cid, ts_avg);
                                                            worker_client = None;
                                                        }
                                                    }
                                                }
                                            }
                                            html = Some(body);
                                        }
                                        Err(_) => {
                                            f.record_failure(cid);
                                            if use_slow_path {
                                                crawl_governor.record_failure_for_circuit(CrawlFailureKind::Http, cid);
                                            }
                                            retry_failure_kind = CrawlFailureKind::Http;
                                            should_retry = true;
                                        }
                                    }
                                }
                                Err(_) => {
                                    f.record_failure(cid);
                                    if use_slow_path {
                                        crawl_governor.record_failure_for_circuit(CrawlFailureKind::Http, cid);
                                    }
                                    retry_failure_kind = CrawlFailureKind::Http;
                                    should_retry = true;
                                }
                            }
                        } else if status == 404 {
                            f.record_success(cid, 512, elapsed_ms);
                            if use_slow_path {
                                crawl_governor.record_success_with_latency(cid, elapsed_ms);
                            }
                        } else {
                            f.record_failure(cid);
                            let failure_kind = classify_http_status_failure(status);
                            if matches!(failure_kind, CrawlFailureKind::Throttle) {
                                record_late_throttle_if_durable(
                                    crawl_governor.telemetry.as_ref(),
                                    durable_root_confirmed.as_ref(),
                                );
                                f.trigger_circuit_isolation(cid).await;
                                if let Some(telemetry) = &crawl_governor.telemetry {
                                    telemetry.record_failover(format!("circuit_{}", cid));
                                }
                            }
                            if use_slow_path || matches!(failure_kind, CrawlFailureKind::Throttle)
                            {
                                crawl_governor.record_failure_for_circuit(failure_kind, cid);
                            }
                            if use_slow_path
                                && matches!(failure_kind, CrawlFailureKind::Throttle)
                            {
                                tokio::time::sleep(Duration::from_secs(2)).await;
                            }
                            retry_failure_kind = failure_kind;
                            should_retry = true;
                            if next_url != active_seed_url {
                                emit_limited_child_log(
                                    &ui_app_clone,
                                    &child_failure_logged,
                                    "Failure",
                                    format!(
                                        "cid={} url={} resolved={} status={} attempt={} kind={}",
                                        cid,
                                        next_url,
                                        effective_url,
                                        status,
                                        current_attempt,
                                        failure_kind_label(failure_kind)
                                    ),
                                );
                            }
                        }
                    } else {
                        f.record_failure(cid);
                        if use_slow_path {
                            let is_collapsed = if resp_result.is_err() {
                                true
                            } else if let Ok(Err(ref req_err)) = resp_result {
                                let err_str = req_err.to_string().to_lowercase();
                                err_str.contains("timeout")
                                    || err_str.contains("connection reset")
                                    || err_str.contains("broken pipe")
                                    || err_str.contains("eos")
                                    || err_str.contains("eof")
                            } else {
                                false
                            };

                            if is_collapsed {
                                let failure_kind = match &resp_result {
                                    Ok(Err(req_err)) => classify_request_error(req_err),
                                    Err(_) => CrawlFailureKind::Timeout,
                                    _ => CrawlFailureKind::Circuit,
                                };
                                crawl_governor.record_failure_for_circuit(failure_kind, cid);
                                retry_failure_kind = failure_kind;
                                f.trigger_circuit_isolation(cid).await;
                                if let Some(telemetry) = &crawl_governor.telemetry {
                                    telemetry.record_failover(format!("circuit_{}", cid));
                                }
                            } else {
                                retry_failure_kind = match &resp_result {
                                    Ok(Err(req_err)) => classify_request_error(req_err),
                                    Err(_) => CrawlFailureKind::Timeout,
                                    _ => CrawlFailureKind::Http,
                                };
                                crawl_governor.record_failure_for_circuit(retry_failure_kind, cid);
                            }
                        } else {
                            retry_failure_kind = match &resp_result {
                                Ok(Err(req_err)) => classify_request_error(req_err),
                                Err(_) => CrawlFailureKind::Timeout,
                                _ => CrawlFailureKind::Http,
                            };
                        }
                        should_retry = true;
                        if next_url != active_seed_url {
                            emit_limited_child_log(
                                &ui_app_clone,
                                &child_failure_logged,
                                "Failure",
                                format!(
                                    "cid={} url={} attempt={} kind={} error={}",
                                    cid,
                                    next_url,
                                    current_attempt,
                                    failure_kind_label(retry_failure_kind),
                                    match &resp_result {
                                        Ok(Err(req_err)) => req_err.to_string(),
                                        Err(_) => "timeout".to_string(),
                                        _ => "unknown".to_string(),
                                    }
                                ),
                            );
                        }
                    }

                    if should_retry {
                        if subtree_shaping_enabled && use_slow_path {
                            if let Some(subtree_key) =
                                SubtreeHeatmap::subtree_key(&active_seed_url, &next_url)
                            {
                                heatmap.lock().await.record_failure(
                                    &subtree_key,
                                    heat_failure_kind(retry_failure_kind),
                                );
                            }
                        }
                        worker_client = None;
                        if current_attempt < 15 {
                            if use_slow_path {
                                let retry_lane =
                                    retry_lane_for_failure(retry_failure_kind, current_attempt);
                                let backoff = retry_backoff(current_attempt, retry_lane);

                                let retry_url = route_plan
                                    .retry_url_for_failure(
                                        &next_url,
                                        retry_failure_kind,
                                        current_attempt,
                                        Some(&ui_app_clone),
                                    )
                                    .await
                                    .unwrap_or_else(|| next_url.clone());

                                let retry_payload = RetryPayload {
                                    url: retry_url,
                                    attempt: current_attempt + 1,
                                    unlock_timestamp: std::time::Instant::now() + backoff,
                                };
                                match retry_lane {
                                    RetryLane::Primary => retry_q_clone.push(retry_payload),
                                    RetryLane::Degraded => {
                                        emit_limited_child_log(
                                            &ui_app_clone,
                                            &child_retry_lane_logged,
                                            "RetryLane",
                                            format!(
                                                "enqueue lane={} url={} next_attempt={} kind={} backoff={}s",
                                                retry_lane_label(retry_lane),
                                                retry_payload.url,
                                                retry_payload.attempt,
                                                failure_kind_label(retry_failure_kind),
                                                backoff.as_secs()
                                            ),
                                        );
                                        degraded_retry_q_clone.push(retry_payload);
                                    }
                                }
                            } else {
                                let retry_payload = RetryPayload {
                                    url: next_url.clone(),
                                    attempt: current_attempt + 1,
                                    unlock_timestamp: std::time::Instant::now()
                                        + Duration::from_secs((current_attempt as u64).clamp(1, 5)),
                                };
                                retry_q_clone.push(retry_payload);
                            }
                            increment_qilin_pending(
                                f.as_ref(),
                                telemetry_clone.as_ref(),
                                pending_clone.as_ref(),
                                max_concurrent,
                            );
                            tokio::task::yield_now().await;
                        } else {
                            use std::io::Write;
                            if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open("failed_nodes.log") {
                                let _ = writeln!(file, "FAILED_NODE: {}", next_url);
                            }
                            eprintln!("[Qilin] Dropping node after 15 retries: {}", next_url);
                            let _ = ui_app_clone.emit("crawl_error", next_url.clone());
                        }
                        continue;
                    }

                    let Some(html) = html else { continue; };

                    // Phase 77: Military-grade CMS JSON Data Bypass in Worker Loop
                    // If the response is JSON, we parse it directly and bypass HTML scraping.
                    if html.trim().starts_with('[') || html.trim().starts_with('{') {
                        let base_domain = if let Ok(parsed) = reqwest::Url::parse(&effective_url) {
                            format!("{}://{}", parsed.scheme(), parsed.host_str().unwrap_or(""))
                        } else {
                            effective_url.split("/site/").next().unwrap_or("").to_string()
                        };
                        let json_entries = parse_qilin_json(&html, &base_domain);

                        emit_limited_child_log(
                            &ui_app_clone,
                            &child_parse_logged,
                            "JSON-Parse",
                            format!(
                                "discovered {} entries via JSON bypass from {}",
                                json_entries.len(),
                                effective_url
                            ),
                        );

                        for entry in json_entries {
                            if entry.entry_type == EntryType::Folder && f.mark_visited(&entry.raw_url) {
                                increment_qilin_pending(
                                    f.as_ref(),
                                    telemetry_clone.as_ref(),
                                    pending_clone.as_ref(),
                                    max_concurrent,
                                );
                                q_clone.push(entry.raw_url.clone());
                            }
                            let _ = ui_tx_clone.send(entry.clone()).await;
                        }
                        continue;
                    }

                    // Phase 44: Mark folder successfully visited
                    {
                        let mut vf = vf_clone.lock().await;
                        vf.insert(next_url.clone());
                        if effective_url != next_url {
                            vf.insert(effective_url.clone());
                        }
                    }
                    if subtree_shaping_enabled && use_slow_path {
                        if let Some(subtree_key) =
                            SubtreeHeatmap::subtree_key(&active_seed_url, &effective_url)
                        {
                            heatmap.lock().await.record_success(&subtree_key);
                        }
                    }
                    route_plan.record_request_success(&effective_url);

                    if !f.active_options.listing {
                        continue;
                    }

                    let mut new_files = Vec::new();

                    // Extract the relative directory path from the base seed URL
                    let mut nested_path = String::new();
                    if next_url == active_seed_url {
                        emit_root_listing_diagnostics(&ui_app_clone, &effective_url, &html);
                    }
                    if effective_url.starts_with(&active_seed_url) {
                        let relative = &effective_url[active_seed_url.len()..];
                        if !relative.is_empty() {
                            nested_path = path_utils::url_decode(relative);
                            if !nested_path.starts_with('/') {
                                nested_path.insert(0, '/');
                            }
                            if !nested_path.ends_with('/') {
                                nested_path.push('/');
                            }
                        }
                    }
                    if nested_path.is_empty() {
                        nested_path = "/".to_string();
                    }

                    // Offload CPU-heavy Regex and string loops so we don't stall the executor
                    let (spawned_files, spawned_folders) = tokio::task::spawn_blocking({
                        let html = html.clone();
                        let effective_url = effective_url.clone();
                        let nested_path = nested_path.clone();
                        let domain_clone = domain_clone.clone();
                        move || {
                            let mut local_files = Vec::new();
                            let mut local_folders = Vec::new();

                            // Check if it's the old <table id="list"> Qilin or the new V3 HTML structure
                            if html.contains("<table id=\"list\">") || html.contains("Data browser") {
                                let mut found_any = false;

                                // PHASE 78: Zero-Copy SIMD HTML Data Extraction (QData V3)
                                let mut cursor = 0;
                                let needle = "<td class=\"link\"><a href=\"";
                                while let Some(start_idx) = html[cursor..].find(needle) {
                                    found_any = true;
                                    let href_start = cursor + start_idx + needle.len();
                                    if let Some(href_end_offset) = html[href_start..].find('"') {
                                        let href_str = &html[href_start..href_start + href_end_offset];
                                        let search_resume = href_start + href_end_offset;
                                        let size_needle = "</td><td class=\"size\">";
                                        if let Some(size_start_offset) = html[search_resume..].find(size_needle) {
                                            let size_start = search_resume + size_start_offset + size_needle.len();
                                            if let Some(size_end_offset) = html[size_start..].find("</td>") {
                                                let size_str_raw = &html[size_start..size_start + size_end_offset];
                                                cursor = size_start + size_end_offset;

                                                if href_str == "../" || href_str == "/" || href_str.starts_with('?') {
                                                    continue;
                                                }

                                                let is_dir = href_str.ends_with('/');
                                                let clean_name = listing_entry_name(href_str);
                                                let child_url = resolve_listing_child_url(&effective_url, href_str, is_dir);
                                                let sanitized_name = path_utils::sanitize_path(&clean_name);
                                                let full_path = format!("{}{}", nested_path, sanitized_name);

                                                if is_dir {
                                                    local_files.push(FileEntry { jwt_exp: None,
                                                        path: full_path,
                                                        size_bytes: None,
                                                        entry_type: EntryType::Folder,
                                                        raw_url: child_url.clone(),
                                                    });
                                                    local_folders.push(child_url);
                                                } else {
                                                    let raw_size = size_str_raw.trim();
                                                    let size_bytes = if raw_size == "-" { None } else { path_utils::parse_size(raw_size) };
                                                    local_files.push(FileEntry { jwt_exp: None,
                                                        path: full_path,
                                                        size_bytes,
                                                        entry_type: EntryType::File,
                                                        raw_url: child_url,
                                                    });
                                                }
                                                continue;
                                            }
                                        }
                                        cursor = search_resume;
                                    } else {
                                        break;
                                    }
                                }

                                // Fallback legacy index
                                if !found_any {
                                   let parsed = crate::adapters::autoindex::parse_autoindex_html(&html);
                                   for (filename, parsed_size, is_dir) in parsed {
                                       let child_url = resolve_listing_child_url(&effective_url, &filename, is_dir);

                                       if is_dir {
                                           let sanitized_name = path_utils::sanitize_path(&filename);
                                           let full_path = format!("{}{}", nested_path, sanitized_name);
                                           local_files.push(FileEntry { jwt_exp: None,
                                               path: full_path,
                                               size_bytes: None,
                                               entry_type: EntryType::Folder,
                                               raw_url: child_url.clone(),
                                           });
                                           local_folders.push(child_url);
                                       } else {
                                           let sanitized_name = path_utils::sanitize_path(&filename);
                                           let full_path = format!("{}{}", nested_path, sanitized_name);
                                           local_files.push(FileEntry { jwt_exp: None,
                                               path: full_path,
                                               size_bytes: parsed_size,
                                               entry_type: EntryType::File,
                                               raw_url: child_url,
                                           });
                                       }
                                   }
                                }
                            }

                            // Phase 78: Zero-Copy SIMD CMS Blog Parsing Layout Scan
                            let mut cursor = 0;
                            let href_needle = "href=\"";
                            while let Some(href_idx) = html[cursor..].find(href_needle) {
                                let start = cursor + href_idx + href_needle.len();
                                if let Some(end_offset) = html[start..].find('"') {
                                    let raw_href = &html[start..start + end_offset];
                                    if raw_href.starts_with("/uploads/") {
                                        let file_url = format!("{}{}", domain_clone, raw_href);
                                        let file_path = path_utils::sanitize_path(raw_href);
                                        local_files.push(FileEntry { jwt_exp: None,
                                            path: format!("/{}", file_path),
                                            size_bytes: None,
                                            entry_type: EntryType::File,
                                            raw_url: file_url,
                                        });
                                    } else if raw_href.starts_with("/site/view") || raw_href.starts_with("/page/") {
                                        let page_url = format!("{}{}", domain_clone, raw_href);
                                        local_folders.push(page_url);
                                    }
                                    cursor = start + end_offset;
                                } else {
                                    break;
                                }
                            }

                            if local_files.is_empty() && local_folders.is_empty() {
                                for watch_target in extract_watch_data_targets(&domain_clone, &html) {
                                    local_folders.push(watch_target);
                                }
                            }

                            (local_files, local_folders)
                        }
                    }).await.unwrap_or_default();
                    if next_url == active_seed_url
                        && !root_parse_logged.swap(true, std::sync::atomic::Ordering::AcqRel)
                    {
                        println!(
                            "[Qilin Root Parse] files={} folders={} next_url={}",
                            spawned_files.len(),
                            spawned_folders.len(),
                            next_url
                        );
                    } else if next_url != active_seed_url {
                        emit_limited_child_log(
                            &ui_app_clone,
                            &child_parse_logged,
                            "Parse",
                            format!(
                                "requested={} resolved={} files={} folders={} html_bytes={}",
                                next_url,
                                effective_url,
                                spawned_files.len(),
                                spawned_folders.len(),
                                html.len()
                            ),
                        );
                    }

                    if next_url == active_seed_url {
                        if let Some(node) = confirm_qilin_root_winner(
                            durable_node_cache.as_ref(),
                            durable_uuid.as_deref(),
                            &effective_url,
                            &html,
                            response_latency_ms.unwrap_or_default(),
                            telemetry_clone.as_ref(),
                            &ui_app_clone,
                        )
                        .await
                        {
                            durable_root_confirmed.store(true, Ordering::Relaxed);
                            crawl_governor
                                .set_repin_interval_hint(node.recommended_repin_interval());
                            *durable_winner_url.lock().await = Some(node.url.clone());
                        }
                    }

                    let spawned_entry_count = spawned_files.len() + spawned_folders.len();
                    new_files.extend(spawned_files);
                    // Phase 67E: Increment discovered entries counter
                    discovered_clone.fetch_add(
                        spawned_entry_count,
                        std::sync::atomic::Ordering::Relaxed,
                    );

                            {
                                let mut df = df_clone.lock().await;
                                for sub_url in &spawned_folders {
                                    df.insert(sub_url.clone());
                                }
                            }

                    for sub_url in spawned_folders {
                        if f.mark_visited(&sub_url) {
                            increment_qilin_pending(
                                f.as_ref(),
                                telemetry_clone.as_ref(),
                                pending_clone.as_ref(),
                                max_concurrent,
                            );
                            let heatmap_key =
                                SubtreeHeatmap::subtree_key(&active_seed_url, &sub_url);
                            let route_to_degraded = subtree_shaping_enabled
                                && if let Some(ref key) = heatmap_key {
                                    heatmap.lock().await.should_route_to_degraded(key)
                                } else {
                                    false
                                };
                            emit_limited_child_log(
                                &ui_app_clone,
                                &child_queue_logged,
                                "Queue",
                                format!(
                                    "parent={} queued={} pending={} lane={}",
                                    effective_url,
                                    sub_url,
                                    pending_clone.load(std::sync::atomic::Ordering::SeqCst),
                                    if route_to_degraded { "degraded" } else { "primary" }
                                ),
                            );
                            if route_to_degraded {
                                degraded_retry_q_clone.push(RetryPayload {
                                    url: sub_url,
                                    attempt: 1,
                                    unlock_timestamp: std::time::Instant::now(),
                                });
                            } else {
                                q_clone.push(sub_url);
                            }
                        }
                    }

                    if !new_files.is_empty() && f.active_options.listing {
                        for entry in &new_files {
                            let _ = ui_tx_clone.send(entry.clone()).await;
                        }
                    }
                }
            });
        }

        // Phase 44: Absolute Directory Verification Engine
        let mut shutdown_verified = false;
        let mut reconciliation_rounds = 0u32;
        let mut stagnant_reconciliation_rounds = 0u32;
        let mut last_missing_count = None;
        let reconciliation_started_at = std::time::Instant::now();
        let reconciliation_wall_clock_budget = Duration::from_secs(
            env_usize("CRAWLI_QILIN_RECONCILIATION_BUDGET_SECS")
                .unwrap_or(120)
                .clamp(30, 300) as u64,
        );
        let mut reconciliation_attempts = HashMap::<String, u8>::new();

        while !shutdown_verified {
            while let Some(res) = workers.join_next().await {
                if let Err(e) = res {
                    eprintln!("[Qilin] worker panicked: {}", e);
                }
            }

            // Perform Reconciliation
            let discovered = discovered_folders.lock().await.clone();
            let visited = visited_folders.lock().await.clone();

            let missing_folders: std::collections::HashSet<_> =
                discovered.difference(&visited).cloned().collect();

            if missing_folders.is_empty() {
                println!(
                    "[Qilin Phase 44] 100% Folder verification achieved! ({} total folders parsed)",
                    visited.len()
                );
                let _ = app.emit(
                    "log",
                    format!(
                        "[Qilin] ✨ 100% Folder verification achieved ({} folders)",
                        visited.len()
                    ),
                );
                shutdown_verified = true;
            } else {
                let missing_count = missing_folders.len();
                reconciliation_rounds = reconciliation_rounds.saturating_add(1);
                if last_missing_count == Some(missing_count) {
                    stagnant_reconciliation_rounds =
                        stagnant_reconciliation_rounds.saturating_add(1);
                } else {
                    stagnant_reconciliation_rounds = 0;
                    last_missing_count = Some(missing_count);
                }

                if reconciliation_rounds >= 6
                    || stagnant_reconciliation_rounds >= 3
                    || reconciliation_started_at.elapsed() >= reconciliation_wall_clock_budget
                {
                    println!(
                        "[Qilin Phase 44] Reconciliation stalled at {} missing folders after {} rounds (elapsed={}s). Returning partial crawl instead of re-queueing forever.",
                        missing_count,
                        reconciliation_rounds,
                        reconciliation_started_at.elapsed().as_secs()
                    );
                    let _ = app.emit(
                        "log",
                        format!(
                            "[Qilin] Reconciliation stalled at {} missing folders after {} rounds (elapsed={}s). Returning partial results.",
                            missing_count,
                            reconciliation_rounds,
                            reconciliation_started_at.elapsed().as_secs()
                        ),
                    );
                    shutdown_verified = true;
                    continue;
                }

                println!(
                    "[Qilin Phase 44] Reconciliation failed: {} folders remain missing. Rotating circuits and re-queueing them now...",
                    missing_count
                );
                let _ = app.emit(
                    "log",
                    format!(
                        "[Qilin] ⚠ Reconciliation round {}: re-queueing {} dropped folders after rotating circuits.",
                        reconciliation_rounds,
                        missing_count
                    ),
                );

                let current_ports = crate::tor::detect_active_managed_tor_ports();
                for port in current_ports {
                    let _ = crate::tor::request_newnym(port).await;
                }
                tokio::time::sleep(Duration::from_millis(800)).await; // Phase 67: Reduced NEWNYM cooldown

                // Re-inject the failed folders into the primary queue and revive the workers
                for folder in missing_folders {
                    increment_qilin_pending(
                        frontier.as_ref(),
                        telemetry.as_ref(),
                        pending.as_ref(),
                        max_concurrent,
                    );
                    let attempt = escalate_reconciliation_attempt(
                        reconciliation_attempts.get(&folder).copied(),
                    );
                    reconciliation_attempts.insert(folder.clone(), attempt);
                    degraded_retry_queue.push(RetryPayload {
                        url: folder,
                        attempt,
                        unlock_timestamp: std::time::Instant::now(),
                    });
                }

                if is_dumb_mode {
                    let _ = app.emit(
                        "log",
                        "[Qilin] DUMB_MODE active: skipping Tail-End Sweep intelligence loop."
                            .to_string(),
                    );
                    shutdown_verified = true;
                    continue;
                }

                // Re-spawn the workers for the Tail-End Sweep
                for worker_idx in 0..max_concurrent {
                    let f = frontier.clone();
                    let q_clone = queue.clone();
                    let retry_q_clone = retry_queue.clone();
                    let degraded_retry_q_clone = degraded_retry_queue.clone();
                    let ui_tx_clone = ui_tx.clone();
                    let ui_app_clone = app.clone();
                    let pending_clone = pending.clone();
                    let discovered_clone = discovered_entries.clone();
                    let domain_clone = base_domain.clone();
                    let crawl_governor = governor.clone();
                    let route_plan = route_plan.clone();
                    let root_fetch_logged = root_fetch_logged.clone();
                    let root_parse_logged = root_parse_logged.clone();
                    let child_queue_logged = child_queue_logged.clone();
                    let child_fetch_logged = child_fetch_logged.clone();
                    let child_parse_logged = child_parse_logged.clone();
                    let child_failure_logged = child_failure_logged.clone();
                    let child_retry_lane_logged = child_retry_lane_logged.clone();
                    let degraded_in_flight = degraded_in_flight.clone();
                    let degraded_dispatch_counter = degraded_dispatch_counter.clone();
                    let heatmap = heatmap.clone();
                    let telemetry_clone = telemetry.clone();
                    let durable_uuid = durable_uuid.clone();
                    let durable_node_cache = durable_node_cache.clone();
                    let durable_root_confirmed = durable_root_confirmed.clone();
                    let durable_winner_url = durable_winner_url.clone();

                    let df_clone = discovered_folders.clone();
                    let vf_clone = visited_folders.clone();

                    workers.spawn(async move {
                        let ramp_initial = std::env::var("CRAWLI_VANGUARD_INITIAL")
                            .ok()
                            .and_then(|v| v.parse::<usize>().ok())
                            .unwrap_or(1);
                        let ramp_interval_ms = std::env::var("CRAWLI_VANGUARD_RAMP_INTERVAL_MS")
                            .ok()
                            .and_then(|v| v.parse::<u64>().ok())
                            .unwrap_or(2500);

                        if f.stealth_ramp_active() && worker_idx >= ramp_initial {
                            let offset_ms = (worker_idx - ramp_initial + 1) as u64 * ramp_interval_ms;
                            let mut slept = 0;
                            while slept < offset_ms {
                                if f.is_cancelled() { return; }
                                let current_workers = f.active_workers();
                                if crawl_governor.should_halt_ramp(current_workers) {
                                    let _ = ui_app_clone.emit(
                                        "log",
                                        format!("[Vanguard] Halting Tail-Sweep induction at {}/{} due to server 503s.", worker_idx, max_concurrent)
                                    );
                                    return;
                                }
                                darwin_kqueue_spinlock(100).await;
                                slept += 100;
                            }
                            let current_workers = f.active_workers();
                            if crawl_governor.should_halt_ramp(current_workers) { return; }
                        }

                        let mut ddos = crate::adapters::qilin_ddos_guard::DdosGuard::new();
                        let mut idle_sleep_ms: u64 = 50;
                        let mut worker_client: Option<(usize, crate::arti_client::ArtiClient)> = None;
                        let mut worker_req_count: usize = 0; // Phase 67I
                        loop {
                            if f.is_cancelled() { break; }

                            let mut degraded_lane_permit = None;
                            let (mut next_url, current_attempt) = match q_clone.pop() {
                                Some(url) => {
                                    idle_sleep_ms = 50;
                                    (url, 1)
                                }
                                None => {
                                    let should_probe_degraded = degraded_dispatch_counter
                                        .fetch_add(1, Ordering::Relaxed)
                                        % degraded_lane_interval
                                        == 0;

                                    if should_probe_degraded {
                                        if let Some(permit) =
                                            try_acquire_degraded_lane(&degraded_in_flight, degraded_lane_limit)
                                        {
                                            if let Some(payload) = take_due_retry(&degraded_retry_q_clone) {
                                                degraded_lane_permit = Some(permit);
                                                idle_sleep_ms = 50;
                                                emit_limited_child_log(
                                                    &ui_app_clone,
                                                    &child_retry_lane_logged,
                                                    "RetryLane",
                                                    format!(
                                                        "dispatch lane={} url={} attempt={} pending={}",
                                                        retry_lane_label(RetryLane::Degraded),
                                                        payload.url,
                                                        payload.attempt,
                                                        pending_clone.load(Ordering::SeqCst)
                                                    ),
                                                );
                                                (payload.url, payload.attempt)
                                            } else {
                                                drop(permit);
                                                if let Some(payload) = take_due_retry(&retry_q_clone) {
                                                let current_seed = route_plan.current_seed_url_sync();
                                                if subtree_shaping_enabled {
                                                    if crate::subtree_heatmap::SubtreeHeatmap::is_subtree_penalized(&payload.url) && !payload.url.starts_with(&current_seed) {
                                                        degraded_retry_q_clone.push(payload);
                                                        continue;
                                                    }
                                                }
                                                idle_sleep_ms = 50;
                                                (payload.url, payload.attempt)
                                            } else {
                                                    if pending_clone.load(Ordering::SeqCst) == 0
                                                        && retry_q_clone.is_empty()
                                                        && degraded_retry_q_clone.is_empty()
                                                    {
                                                        break;
                                                    }
                                                    darwin_kqueue_spinlock(idle_sleep_ms).await;
                                                    idle_sleep_ms = std::cmp::min(idle_sleep_ms * 2, 150);
                                                    continue;
                                                }
                                            }
                                        } else if let Some(payload) = take_due_retry(&retry_q_clone) {
                                            let current_seed = route_plan.current_seed_url_sync();
                                            if subtree_shaping_enabled {
                                                if crate::subtree_heatmap::SubtreeHeatmap::is_subtree_penalized(&payload.url) && !payload.url.starts_with(&current_seed) {
                                                    degraded_retry_q_clone.push(payload);
                                                    continue;
                                                }
                                            }
                                            idle_sleep_ms = 50;
                                            (payload.url, payload.attempt)
                                        } else {
                                            if pending_clone.load(Ordering::SeqCst) == 0
                                                && retry_q_clone.is_empty()
                                                && degraded_retry_q_clone.is_empty()
                                            {
                                                break;
                                            }
                                            darwin_kqueue_spinlock(idle_sleep_ms).await;
                                            idle_sleep_ms = std::cmp::min(idle_sleep_ms * 2, 150);
                                            continue;
                                        }
                                    } else if let Some(payload) = take_due_retry(&retry_q_clone) {
                                        let current_seed = route_plan.current_seed_url_sync();
                                        if subtree_shaping_enabled {
                                            if crate::subtree_heatmap::SubtreeHeatmap::is_subtree_penalized(&payload.url) && !payload.url.starts_with(&current_seed) {
                                                degraded_retry_q_clone.push(payload);
                                                continue;
                                            }
                                        }
                                        idle_sleep_ms = 50;
                                        (payload.url, payload.attempt)
                                    } else if let Some(permit) =
                                        try_acquire_degraded_lane(&degraded_in_flight, degraded_lane_limit)
                                    {
                                        if let Some(payload) = take_due_retry(&degraded_retry_q_clone) {
                                            degraded_lane_permit = Some(permit);
                                            idle_sleep_ms = 50;
                                            emit_limited_child_log(
                                                &ui_app_clone,
                                                &child_retry_lane_logged,
                                                "RetryLane",
                                                format!(
                                                    "dispatch lane={} url={} attempt={} pending={}",
                                                    retry_lane_label(RetryLane::Degraded),
                                                    payload.url,
                                                    payload.attempt,
                                                    pending_clone.load(Ordering::SeqCst)
                                                ),
                                            );
                                            (payload.url, payload.attempt)
                                        } else {
                                            drop(permit);
                                            if pending_clone.load(Ordering::SeqCst) == 0
                                                && retry_q_clone.is_empty()
                                                && degraded_retry_q_clone.is_empty()
                                            {
                                                break;
                                            }
                                            darwin_kqueue_spinlock(idle_sleep_ms).await;
                                            idle_sleep_ms = std::cmp::min(idle_sleep_ms * 2, 150);
                                            continue;
                                        }
                                    } else {
                                        if pending_clone.load(Ordering::SeqCst) == 0
                                            && retry_q_clone.is_empty()
                                            && degraded_retry_q_clone.is_empty()
                                        {
                                            break;
                                        }
                                        darwin_kqueue_spinlock(idle_sleep_ms).await;
                                        idle_sleep_ms = std::cmp::min(idle_sleep_ms * 2, 150);
                                        continue;
                                    }
                                }
                            };
                            let active_seed = route_plan.current_seed_url_sync();
                            let is_root_retry = is_root_retry_url(&active_seed, &next_url);
                            let keep_active_seed = should_keep_child_retry_on_active_seed(
                                &active_seed,
                                &next_url,
                                current_attempt,
                            );
                            let pinned_seed = route_plan
                                .preferred_seed_for_request(worker_idx, &next_url, current_attempt)
                                .unwrap_or_else(|| route_plan.worker_node_url(worker_idx));
                            if !is_root_retry && !keep_active_seed && !pinned_seed.is_empty() {
                                if let Some((current_seed, _)) =
                                    split_qilin_seed_and_relative_path(&next_url)
                                {
                                    if pinned_seed != current_seed {
                                        next_url = remap_seed_url(
                                            &next_url,
                                            &current_seed,
                                            &pinned_seed,
                                        );
                                    }
                                }
                            }
                            let _active_request_guard = QilinActiveRequestGuard::new(
                                f.clone(),
                                telemetry_clone.clone(),
                                max_concurrent,
                            );
                            let _degraded_lane_permit = degraded_lane_permit;

                            let _crawl_slot = crawl_governor.acquire_slot().await;
                            let _permit = f.politeness_semaphore.acquire().await.ok();
                            let (cid, client) = if let Some((cid, client)) = &worker_client {
                                (*cid, client.clone())
                            } else {
                                let (cid, client) = f.get_client();
                                worker_client = Some((cid, client.clone()));
                                (cid, client)
                            };
                            let delay = f.scorer.yield_delay(cid);
                            if delay > std::time::Duration::ZERO {
                                darwin_kqueue_spinlock(delay.as_millis() as u64).await;
                            }

                            let _guard = QilinPendingGuard {
                                frontier: f.clone(),
                                telemetry: telemetry_clone.clone(),
                                pending: pending_clone.clone(),
                                worker_target: max_concurrent,
                            };
                            let active_seed_url = route_plan.current_seed_url().await;
                            route_plan.record_request_route(&active_seed_url, &next_url);
                            if next_url == active_seed_url
                                && !root_fetch_logged.swap(true, std::sync::atomic::Ordering::AcqRel)
                            {
                                println!(
                                    "[Qilin Root Fetch] cid={} url={} attempt={} pending={}",
                                    cid,
                                    next_url,
                                    current_attempt,
                                    pending_clone.load(std::sync::atomic::Ordering::SeqCst)
                                );
                            } else if next_url != active_seed_url {
                                emit_limited_child_log(
                                    &ui_app_clone,
                                    &child_fetch_logged,
                                    "Fetch",
                                    format!(
                                        "cid={} url={} attempt={} pending={}",
                                        cid,
                                        next_url,
                                        current_attempt,
                                        pending_clone.load(std::sync::atomic::Ordering::SeqCst)
                                    ),
                                );
                            }

                            let start_time = std::time::Instant::now();
                            let resp_result = tokio::time::timeout(
                                std::time::Duration::from_secs(crawl_governor.adaptive_timeout_secs()),
                                client.get(&next_url).send()
                            ).await;

                            let mut html = None;
                            let mut effective_url = next_url.clone();
                            let mut should_retry = false;
                            let mut retry_failure_kind = CrawlFailureKind::Http;
                            let mut response_latency_ms = None;

                            if let Ok(Ok(resp)) = resp_result {
                                let elapsed_ms = start_time.elapsed().as_millis() as u64;
                                response_latency_ms = Some(elapsed_ms);
                                let status = resp.status();

                                // Phase 76: EKF+BBR DDoS guard with quarantine
                                match ddos.record_response(status.as_u16()) {
                                    crate::adapters::qilin_ddos_guard::DdosOutcome::Proceed(Some(delay)) => {
                                        darwin_kqueue_spinlock(delay.as_millis() as u64).await;
                                    }
                                    crate::adapters::qilin_ddos_guard::DdosOutcome::Proceed(None) => {}
                                    crate::adapters::qilin_ddos_guard::DdosOutcome::Quarantine(_) => {}
                                }
                                if status.is_success() {
                                    ddos.update_rtt(elapsed_ms);
                                }

                                effective_url = resp.url().as_str().to_string();

                                if status.is_success() {
                                    if let Ok(body) = resp.text().await {
                                        f.record_success(cid, 4096, elapsed_ms);
                                        crawl_governor.record_success_with_latency(cid, elapsed_ms);
                                        // Phase 74: Thompson Sampling in secondary loop
                                        worker_req_count += 1;
                                        if worker_req_count % crawl_governor.current_repin_interval() == 0
                                            && crawl_governor.should_repin(cid)
                                        {
                                            if let Some((ts_cid, _)) = crawl_governor.thompson_sample_circuit() {
                                                if ts_cid != cid {
                                                    println!("[Qilin Worker] Thompson secondary re-pin c{} → c{}", cid, ts_cid);
                                                    worker_client = None;
                                                }
                                            }
                                        }
                                        html = Some(body);
                                    } else {
                                        f.record_failure(cid);
                                        crawl_governor.record_failure(CrawlFailureKind::Http);
                                        retry_failure_kind = CrawlFailureKind::Http;
                                        should_retry = true;
                                    }
                                } else if status == 404 {
                                    f.record_success(cid, 512, elapsed_ms);
                                    crawl_governor.record_success_with_latency(cid, elapsed_ms);
                                } else {
                                    f.record_failure(cid);
                                    let failure_kind = classify_http_status_failure(status);
                                    crawl_governor.record_failure(failure_kind);
                                    // Phase 67D: Per-worker cool-off after throttle
                                    if matches!(failure_kind, CrawlFailureKind::Throttle) {
                                        record_late_throttle_if_durable(
                                            crawl_governor.telemetry.as_ref(),
                                            durable_root_confirmed.as_ref(),
                                        );
                                        f.trigger_circuit_isolation(cid).await;
                                        tokio::time::sleep(Duration::from_secs(2)).await;
                                    }
                                    retry_failure_kind = failure_kind;
                                    should_retry = true;
                                    if next_url != active_seed_url {
                                        emit_limited_child_log(
                                            &ui_app_clone,
                                            &child_failure_logged,
                                            "Failure",
                                            format!(
                                                "cid={} url={} resolved={} status={} attempt={} kind={}",
                                                cid,
                                                next_url,
                                                effective_url,
                                                status,
                                                current_attempt,
                                                failure_kind_label(failure_kind)
                                            ),
                                        );
                                    }
                                }
                            } else {
                                f.record_failure(cid);
                                let failure_kind = match &resp_result {
                                    Ok(Err(req_err)) => classify_request_error(req_err),
                                    Err(_) => CrawlFailureKind::Timeout,
                                    _ => CrawlFailureKind::Http,
                                };
                                crawl_governor.record_failure(failure_kind);
                                retry_failure_kind = failure_kind;
                                if matches!(failure_kind, CrawlFailureKind::Circuit | CrawlFailureKind::Timeout) {
                                    f.trigger_circuit_isolation(cid).await;
                                }
                                should_retry = true;
                                if next_url != active_seed_url {
                                    emit_limited_child_log(
                                        &ui_app_clone,
                                        &child_failure_logged,
                                        "Failure",
                                        format!(
                                            "cid={} url={} attempt={} kind={} error={}",
                                            cid,
                                            next_url,
                                            current_attempt,
                                            failure_kind_label(retry_failure_kind),
                                            match &resp_result {
                                                Ok(Err(req_err)) => req_err.to_string(),
                                                Err(_) => "timeout".to_string(),
                                                _ => "unknown".to_string(),
                                            }
                                        ),
                                    );
                                }
                            }

                            if should_retry {
                                if subtree_shaping_enabled {
                                    if let Some(subtree_key) =
                                        SubtreeHeatmap::subtree_key(&active_seed_url, &next_url)
                                    {
                                        heatmap.lock().await.record_failure(
                                            &subtree_key,
                                            heat_failure_kind(retry_failure_kind),
                                        );
                                    }
                                }
                                worker_client = None;
                                if current_attempt < 15 {
                                    let retry_lane =
                                        retry_lane_for_failure(retry_failure_kind, current_attempt);
                                    let backoff = retry_backoff(current_attempt, retry_lane);

                                    let retry_url = route_plan
                                        .retry_url_for_failure(
                                            &next_url,
                                            retry_failure_kind,
                                            current_attempt,
                                            Some(&ui_app_clone),
                                        )
                                        .await
                                        .unwrap_or_else(|| next_url.to_string());

                                    let retry_payload = RetryPayload {
                                        url: retry_url,
                                        attempt: current_attempt + 1,
                                        unlock_timestamp: std::time::Instant::now() + backoff,
                                    };
                                    match retry_lane {
                                        RetryLane::Primary => retry_q_clone.push(retry_payload),
                                        RetryLane::Degraded => {
                                            emit_limited_child_log(
                                                &ui_app_clone,
                                                &child_retry_lane_logged,
                                                "RetryLane",
                                                format!(
                                                    "enqueue lane={} url={} next_attempt={} kind={} backoff={}s",
                                                    retry_lane_label(retry_lane),
                                                    retry_payload.url,
                                                    retry_payload.attempt,
                                                    failure_kind_label(retry_failure_kind),
                                                    backoff.as_secs()
                                                ),
                                            );
                                            degraded_retry_q_clone.push(retry_payload);
                                        }
                                    }
                                    increment_qilin_pending(
                                        f.as_ref(),
                                        telemetry_clone.as_ref(),
                                        pending_clone.as_ref(),
                                        max_concurrent,
                                    );
                                    tokio::task::yield_now().await;
                                } else {
                                    use std::io::Write;
                                    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open("failed_nodes.log") {
                                        let _ = writeln!(file, "FAILED_NODE: {}", next_url);
                                    }
                                    eprintln!("[Qilin] Dropping node after 15 retries: {}", next_url);
                                    let _ = ui_app_clone.emit("log", format!("[Qilin Queue] Re-enqueuing active_seed_url: {} -> {}", &active_seed_url, &next_url));
                                    increment_qilin_pending(
                                        f.as_ref(),
                                        telemetry_clone.as_ref(),
                                        pending_clone.as_ref(),
                                        max_concurrent,
                                    );
                                    q_clone.push(active_seed_url);
                                }
                                continue;
                            }

                            let Some(html) = html else { continue; };

                            {
                                let mut vf = vf_clone.lock().await;
                                vf.insert(next_url.to_string());
                                if effective_url != next_url {
                                    vf.insert(effective_url.clone());
                                }
                            }
                            if subtree_shaping_enabled {
                                if let Some(subtree_key) =
                                    SubtreeHeatmap::subtree_key(&active_seed_url, &effective_url)
                                {
                                    heatmap.lock().await.record_success(&subtree_key);
                                }
                            }
                            route_plan.record_request_success(&effective_url);

                            if !f.active_options.listing { continue; }

                            let mut new_files = Vec::new();
                            let mut nested_path = String::new();
                            if next_url == active_seed_url {
                                emit_root_listing_diagnostics(&ui_app_clone, &effective_url, &html);
                            }
                            if effective_url.starts_with(&active_seed_url) {
                                let relative = &effective_url[active_seed_url.len()..];
                                if !relative.is_empty() {
                                    nested_path = path_utils::url_decode(relative);
                                    if !nested_path.starts_with('/') { nested_path.insert(0, '/'); }
                                    if !nested_path.ends_with('/') { nested_path.push('/'); }
                                }
                            }
                            if nested_path.is_empty() { nested_path = "/".to_string(); }

                            let (spawned_files, spawned_folders) = tokio::task::spawn_blocking({
                                let html = html.clone();
                                let effective_url = effective_url.clone();
                                let nested_path = nested_path.clone();
                                let domain_clone = domain_clone.clone();
                                move || {
                                    let mut local_files = Vec::new();
                                    let mut local_folders = Vec::new();

                                    if html.contains("<table id=\"list\">") || html.contains("Data browser") {
                                        let mut found_any = false;
                                        static V3_ROW_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
                                            regex::Regex::new(r#"<td class="link"><a href="([^"]+)"[^>]*>.*?</a></td><td class="size">([^<]*)</td>"#).unwrap()
                                        });
                                        let v3_row_re = &*V3_ROW_RE;

                                        for cap in v3_row_re.captures_iter(&html) {
                                            found_any = true;
                                            if let (Some(href), Some(size_str)) = (cap.get(1), cap.get(2)) {
                                                let href_str = href.as_str();
                                                if href_str == "../" || href_str == "/" || href_str.starts_with("?") { continue; }

                                                let is_dir = href_str.ends_with('/');
                                                let clean_name = listing_entry_name(href_str);
                                                let child_url = resolve_listing_child_url(&effective_url, href_str, is_dir);

                                                if is_dir {
                                                    let sanitized_name = path_utils::sanitize_path(&clean_name);
                                                    let full_path = format!("{}{}", nested_path, sanitized_name);
                                                    local_files.push(FileEntry { jwt_exp: None,
                                                        path: full_path,
                                                        size_bytes: None,
                                                        entry_type: EntryType::Folder,
                                                        raw_url: child_url.clone(),
                                                    });
                                                    local_folders.push(child_url);
                                                } else {
                                                    let raw_size = size_str.as_str().trim();
                                                    let size_bytes = if raw_size == "-" { None } else { path_utils::parse_size(raw_size) };
                                                    let sanitized_name = path_utils::sanitize_path(&clean_name);
                                                    let full_path = format!("{}{}", nested_path, sanitized_name);
                                                    local_files.push(FileEntry { jwt_exp: None,
                                                        path: full_path,
                                                        size_bytes,
                                                        entry_type: EntryType::File,
                                                        raw_url: child_url,
                                                    });
                                                }
                                            }
                                        }

                                        if !found_any {
                                           let parsed = crate::adapters::autoindex::parse_autoindex_html(&html);
                                           for (filename, parsed_size, is_dir) in parsed {
                                               let child_url = resolve_listing_child_url(&effective_url, &filename, is_dir);

                                               if is_dir {
                                                   let sanitized_name = path_utils::sanitize_path(&filename);
                                                   let full_path = format!("{}{}", nested_path, sanitized_name);
                                                   local_files.push(FileEntry { jwt_exp: None,
                                                       path: full_path,
                                                       size_bytes: None,
                                                       entry_type: EntryType::Folder,
                                                       raw_url: child_url.clone(),
                                                   });
                                                   local_folders.push(child_url);
                                               } else {
                                                   let sanitized_name = path_utils::sanitize_path(&filename);
                                                   let full_path = format!("{}{}", nested_path, sanitized_name);
                                                   local_files.push(FileEntry { jwt_exp: None,
                                                       path: full_path,
                                                       size_bytes: parsed_size,
                                                       entry_type: EntryType::File,
                                                       raw_url: child_url,
                                                   });
                                               }
                                           }
                                        }
                                    }

                                    for line in html.lines() {
                                        if let Some(href_start) = line.find("href=\"") {
                                            let after_href = &line[href_start + 6..];
                                            if let Some(href_end) = after_href.find('"') {
                                                let raw_href = after_href[..href_end].to_string();

                                                if raw_href.starts_with("/uploads/") {
                                                    let file_url = format!("{}{}", domain_clone, raw_href);
                                                    let file_path = path_utils::sanitize_path(&raw_href);
                                                    local_files.push(FileEntry { jwt_exp: None,
                                                        path: format!("/{}", file_path),
                                                        size_bytes: None,
                                                        entry_type: EntryType::File,
                                                        raw_url: file_url,
                                                    });
                                                } else if raw_href.starts_with("/site/view") || raw_href.starts_with("/page/") {
                                                    let page_url = format!("{}{}", domain_clone, raw_href);
                                                    local_folders.push(page_url);
                                                }
                                            }
                                        }
                                    }

                                    if local_files.is_empty() && local_folders.is_empty() {
                                        for watch_target in
                                            extract_watch_data_targets(&domain_clone, &html)
                                        {
                                            local_folders.push(watch_target);
                                        }
                                    }

                                    (local_files, local_folders)
                                }
                            }).await.unwrap_or_default();
                            if next_url == active_seed_url
                                && !root_parse_logged.swap(true, std::sync::atomic::Ordering::AcqRel)
                            {
                                println!(
                                    "[Qilin Root Parse] files={} folders={} next_url={}",
                                    spawned_files.len(),
                                    spawned_folders.len(),
                                    next_url
                                );
                            } else if next_url != active_seed_url {
                                emit_limited_child_log(
                                    &ui_app_clone,
                                    &child_parse_logged,
                                    "Parse",
                                    format!(
                                        "requested={} resolved={} files={} folders={} html_bytes={}",
                                        next_url,
                                        effective_url,
                                        spawned_files.len(),
                                        spawned_folders.len(),
                                        html.len()
                                    ),
                                );
                            }

                            if next_url == active_seed_url {
                                if let Some(node) = confirm_qilin_root_winner(
                                    durable_node_cache.as_ref(),
                                    durable_uuid.as_deref(),
                                    &effective_url,
                                    &html,
                                    response_latency_ms.unwrap_or_default(),
                                    telemetry_clone.as_ref(),
                                    &ui_app_clone,
                                )
                                .await
                                {
                                    durable_root_confirmed.store(true, Ordering::Relaxed);
                                    crawl_governor
                                        .set_repin_interval_hint(node.recommended_repin_interval());
                                    *durable_winner_url.lock().await = Some(node.url.clone());
                                }
                            }

                            let spawned_entry_count = spawned_files.len() + spawned_folders.len();
                            new_files.extend(spawned_files);
                            // Phase 67E: Increment discovered entries counter
                            discovered_clone.fetch_add(
                                spawned_entry_count,
                                std::sync::atomic::Ordering::Relaxed,
                            );

                            {
                                let mut df = df_clone.lock().await;
                                for sub_url in &spawned_folders {
                                    df.insert(sub_url.clone());
                                }
                            }

                            for sub_url in spawned_folders {
                                if f.mark_visited(&sub_url) {
                                    increment_qilin_pending(
                                        f.as_ref(),
                                        telemetry_clone.as_ref(),
                                        pending_clone.as_ref(),
                                        max_concurrent,
                                    );
                                    let heatmap_key =
                                        SubtreeHeatmap::subtree_key(&active_seed_url, &sub_url);
                                    let route_to_degraded = subtree_shaping_enabled
                                        && if let Some(ref key) = heatmap_key {
                                            heatmap.lock().await.should_route_to_degraded(key)
                                        } else {
                                            false
                                        };
                                    emit_limited_child_log(
                                        &ui_app_clone,
                                        &child_queue_logged,
                                        "Queue",
                                        format!(
                                            "parent={} queued={} pending={} lane={}",
                                            effective_url,
                                            sub_url,
                                            pending_clone.load(std::sync::atomic::Ordering::SeqCst),
                                            if route_to_degraded { "degraded" } else { "primary" }
                                        ),
                                    );
                                    if route_to_degraded {
                                        degraded_retry_q_clone.push(RetryPayload {
                                            url: sub_url,
                                            attempt: 1,
                                            unlock_timestamp: std::time::Instant::now(),
                                        });
                                    } else {
                                        q_clone.push(sub_url);
                                    }
                                }
                            }

                            if !new_files.is_empty() && f.active_options.listing {
                                for entry in &new_files {
                                    let _ = ui_tx_clone.send(entry.clone()).await;
                                }
                            }
                        }
                    });
                }
            }
        }

        if subtree_heatmap_enabled {
            if let Some(path) = &heatmap_path {
                if let Err(err) = heatmap.lock().await.save(path) {
                    let _ = app.emit(
                        "log",
                        format!(
                            "[Qilin] Failed to persist subtree heatmap at {}: {}",
                            path.display(),
                            err
                        ),
                    );
                }
            }
            // Phase 75: Also persist to Sled VFS heatmap tree for crash-safe cross-session tracking
            if let Some(vfs_ref) = &vfs {
                let hm = heatmap.lock().await;
                let records: Vec<crate::subtree_heatmap::SubtreeHeatRecord> =
                    hm.entries.values().cloned().collect();
                if !records.is_empty() {
                    if let Err(err) = vfs_ref.upsert_heatmap_batch(&records).await {
                        println!("[Qilin] Failed to persist heatmap to Sled VFS: {}", err);
                    } else {
                        println!(
                            "[Qilin] Persisted {} heatmap records to Sled VFS tree",
                            records.len()
                        );
                    }
                }
            }
        }

        match route_plan.persist_subtree_preferences() {
            Ok(count) if count > 0 => {
                if let Some(path) = &subtree_summary_path {
                    let _ = app.emit(
                        "log",
                        format!(
                            "[Qilin] Persisted {} subtree host preferences to {}",
                            count,
                            path.display()
                        ),
                    );
                }
            }
            Ok(_) => {}
            Err(err) => {
                if let Some(path) = &subtree_summary_path {
                    let _ = app.emit(
                        "log",
                        format!(
                            "[Qilin] Failed to persist subtree host preferences at {}: {}",
                            path.display(),
                            err
                        ),
                    );
                }
            }
        }

        // Phase 76: Clear the global heatmap to prevent stale data in next session
        SubtreeHeatmap::uninstall_global_heatmap();

        drop(ui_tx);
        let _ = ui_flush_task.await;

        let total_discovered = discovered_entries.load(Ordering::Relaxed);
        let effective_entries = if let Some(vfs_ref) = &vfs {
            vfs_ref
                .summarize_entries()
                .await
                .map(|summary| summary.discovered_count)
                .unwrap_or(total_discovered)
        } else {
            total_discovered
        };
        if total_discovered == 0 {
            let msg = format!(
                "[Qilin] Crawl resolved zero files/folders for {}. Treating as unresolved target.",
                current_url
            );
            let _ = app.emit("log", msg.clone());
            anyhow::bail!(msg);
        }

        if durable_root_confirmed.load(Ordering::Relaxed) {
            if let (Some(node_cache), Some(uuid)) =
                (durable_node_cache.as_ref(), durable_uuid.as_deref())
            {
                let winner_url = durable_winner_url
                    .lock()
                    .await
                    .clone()
                    .unwrap_or_else(|| actual_seed_url.clone());
                if let Some(telemetry_ref) = telemetry.as_ref() {
                    let snapshot = telemetry_ref.snapshot_counters();
                    if let Some(node) = node_cache
                        .record_winner_run_outcome(
                            uuid,
                            &winner_url,
                            effective_entries,
                            crawl_started_at.elapsed().as_millis() as u64,
                            snapshot.node_failovers,
                            snapshot.late_throttles,
                        )
                        .await
                    {
                        telemetry_ref.set_winner_host(node.host.clone());
                    }
                }
            }
        }

        if let Some(telemetry_ref) = telemetry.as_ref() {
            let snapshot = telemetry_ref.snapshot_counters();
            let winner_host = snapshot
                .winner_host
                .as_deref()
                .map(shorten_tail_host)
                .unwrap_or_else(|| "-".to_string());
            let slowest_circuit = snapshot.slowest_circuit.as_deref().unwrap_or("-");
            let tail_summary = format!(
                "[Qilin Tail] winner_host={} slowest_circuit={} late_throttles={} outlier_isolations={}",
                winner_host,
                slowest_circuit,
                snapshot.late_throttles,
                snapshot.outlier_isolations
            );
            println!("{}", tail_summary);
            let _ = app.emit("log", tail_summary);
        }

        if collect_results_locally {
            let final_entries = collected_entries.lock().await.clone();
            Ok(final_entries)
        } else {
            Ok(Vec::new())
        }
    }

    fn name(&self) -> &'static str {
        "Qilin Nginx Autoindex / CMS"
    }

    fn known_domains(&self) -> Vec<&'static str> {
        vec![
            // CMS frontends
            "iv6lrjrd5ioyanvvemnkhturmyfpfbdcy442e22oqd2izkwnjw23m3id.onion",
            "ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion",
            "ef4p3qn56susyjy56vym4gawjzaoc52e52w545e7mu6qhbmfut5iwxqd.onion",
            "6esfx73oxphqeh2lpgporkw72uj2xqm5bbb6pfl24mt27hlll7jdswyd.onion",
            // Phase 42: Storage nodes (auto-detected via Stage A/B/D)
            "szgkpzhcrnshftjb5mtvd6bc5oep5yabmgfmwt7u3tiqzfikoew27hqd.onion",
            "7mnkv5nvnjyifezlfyba6gek7aeimg5eghej5vp65qxnb2hjbtlttlyd.onion",
            "25mjg55vcbjzwykz2uqsvaw7hcevm4pqxl42o324zr6qf5zgddmghkqd.onion",
            "arrfcpipltlfgxc6hvjylixc6c5hrummwctz4wqysk3h56ntqz5scnad.onion",
        ]
    }

    fn regex_marker(&self) -> Option<&'static str> {
        Some(
            r#"<div class="page-header-title">QData</div>|Data browser|_csrf-blog|item_box_photos|value="[a-z2-7]{56}\.onion""#,
        )
    }
}

/// Phase 77: Military-grade Qilin CMS JSON Parser.
/// Robustly parses a wide variety of Qilin JSON formats returned by the /site/data endpoint.
/// Bypasses storage nodes by converting JSON records directly into FileEntry objects.
fn parse_qilin_json(json: &str, base_domain: &str) -> Vec<FileEntry> {
    let mut entries = Vec::new();
    let value: serde_json::Value = match serde_json::from_str(json) {
        Ok(v) => v,
        Err(e) => {
            println!(
                "[Qilin JSON] ⚠ Critical: Failed to parse JSON payload: {}",
                e
            );
            return entries;
        }
    };

    // Qilin JSON data usually resides in a root array or a "data" / "items" / "files" object.
    let items = if let Some(arr) = value.as_array() {
        arr
    } else if let Some(arr) = value.get("data").and_then(|v| v.as_array()) {
        arr
    } else if let Some(arr) = value.get("items").and_then(|v| v.as_array()) {
        arr
    } else if let Some(arr) = value.get("files").and_then(|v| v.as_array()) {
        arr
    } else {
        println!("[Qilin JSON] ⚠ Warning: Could not find data array in JSON structure.");
        return entries;
    };

    for item in items {
        let name = item
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let size_str = item.get("size").and_then(|v| v.as_str()).unwrap_or("-");
        let item_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("file");
        let uuid = item
            .get("uuid")
            .or_else(|| item.get("id"))
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let is_dir = matches!(
            item_type.to_lowercase().as_str(),
            "dir" | "folder" | "directory"
        );

        // Construct the raw URL. If it's a file, it's a download. If it's a dir, it's a new site view.
        let raw_url = if is_dir {
            if !uuid.is_empty() {
                format!("{}/site/view?uuid={}", base_domain, uuid)
            } else {
                format!(
                    "{}/site/view?name={}",
                    base_domain,
                    urlencoding::encode(name)
                )
            }
        } else {
            // For files, we point to the /site/data endpoint or a direct download if provided.
            if let Some(dl_url) = item.get("download_url").and_then(|v| v.as_str()) {
                if dl_url.starts_with("http") {
                    dl_url.to_string()
                } else {
                    format!("{}{}", base_domain, dl_url)
                }
            } else if !uuid.is_empty() {
                format!("{}/site/data?uuid={}", base_domain, uuid)
            } else {
                format!(
                    "{}/site/data?name={}",
                    base_domain,
                    urlencoding::encode(name)
                )
            }
        };

        let size_bytes = if size_str == "-" {
            None
        } else {
            path_utils::parse_size(size_str)
        };
        let sanitized_path = format!("/{}", path_utils::sanitize_path(name));

        entries.push(FileEntry {
            jwt_exp: None,
            path: sanitized_path,
            size_bytes,
            entry_type: if is_dir {
                EntryType::Folder
            } else {
                EntryType::File
            },
            raw_url,
        });
    }

    entries
}

/// Phase 77: Parse the Qilin CMS `/site/view?uuid=X` HTML page for file entries.
/// The view page contains structured elements with file names, sizes, and links.
/// Typical Qilin view page HTML patterns:
///   - `<div class="item_box_photos">` containers with item details
///   - `<a href="/site/data?uuid=...">` download links
///   - `<a href="/site/view?uuid=...">` subfolder links
///   - `<td>filename</td>` in listing tables
///   - `data-name="..."` and `data-size="..."` attributes
fn parse_qilin_view_page(html: &str, base_domain: &str, _parent_uuid: &str) -> Vec<FileEntry> {
    let mut entries = Vec::new();
    let mut seen_urls = std::collections::HashSet::new();

    // Pattern 1: <a> tags with /site/view?uuid= (folders) or /site/data?uuid= (files)
    let link_re = regex::Regex::new(
        r#"<a\s[^>]*href="(/site/(view|data)\?uuid=([a-f0-9\-]{36}))"[^>]*>([^<]+)</a>"#,
    )
    .unwrap();

    for cap in link_re.captures_iter(html) {
        let path = cap.get(1).map(|m| m.as_str()).unwrap_or("");
        let link_type = cap.get(2).map(|m| m.as_str()).unwrap_or("");
        let uuid = cap.get(3).map(|m| m.as_str()).unwrap_or("");
        let name = cap.get(4).map(|m| m.as_str().trim()).unwrap_or("unknown");

        if name.is_empty() || name == "Watch data" || name == "Back" || uuid.is_empty() {
            continue;
        }

        let raw_url = format!("{}{}", base_domain, path);
        if !seen_urls.insert(raw_url.clone()) {
            continue;
        }

        let is_folder = link_type == "view";
        let sanitized = format!("/{}", path_utils::sanitize_path(name));

        entries.push(FileEntry {
            jwt_exp: None,
            path: sanitized,
            size_bytes: None,
            entry_type: if is_folder {
                EntryType::Folder
            } else {
                EntryType::File
            },
            raw_url,
        });
    }

    // Pattern 2: data-attributes on elements (some CMS versions use these)
    let data_re = regex::Regex::new(
        r#"data-name="([^"]+)"[^>]*data-uuid="([a-f0-9\-]{36})"(?:[^>]*data-type="([^"]*)")?"#,
    )
    .unwrap();

    for cap in data_re.captures_iter(html) {
        let name = cap.get(1).map(|m| m.as_str()).unwrap_or("unknown");
        let uuid = cap.get(2).map(|m| m.as_str()).unwrap_or("");
        let item_type = cap.get(3).map(|m| m.as_str()).unwrap_or("file");

        if uuid.is_empty() {
            continue;
        }

        let is_folder = matches!(
            item_type.to_lowercase().as_str(),
            "dir" | "folder" | "directory"
        );
        let raw_url = if is_folder {
            format!("{}/site/view?uuid={}", base_domain, uuid)
        } else {
            format!("{}/site/data?uuid={}", base_domain, uuid)
        };

        if !seen_urls.insert(raw_url.clone()) {
            continue;
        }

        let sanitized = format!("/{}", path_utils::sanitize_path(name));
        entries.push(FileEntry {
            jwt_exp: None,
            path: sanitized,
            size_bytes: None,
            entry_type: if is_folder {
                EntryType::Folder
            } else {
                EntryType::File
            },
            raw_url,
        });
    }

    // Pattern 3: item_box containers with structured content
    let item_box_re =
        regex::Regex::new(r#"(?s)<div[^>]*class="[^"]*item_box[^"]*"[^>]*>(.+?)</div>"#).unwrap();
    let inner_link_re =
        regex::Regex::new(r#"href="(/site/(view|data)\?uuid=([a-f0-9\-]{36}))"[^>]*>([^<]*)"#)
            .unwrap();

    for cap in item_box_re.captures_iter(html) {
        let inner = cap.get(1).map(|m| m.as_str()).unwrap_or("");
        for inner_cap in inner_link_re.captures_iter(inner) {
            let path = inner_cap.get(1).map(|m| m.as_str()).unwrap_or("");
            let link_type = inner_cap.get(2).map(|m| m.as_str()).unwrap_or("");
            let _uuid = inner_cap.get(3).map(|m| m.as_str()).unwrap_or("");
            let name = inner_cap
                .get(4)
                .map(|m| m.as_str().trim())
                .unwrap_or("unknown");

            if name.is_empty() || name == "Watch data" || name == "Back" {
                continue;
            }

            let raw_url = format!("{}{}", base_domain, path);
            if !seen_urls.insert(raw_url.clone()) {
                continue;
            }

            let is_folder = link_type == "view";
            let sanitized = format!("/{}", path_utils::sanitize_path(name));

            entries.push(FileEntry {
                jwt_exp: None,
                path: sanitized,
                size_bytes: None,
                entry_type: if is_folder {
                    EntryType::Folder
                } else {
                    EntryType::File
                },
                raw_url,
            });
        }
    }

    if !entries.is_empty() {
        println!(
            "[Qilin Phase 77] View page parser found {} entries ({} files, {} folders)",
            entries.len(),
            entries
                .iter()
                .filter(|e| e.entry_type == EntryType::File)
                .count(),
            entries
                .iter()
                .filter(|e| e.entry_type == EntryType::Folder)
                .count(),
        );
    }

    entries
}

#[cfg(test)]
mod tests {
    use super::{
        classify_qilin_ingress, extract_watch_data_targets, is_root_retry_url, now_unix_secs,
        parse_qilin_json, parse_qilin_view_page, remap_seed_url, retry_lane_for_failure,
        should_keep_child_retry_on_active_seed, split_qilin_seed_and_relative_path,
        standby_seed_urls, url_depth_relative, CrawlFailureKind, EntryType, QilinCrawlGovernor,
        QilinIngressKind, QilinRoutePlan, RetryLane, SubtreeHostHealth,
    };
    use crate::adapters::qilin_nodes::StorageNode;
    use crate::runtime_metrics::RuntimeTelemetry;
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

    fn node(url: &str, host: &str) -> StorageNode {
        StorageNode {
            url: url.to_string(),
            host: host.to_string(),
            last_seen: 0,
            avg_latency_ms: 0,
            hit_count: 0,
            success_count: 0,
            failure_count: 0,
            failure_streak: 0,
            cooldown_until: 0,
            ..StorageNode::default()
        }
    }

    #[test]
    fn standby_seed_urls_are_bounded_and_skip_primary() {
        let primary = "http://primary.onion/uuid/";
        let ranked = vec![
            node(primary, "primary.onion"),
            node("http://backup-a.onion/uuid/", "backup-a.onion"),
            node("http://backup-b.onion/uuid/", "backup-b.onion"),
            node("http://backup-c.onion/uuid/", "backup-c.onion"),
        ];

        let standby = standby_seed_urls(primary, &ranked, 2);
        assert_eq!(
            standby,
            vec![
                "http://backup-a.onion/uuid/".to_string(),
                "http://backup-b.onion/uuid/".to_string()
            ]
        );
    }

    #[test]
    fn remap_seed_url_preserves_relative_path() {
        let remapped = remap_seed_url(
            "http://primary.onion/uuid/folder/file.tar",
            "http://primary.onion/uuid/",
            "http://backup-a.onion/uuid/",
        );

        assert_eq!(remapped, "http://backup-a.onion/uuid/folder/file.tar");
    }

    #[test]
    fn root_retry_detection_skips_seed_remap() {
        let active_seed = "http://primary.onion/uuid/";
        assert!(is_root_retry_url(active_seed, active_seed));
        assert!(is_root_retry_url(active_seed, "http://primary.onion/uuid"));
        assert!(!is_root_retry_url(
            active_seed,
            "http://primary.onion/uuid/folder/"
        ));
    }

    #[test]
    fn first_child_retry_keeps_active_seed_affinity() {
        let active_seed = "http://primary.onion/uuid/";
        let child_url = "http://primary.onion/uuid/folder/";

        assert!(should_keep_child_retry_on_active_seed(
            active_seed,
            child_url,
            2
        ));
        assert!(!should_keep_child_retry_on_active_seed(
            active_seed,
            child_url,
            1
        ));
        assert!(!should_keep_child_retry_on_active_seed(
            active_seed,
            child_url,
            3
        ));
        assert!(!should_keep_child_retry_on_active_seed(
            active_seed,
            active_seed,
            2
        ));
    }

    #[test]
    fn split_qilin_seed_and_relative_path_extracts_subtree_key() {
        let (seed, relative) = split_qilin_seed_and_relative_path(
            "http://primary.onion/12345678-1234-1234-1234-123456789abc/kent/2016/",
        )
        .expect("seed should parse");

        assert_eq!(
            seed,
            "http://primary.onion/12345678-1234-1234-1234-123456789abc/"
        );
        assert_eq!(relative, "kent/2016");
    }

    #[test]
    fn subtree_success_promotes_preferred_seed() {
        let plan = QilinRoutePlan::new(
            "http://primary.onion/12345678-1234-1234-1234-123456789abc/".to_string(),
            vec![
                "http://backup-a.onion/12345678-1234-1234-1234-123456789abc/".to_string(),
                "http://backup-b.onion/12345678-1234-1234-1234-123456789abc/".to_string(),
            ],
            None,
            None,
        );

        plan.record_request_success(
            "http://backup-a.onion/12345678-1234-1234-1234-123456789abc/kent/2016/",
        );

        assert_eq!(
            plan.preferred_seed_for_request(
                0,
                "http://primary.onion/12345678-1234-1234-1234-123456789abc/kent/2016/",
                3,
            ),
            Some("http://backup-a.onion/12345678-1234-1234-1234-123456789abc/".to_string())
        );
    }

    #[test]
    fn quarantined_standby_is_skipped_for_same_subtree() {
        let plan = QilinRoutePlan::new(
            "http://primary.onion/12345678-1234-1234-1234-123456789abc/".to_string(),
            vec![
                "http://backup-a.onion/12345678-1234-1234-1234-123456789abc/".to_string(),
                "http://backup-b.onion/12345678-1234-1234-1234-123456789abc/".to_string(),
            ],
            None,
            None,
        );

        let now = now_unix_secs();
        let mut state_guard = plan.subtree_route_health.lock().unwrap();
        let subtree = state_guard.entry("kent/2016".to_string()).or_default();
        subtree.preferred_seed_url =
            Some("http://backup-a.onion/12345678-1234-1234-1234-123456789abc/".to_string());
        subtree.host_health.insert(
            "http://backup-a.onion/12345678-1234-1234-1234-123456789abc/".to_string(),
            SubtreeHostHealth {
                success_count: 3,
                failure_count: 1,
                consecutive_failures: 1,
                quarantine_until: now + 300,
                last_success_epoch: now,
                last_failure_epoch: now,
            },
        );
        drop(state_guard);

        assert_eq!(
            plan.preferred_seed_for_request(
                0,
                "http://primary.onion/12345678-1234-1234-1234-123456789abc/kent/2016/",
                3,
            ),
            Some("http://backup-b.onion/12345678-1234-1234-1234-123456789abc/".to_string())
        );
    }

    #[test]
    fn quarantined_preferred_seed_increments_quarantine_metric() {
        let telemetry = RuntimeTelemetry::default();
        telemetry.begin_crawl_session();
        let plan = QilinRoutePlan::new(
            "http://primary.onion/12345678-1234-1234-1234-123456789abc/".to_string(),
            vec![
                "http://backup-a.onion/12345678-1234-1234-1234-123456789abc/".to_string(),
                "http://backup-b.onion/12345678-1234-1234-1234-123456789abc/".to_string(),
            ],
            Some(telemetry.clone()),
            None,
        );

        let now = now_unix_secs();
        let mut state_guard = plan.subtree_route_health.lock().unwrap();
        let subtree = state_guard.entry("kent/2016".to_string()).or_default();
        subtree.preferred_seed_url =
            Some("http://backup-a.onion/12345678-1234-1234-1234-123456789abc/".to_string());
        subtree.host_health.insert(
            "http://backup-a.onion/12345678-1234-1234-1234-123456789abc/".to_string(),
            SubtreeHostHealth {
                success_count: 1,
                failure_count: 1,
                consecutive_failures: 1,
                quarantine_until: now + 300,
                last_success_epoch: now,
                last_failure_epoch: now,
            },
        );
        drop(state_guard);

        let _ = plan.preferred_seed_for_request(
            0,
            "http://primary.onion/12345678-1234-1234-1234-123456789abc/kent/2016/",
            3,
        );

        let snapshot = telemetry.snapshot_counters();
        assert_eq!(snapshot.subtree_quarantine_hits, 1);
    }

    #[test]
    fn off_winner_child_request_is_counted() {
        let telemetry = RuntimeTelemetry::default();
        telemetry.begin_crawl_session();
        let plan = QilinRoutePlan::new(
            "http://primary.onion/12345678-1234-1234-1234-123456789abc/".to_string(),
            vec!["http://backup-a.onion/12345678-1234-1234-1234-123456789abc/".to_string()],
            Some(telemetry.clone()),
            None,
        );

        plan.record_request_route(
            "http://primary.onion/12345678-1234-1234-1234-123456789abc/",
            "http://backup-a.onion/12345678-1234-1234-1234-123456789abc/kent/2016/",
        );
        plan.record_request_route(
            "http://primary.onion/12345678-1234-1234-1234-123456789abc/",
            "http://primary.onion/12345678-1234-1234-1234-123456789abc/",
        );

        let snapshot = telemetry.snapshot_counters();
        assert_eq!(snapshot.off_winner_child_requests, 1);
    }

    #[tokio::test]
    async fn degraded_route_harness_exercises_subtree_counters() {
        let telemetry = RuntimeTelemetry::default();
        telemetry.begin_crawl_session();

        let primary = "http://primary.onion/12345678-1234-1234-1234-123456789abc/".to_string();
        let backup_a = "http://backup-a.onion/12345678-1234-1234-1234-123456789abc/".to_string();
        let backup_b = "http://backup-b.onion/12345678-1234-1234-1234-123456789abc/".to_string();
        let subtree_path = "kent/2016/";
        let off_winner_child = format!("{backup_a}{subtree_path}");

        let plan = QilinRoutePlan::new(
            primary.clone(),
            vec![backup_a.clone(), backup_b.clone()],
            Some(telemetry.clone()),
            None,
        );

        plan.record_request_success(&off_winner_child);
        plan.record_request_route(&primary, &off_winner_child);

        let rerouted = plan
            .retry_url_for_failure(&off_winner_child, CrawlFailureKind::Timeout, 4, None)
            .await
            .expect("subtree reroute");
        assert_ne!(rerouted, off_winner_child);

        let next_seed = plan
            .preferred_seed_for_request(0, &format!("{primary}{subtree_path}"), 3)
            .expect("quarantined standby should yield a new subtree candidate");
        assert_eq!(next_seed, backup_b);

        plan.record_request_route(&primary, &format!("{next_seed}{subtree_path}"));

        let snapshot = telemetry.snapshot_counters();
        assert_eq!(snapshot.subtree_reroutes, 1);
        assert_eq!(snapshot.subtree_quarantine_hits, 1);
        assert_eq!(snapshot.off_winner_child_requests, 2);
    }

    #[test]
    fn persisted_subtree_preference_remaps_to_current_seed_host() {
        let persist_path = std::env::temp_dir().join(format!(
            "qilin_subtree_pref_{}_{}.json",
            std::process::id(),
            now_unix_secs()
        ));
        let _ = std::fs::remove_file(&persist_path);

        let persisted_plan = QilinRoutePlan::new(
            "http://primary.onion/11111111-1111-1111-1111-111111111111/".to_string(),
            vec!["http://backup-a.onion/11111111-1111-1111-1111-111111111111/".to_string()],
            None,
            Some(persist_path.clone()),
        );
        persisted_plan.record_request_success(
            "http://backup-a.onion/11111111-1111-1111-1111-111111111111/kent/2016/",
        );
        assert_eq!(persisted_plan.persist_subtree_preferences().unwrap(), 1);

        let restored_plan = QilinRoutePlan::new(
            "http://primary.onion/22222222-2222-2222-2222-222222222222/".to_string(),
            vec!["http://backup-a.onion/22222222-2222-2222-2222-222222222222/".to_string()],
            None,
            Some(persist_path.clone()),
        );
        assert_eq!(
            restored_plan.load_persisted_subtree_preferences().unwrap(),
            1
        );
        assert_eq!(
            restored_plan.preferred_seed_for_request(
                0,
                "http://primary.onion/22222222-2222-2222-2222-222222222222/kent/2016/",
                3,
            ),
            Some("http://backup-a.onion/22222222-2222-2222-2222-222222222222/".to_string())
        );

        let _ = std::fs::remove_file(&persist_path);
    }

    #[test]
    fn timeout_and_circuit_failures_move_into_degraded_lane_immediately() {
        assert_eq!(
            retry_lane_for_failure(CrawlFailureKind::Timeout, 1),
            RetryLane::Degraded
        );
        assert_eq!(
            retry_lane_for_failure(CrawlFailureKind::Circuit, 1),
            RetryLane::Degraded
        );
    }

    #[test]
    fn throttles_need_repeat_failure_before_degraded_lane() {
        assert_eq!(
            retry_lane_for_failure(CrawlFailureKind::Throttle, 1),
            RetryLane::Primary
        );
        assert_eq!(
            retry_lane_for_failure(CrawlFailureKind::Throttle, 2),
            RetryLane::Degraded
        );
        assert_eq!(
            retry_lane_for_failure(CrawlFailureKind::Http, 4),
            RetryLane::Primary
        );
    }

    #[test]
    fn service_unavailable_is_classified_as_throttle() {
        assert!(matches!(
            super::classify_http_status_failure(reqwest::StatusCode::SERVICE_UNAVAILABLE),
            CrawlFailureKind::Throttle
        ));
        assert!(matches!(
            super::classify_http_status_failure(reqwest::StatusCode::TOO_MANY_REQUESTS),
            CrawlFailureKind::Throttle
        ));
        assert!(matches!(
            super::classify_http_status_failure(reqwest::StatusCode::INTERNAL_SERVER_ERROR),
            CrawlFailureKind::Http
        ));
    }

    #[test]
    fn throttle_suppresses_fast_reescalation() {
        let gov = QilinCrawlGovernor {
            desired_active: AtomicUsize::new(4),
            in_flight: AtomicUsize::new(0),
            min_active: 2,
            max_active: 12,
            available_clients: 8,
            permit_budget: 8,
            reserve_for_downloads: false,
            successes: AtomicUsize::new(10),
            failures: AtomicUsize::new(0),
            timeouts: AtomicUsize::new(0),
            circuit_failures: AtomicUsize::new(0),
            throttles: std::sync::Arc::new(AtomicUsize::new(0)),
            http_failures: AtomicUsize::new(0),
            last_throttle_epoch_ms: std::sync::Arc::new(AtomicU64::new(0)),
            circuit_latency_sum_ms: std::array::from_fn(|_| AtomicU64::new(0)),
            circuit_request_count: std::array::from_fn(|_| AtomicU64::new(0)),
            circuit_error_count: std::array::from_fn(|_| AtomicUsize::new(0)),
            last_outlier_isolation_epoch_ms: std::array::from_fn(|_| AtomicU64::new(0)),
            repin_interval_hint: AtomicUsize::new(20),
            adaptive_ramp_ceiling: std::sync::Arc::new(AtomicUsize::new(12)),
            throttles_in_window: std::sync::Arc::new(AtomicUsize::new(0)),
            last_ewma_decay_epoch_ms: AtomicU64::new(0),
            last_ceiling_change_epoch_ms: AtomicU64::new(0),
            prev_ceiling_value: AtomicUsize::new(12),
            telemetry: None,
            listing_circuit_start: 0,
            listing_circuit_end: 8,
            download_circuit_start: 0,
            download_circuit_end: 8,
            listing_workers_active: AtomicUsize::new(0),
            download_workers_active: AtomicUsize::new(0),
        };

        // Without recent throttle: scale up by +4 (pending=100, success_ratio=1.0)
        let result = gov.rebalance(100);
        assert!(result.is_some());
        let (next, _) = result.unwrap();
        // Should scale up by +4 from 4 → 8
        assert!(
            next >= 7,
            "expected large step-up without throttle, got {next}"
        );

        // Reset governor to 4 workers
        gov.desired_active.store(4, Ordering::Relaxed);
        gov.successes.store(10, Ordering::Relaxed);

        // Now record a throttle (sets last_throttle_epoch_ms to now)
        gov.record_failure(CrawlFailureKind::Throttle);
        // Reset the failures/throttle counters as rebalance would have consumed them
        gov.failures.store(0, Ordering::Relaxed);
        gov.throttles.store(0, Ordering::Relaxed);
        gov.successes.store(10, Ordering::Relaxed);

        // With recent throttle: scale up should be limited to +1
        let result = gov.rebalance(100);
        assert!(result.is_some());
        let (next_throttled, _) = result.unwrap();
        // Should scale up by +1 from 4 → 5 (not +4 → 8)
        assert_eq!(next_throttled, 5, "expected +1 step with throttle cooldown");
    }

    #[test]
    fn url_depth_relative_returns_correct_depth() {
        let seed = "http://x.onion/root/";
        assert_eq!(url_depth_relative(seed, "http://x.onion/root/"), 0);
        assert_eq!(url_depth_relative(seed, "http://x.onion/root/A/"), 1);
        assert_eq!(url_depth_relative(seed, "http://x.onion/root/A/B/"), 2);
        assert_eq!(url_depth_relative(seed, "http://x.onion/root/A/B/C/"), 3);
        assert_eq!(
            url_depth_relative(seed, "http://x.onion/root/A/B/C/file.txt"),
            4
        );
        // Different origin falls back to absolute segment count
        assert!(url_depth_relative(seed, "http://other.onion/foo/bar/") > 0);
    }

    #[test]
    fn reconciliation_attempts_start_hot_and_cap() {
        assert_eq!(super::escalate_reconciliation_attempt(None), 8);
        assert_eq!(super::escalate_reconciliation_attempt(Some(8)), 10);
        assert_eq!(super::escalate_reconciliation_attempt(Some(14)), 15);
        assert_eq!(super::escalate_reconciliation_attempt(Some(15)), 15);
    }

    #[test]
    fn circuit_partition_reserves_for_downloads() {
        let gov = QilinCrawlGovernor::new(8, true, None);
        // 2/3 of 8 = 5 for listing, 3 for downloads
        assert_eq!(gov.listing_circuit_start, 0);
        assert_eq!(gov.listing_circuit_end, 5);
        assert_eq!(gov.download_circuit_start, 5);
        assert_eq!(gov.download_circuit_end, 8);
    }

    #[test]
    fn circuit_partition_unified_when_no_downloads() {
        let gov = QilinCrawlGovernor::new(8, false, None);
        assert_eq!(gov.listing_circuit_start, 0);
        assert_eq!(gov.listing_circuit_end, 8);
        // Download range starts at 0 (overlapping = all serve listing)
        assert_eq!(gov.download_circuit_start, 0);
        assert_eq!(gov.download_circuit_end, 8);
    }

    #[test]
    fn latency_outlier_candidate_flags_poisoned_circuit() {
        let gov = QilinCrawlGovernor::new(8, false, None);
        gov.circuit_request_count[0].store(12, Ordering::Relaxed);
        gov.circuit_latency_sum_ms[0].store(120_000, Ordering::Relaxed);
        for cid in 1..4 {
            gov.circuit_request_count[cid].store(12, Ordering::Relaxed);
            gov.circuit_latency_sum_ms[cid].store(12_000 + (cid as u64 * 1_200), Ordering::Relaxed);
        }

        let (cid, avg_ms, baseline_ms) = gov.latency_outlier_candidate().unwrap();
        assert_eq!(cid, 0);
        assert_eq!(avg_ms, 10_000);
        assert!(baseline_ms <= 1_300);
    }

    #[test]
    fn latency_outlier_isolation_respects_cooldown() {
        let gov = QilinCrawlGovernor::new(8, false, None);
        assert!(gov.mark_latency_outlier_isolation(0));
        assert!(!gov.mark_latency_outlier_isolation(0));
    }

    #[test]
    fn repin_interval_tightens_under_throttle_pressure() {
        let gov = QilinCrawlGovernor::new(8, false, None);
        gov.set_repin_interval_hint(24);
        assert_eq!(gov.current_repin_interval(), 24);

        gov.throttles_in_window.store(1, Ordering::Relaxed);
        assert_eq!(gov.current_repin_interval(), 12);

        gov.throttles_in_window.store(4, Ordering::Relaxed);
        assert_eq!(gov.current_repin_interval(), 8);
    }

    #[test]
    fn classify_qilin_ingress_detects_cms_launcher() {
        let (uuid, kind) = classify_qilin_ingress(
            "http://ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion/site/view?uuid=f0668431-ee3f-3570-99cb-ea7d9c0691c6",
        );
        assert_eq!(
            uuid.as_deref(),
            Some("f0668431-ee3f-3570-99cb-ea7d9c0691c6")
        );
        assert_eq!(kind, QilinIngressKind::CmsLauncher);
    }

    #[test]
    fn classify_qilin_ingress_detects_direct_listing() {
        let (uuid, kind) = classify_qilin_ingress(
            "http://35ojiqspz3f6mgitndh647672qrizhks63ptqhnzs2zxrhym3r5fyjqd.onion/61ad2a5f-7386-4d4b-b5a2-34a5603ddcc7/",
        );
        assert_eq!(
            uuid.as_deref(),
            Some("61ad2a5f-7386-4d4b-b5a2-34a5603ddcc7")
        );
        assert_eq!(kind, QilinIngressKind::DirectListing);
    }

    #[test]
    fn classify_qilin_ingress_returns_unknown_without_uuid() {
        let (uuid, kind) = classify_qilin_ingress(
            "http://ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion/",
        );
        assert!(uuid.is_none());
        assert_eq!(kind, QilinIngressKind::Unknown);
    }

    #[test]
    fn parse_qilin_json_handles_various_formats() {
        let base = "http://qilin.onion";

        // Format A: Root array
        let json_a = r#"[
            {"name": "file1.txt", "size": "1024", "type": "file", "uuid": "u1"},
            {"name": "folder1", "size": "-", "type": "dir", "uuid": "u2"}
        ]"#;
        let res_a = parse_qilin_json(json_a, base);
        assert_eq!(res_a.len(), 2);
        assert_eq!(res_a[0].path, "/file1.txt");
        assert_eq!(res_a[0].entry_type, EntryType::File);
        assert_eq!(res_a[0].raw_url, "http://qilin.onion/site/data?uuid=u1");
        assert_eq!(res_a[1].path, "/folder1");
        assert_eq!(res_a[1].entry_type, EntryType::Folder);
        assert_eq!(res_a[1].raw_url, "http://qilin.onion/site/view?uuid=u2");

        // Format B: Nested "data" array
        let json_b = r#"{
            "data": [
                {"name": "photo.jpg", "size": "500 KB", "type": "file", "download_url": "/dl/photo"}
            ]
        }"#;
        let res_b = parse_qilin_json(json_b, base);
        assert_eq!(res_b.len(), 1);
        assert_eq!(res_b[0].path, "/photo.jpg");
        assert_eq!(res_b[0].size_bytes, Some(512000));
        assert_eq!(res_b[0].raw_url, "http://qilin.onion/dl/photo");
    }

    #[test]
    fn parse_qilin_view_page_extracts_links() {
        let base = "http://qilin.onion";
        let html = r#"
            <div class="page-header-title">QData</div>
            <div class="item_box_photos">
                <a href="/site/view?uuid=aaaabbbb-cccc-dddd-eeee-ffffffffffff">Subfolder A</a>
            </div>
            <div class="item_box_photos">
                <a href="/site/data?uuid=11112222-3333-4444-5555-666666666666">report.pdf</a>
            </div>
            <a href="/site/view?uuid=ffffaaaa-bbbb-cccc-dddd-eeeeeeeeeeee">Subfolder B</a>
            <a href="/site/data?uuid=99998888-7777-6666-5555-111111111111">backup.zip</a>
            <a href="/site/data?uuid=f0668431-ee3f-3570-99cb-ea7d9c0691c6">Watch data</a>
        "#;

        let entries = parse_qilin_view_page(html, base, "parent-uuid");
        // Should find 4 entries (2 folders + 2 files), excluding "Watch data"
        assert_eq!(entries.len(), 4);

        let folders: Vec<_> = entries
            .iter()
            .filter(|e| e.entry_type == EntryType::Folder)
            .collect();
        let files: Vec<_> = entries
            .iter()
            .filter(|e| e.entry_type == EntryType::File)
            .collect();
        assert_eq!(folders.len(), 2);
        assert_eq!(files.len(), 2);

        assert!(folders.iter().any(|f| f.path.contains("Subfolder A")));
        assert!(files.iter().any(|f| f.path.contains("report.pdf")));
    }

    #[test]
    fn extract_watch_data_targets_resolves_and_deduplicates_links() {
        let base = "http://ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion";
        let other_host = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.onion";
        let html = r#"
            <a href="/site/data?uuid=afa2a0ea-20ba-3ddf-8c5c-2aeea9e5dc43">Watch data</a>
            <a href="http://ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion/site/data?uuid=afa2a0ea-20ba-3ddf-8c5c-2aeea9e5dc43">Watch data</a>
        "#;
        let html = format!(
            "{}<a href=\"http://{}/site/data?uuid=bbbbbbbb-20ba-3ddf-8c5c-2aeea9e5dc43\">Watch data</a>",
            html, other_host
        );

        let targets = extract_watch_data_targets(base, &html);
        assert_eq!(targets.len(), 2);
        assert_eq!(
            targets[0],
            "http://ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion/site/data?uuid=afa2a0ea-20ba-3ddf-8c5c-2aeea9e5dc43"
        );
        assert_eq!(
            targets[1],
            format!(
                "http://{}/site/data?uuid=bbbbbbbb-20ba-3ddf-8c5c-2aeea9e5dc43",
                other_host
            )
        );
    }
}
