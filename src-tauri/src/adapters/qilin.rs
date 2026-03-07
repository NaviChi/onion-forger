use crate::adapters::qilin_nodes::QilinNodeCache;
use crate::adapters::{CrawlerAdapter, EntryType, FileEntry, SiteFingerprint};
use crate::frontier::CrawlerFrontier;
use crate::path_utils;
use crate::runtime_metrics::RuntimeTelemetry;
use crate::subtree_heatmap::{HeatFailureKind, SubtreeHeatmap};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::mpsc;

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

fn env_bool(name: &str) -> bool {
    matches!(
        std::env::var(name).ok().as_deref(),
        Some("1" | "true" | "TRUE" | "yes" | "YES")
    )
}

#[derive(Clone)]
struct QilinRoutePlan {
    active_seed_url: Arc<tokio::sync::RwLock<String>>,
    standby_seed_urls: Arc<Vec<String>>,
    next_failover_idx: Arc<AtomicUsize>,
    telemetry: Option<RuntimeTelemetry>,
}

impl QilinRoutePlan {
    fn new(
        primary_seed_url: String,
        standby_seed_urls: Vec<String>,
        telemetry: Option<RuntimeTelemetry>,
    ) -> Self {
        Self {
            active_seed_url: Arc::new(tokio::sync::RwLock::new(primary_seed_url)),
            standby_seed_urls: Arc::new(standby_seed_urls),
            next_failover_idx: Arc::new(AtomicUsize::new(0)),
            telemetry,
        }
    }

    async fn current_seed_url(&self) -> String {
        self.active_seed_url.read().await.clone()
    }

    async fn failover_url(
        &self,
        failed_url: &str,
        failure_kind: CrawlFailureKind,
        attempt: u8,
        app: &AppHandle,
    ) -> Option<String> {
        let required_attempts = match failure_kind {
            CrawlFailureKind::Throttle => 2, // Fast failover for 403/400 DDoS protection
            CrawlFailureKind::Timeout | CrawlFailureKind::Circuit => 4,
            CrawlFailureKind::Http => 5, // Fallback even for HTTP errors eventually
        };

        if attempt < required_attempts {
            return None;
        }

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
        Some(remapped)
    }
}

fn failure_kind_label(kind: CrawlFailureKind) -> &'static str {
    match kind {
        CrawlFailureKind::Timeout => "timeout",
        CrawlFailureKind::Circuit => "circuit",
        CrawlFailureKind::Throttle => "throttle",
        CrawlFailureKind::Http => "http",
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

const CHILD_DIAGNOSTIC_LIMIT: usize = 16;

fn emit_limited_child_log(app: &AppHandle, counter: &AtomicUsize, stage: &str, message: String) {
    let idx = counter.fetch_add(1, Ordering::Relaxed);
    if idx < CHILD_DIAGNOSTIC_LIMIT {
        let line = format!("[Qilin Child {}] {}", stage, message);
        println!("{}", line);
        let _ = app.emit("log", line);
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
    throttles: AtomicUsize,
    http_failures: AtomicUsize,
    telemetry: Option<RuntimeTelemetry>,
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
        let default_max = if reserve_for_downloads { 8 } else { 12 };
        let max_active = env_usize(if reserve_for_downloads {
            "CRAWLI_QILIN_PAGE_WORKERS_DOWNLOAD_MAX"
        } else {
            "CRAWLI_QILIN_PAGE_WORKERS_MAX"
        })
        .unwrap_or(default_max)
        .clamp(min_active, effective_budget)
        .min(profile_budget.worker_cap.max(min_active));
        let desired_active = env_usize(if reserve_for_downloads {
            "CRAWLI_QILIN_PAGE_WORKERS_DOWNLOAD_START"
        } else {
            "CRAWLI_QILIN_PAGE_WORKERS_START"
        })
        .unwrap_or(if reserve_for_downloads { 4 } else { 6 })
        .clamp(min_active, max_active)
        .min(profile_budget.worker_cap.clamp(min_active, max_active));

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
            throttles: AtomicUsize::new(0),
            http_failures: AtomicUsize::new(0),
            telemetry,
        }
    }

    fn current_target(&self) -> usize {
        self.desired_active.load(Ordering::Relaxed)
    }

    fn record_success(&self) {
        self.successes.fetch_add(1, Ordering::Relaxed);
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
                self.throttles.fetch_add(1, Ordering::Relaxed);
                if let Some(telemetry) = &self.telemetry {
                    telemetry.record_throttle();
                }
            }
            CrawlFailureKind::Http => {
                self.http_failures.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    async fn acquire_slot(self: &Arc<Self>) -> QilinCrawlPermit {
        loop {
            let desired = self.desired_active.load(Ordering::Relaxed).max(1);
            let current = self.in_flight.load(Ordering::Relaxed);
            if current < desired
                && self
                    .in_flight
                    .compare_exchange(current, current + 1, Ordering::AcqRel, Ordering::Relaxed)
                    .is_ok()
            {
                if let Some(telemetry) = &self.telemetry {
                    telemetry.set_worker_metrics(current + 1, desired);
                }
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

        let mut next = current.min(pressure_cap).max(self.min_active);
        if pressure >= 0.85 {
            next = ((current * 2) / 3).max(self.min_active).min(pressure_cap);
        } else if pressure >= 0.70 {
            next = ((current * 4) / 5).max(self.min_active).min(pressure_cap);
        } else if throttles > 0 || circuit_failures >= 2 {
            next = ((current * 2) / 3).max(self.min_active);
        } else if timeouts >= 3 && timeouts >= successes.max(1) {
            next = ((current * 3) / 4).max(self.min_active);
        } else if http_failures >= 4 && http_failures > successes {
            next = ((current * 4) / 5).max(self.min_active);
        } else if pending > current * 3 && success_ratio > 0.90 && total >= current.min(6) {
            next = (current + 4).min(pressure_cap);
        } else if pending > current && success_ratio > 0.75 && total >= 4 {
            next = (current + 2).min(pressure_cap);
        } else if total >= 6 && success_ratio < 0.50 {
            next = ((current * 3) / 4).max(self.min_active);
        }
        next = next.clamp(self.min_active, pressure_cap);

        if next != current {
            self.desired_active.store(next, Ordering::Relaxed);
            if let Some(telemetry) = &self.telemetry {
                telemetry.set_worker_metrics(self.in_flight.load(Ordering::Relaxed), next);
            }
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
        let remaining = self
            .governor
            .in_flight
            .fetch_sub(1, Ordering::Release)
            .saturating_sub(1);
        if let Some(telemetry) = &self.governor.telemetry {
            telemetry.set_worker_metrics(remaining, self.governor.current_target());
        }
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
        fingerprint
            .body
            .contains("<div class=\"page-header-title\">QData</div>")
            || fingerprint.body.contains("Data browser")
            || fingerprint.body.contains("_csrf-blog")
            || fingerprint.body.contains("item_box_photos")
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
        let subtree_shaping_enabled = env_bool("CRAWLI_QILIN_SUBTREE_SHAPING");
        let subtree_heatmap_enabled =
            subtree_shaping_enabled && env_bool("CRAWLI_QILIN_SUBTREE_HEATMAP");
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
            if let Some(path) = &heatmap_path {
                let loaded_entries = heatmap.lock().await.entries.len();
                if loaded_entries > 0 {
                    let _ = app.emit(
                        "log",
                        format!(
                            "[Qilin] Loaded subtree heatmap: {} clustered prefixes from {}",
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

        // Phase 30: Multi-Node Storage Discovery with Persistent Cache
        let mut actual_seed_url = current_url.to_string();
        let mut standby_routes = Vec::new();
        if current_url.contains("/site/view") || current_url.contains("/site/data") {
            if let Some(uuid_start) = current_url.find("uuid=") {
                let uuid = current_url[uuid_start + 5..].trim_end_matches('/');

                let _ = app.emit(
                    "log",
                    format!("[Qilin] Phase 30: Multi-node discovery for UUID: {}", uuid),
                );
                println!(
                    "[Qilin Phase 30] Starting multi-node discovery for UUID: {}",
                    uuid
                );

                // Initialize the persistent node cache
                let node_cache = QilinNodeCache::default();
                if let Err(e) = node_cache.initialize().await {
                    eprintln!("[Qilin Phase 30] Failed to init node cache: {}", e);
                }

                // Pre-seed known QData storage domains as fallback (Stage C insurance)
                node_cache.seed_known_mirrors(uuid).await;

                // Run the 4-stage discovery algorithm
                let (_, client) = frontier.get_client();
                if let Some(best_node) = node_cache
                    .discover_and_resolve(current_url, uuid, &client)
                    .await
                {
                    actual_seed_url = best_node.url.clone();
                    if let Some(telemetry) = &telemetry {
                        telemetry.set_current_node_host(best_node.host.clone());
                    }
                    standby_routes =
                        standby_seed_urls(&best_node.url, &node_cache.get_nodes(uuid).await, 2);
                    println!(
                        "[Qilin Phase 30] ✅ Resolved to storage node: {} ({}ms, {} hits)",
                        best_node.host, best_node.avg_latency_ms, best_node.hit_count
                    );
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
                    println!("[Qilin Phase 42] ⚠ All storage nodes dead. Attempting direct UUID retry with NEWNYM...");
                    let _ = app.emit("log", "[Qilin] All storage nodes dead. Trying direct UUID construction with fresh circuits...".to_string());

                    // Blast NEWNYM to all active managed Tor daemons to get fresh circuits
                    for slot_idx in 0..crate::tor::active_client_count() {
                        let _ = crate::tor::request_newnym_slot(slot_idx).await;
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

                    let known_mirrors = vec![
                        "7mnkv5nvnjyifezlfyba6gek7aeimg5eghej5vp65qxnb2hjbtlttlyd.onion",
                        "25mjg55vcbjzwykz2uqsvaw7hcevm4pqxl42o324zr6qf5zgddmghkqd.onion",
                        "arrfcpipltlfgxc6hvjylixc6c5hrummwctz4wqysk3h56ntqz5scnad.onion",
                    ];

                    let mut found_alive = false;
                    for mirror in &known_mirrors {
                        let test_url = format!("http://{}/{}/", mirror, uuid);
                        println!("[Qilin Phase 42] Probing direct mirror: {}", test_url);
                        match tokio::time::timeout(
                            std::time::Duration::from_secs(15),
                            client.get(&test_url).send(),
                        )
                        .await
                        {
                            Ok(Ok(resp))
                                if resp.status().is_success()
                                    || resp.status().as_u16() == 301
                                    || resp.status().as_u16() == 302 =>
                            {
                                // Check if response body looks like an autoindex
                                let final_url = resp.url().as_str().to_string();
                                if let Ok(body) = resp.text().await {
                                    if body.contains("<table id=\"list\">")
                                        || body.contains("Index of")
                                        || body.contains("<td class=\"link\">")
                                    {
                                        println!("[Qilin Phase 42] ✅ Direct mirror alive with file index: {}", mirror);
                                        let _ = app.emit(
                                            "log",
                                            format!("[Qilin] ✅ Direct mirror alive: {}", mirror),
                                        );
                                        actual_seed_url = if final_url != test_url {
                                            final_url
                                        } else {
                                            test_url
                                        };
                                        if let Some(telemetry) = &telemetry {
                                            telemetry.set_current_node_host((*mirror).to_string());
                                        }
                                        found_alive = true;
                                        break;
                                    }
                                }
                            }
                            Ok(Ok(resp)) => {
                                println!(
                                    "[Qilin Phase 42] Mirror {} responded with {}",
                                    mirror,
                                    resp.status()
                                );
                            }
                            Ok(Err(e)) => {
                                println!("[Qilin Phase 42] Mirror {} unreachable: {}", mirror, e);
                            }
                            Err(_) => {
                                println!("[Qilin Phase 42] Mirror {} timed out", mirror);
                            }
                        }
                    }

                    if !found_alive {
                        println!(
                            "[Qilin Phase 42] ⚠ No alive mirrors found. Falling back to CMS URL."
                        );
                        let _ = app.emit("log", "[Qilin] No alive storage nodes. Using CMS URL directly (limited results expected).".to_string());
                    }
                }
            }
        }

        let route_plan = Arc::new(QilinRoutePlan::new(
            actual_seed_url.clone(),
            standby_routes,
            telemetry.clone(),
        ));

        // Reverted to Strict Depth-First Search parsing (Phase 27)
        queue.push(actual_seed_url.clone());
        frontier.mark_visited(&actual_seed_url);

        let pending = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        pending.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        let (ui_tx, mut ui_rx) = mpsc::channel::<FileEntry>(4096);
        let ui_app = app.clone();
        let vfs_for_batches = vfs.clone();
        let collected_entries_for_batches = collected_entries.clone();
        let ui_flush_task = tokio::spawn(async move {
            let mut batch = Vec::new();
            let mut interval = tokio::time::interval(std::time::Duration::from_millis(500));
            let mut channel_closed = false;
            loop {
                tokio::select! {
                    entry = ui_rx.recv(), if !channel_closed => {
                        match entry {
                            Some(entry) => {
                                batch.push(entry);
                                if batch.len() >= 500 {
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
        let governor = Arc::new(QilinCrawlGovernor::new(
            frontier.active_client_count(),
            reserve_for_downloads,
            telemetry.clone(),
        ));
        let max_concurrent = governor.max_active;
        let degraded_lane_limit = degraded_lane_limit(max_concurrent);
        let degraded_lane_interval = degraded_lane_interval();
        let governor_interval = governor_rebalance_interval();
        let degraded_in_flight = Arc::new(AtomicUsize::new(0));
        let degraded_dispatch_counter = Arc::new(AtomicUsize::new(0));
        let mut workers = tokio::task::JoinSet::new();

        let _ = app.emit(
            "log",
            format!(
                "[Qilin] Adaptive page governor online: target={} max={} reserve_for_downloads={} degraded_lane_max={} degraded_lane_interval={}",
                governor.current_target(),
                max_concurrent,
                reserve_for_downloads,
                degraded_lane_limit,
                degraded_lane_interval
            ),
        );

        {
            let governor = governor.clone();
            let pending = pending.clone();
            let cancel_flag = frontier.cancel_flag.clone();
            let app = app.clone();
            let governor_interval = governor_interval;
            tokio::spawn(async move {
                let mut idle_rounds = 0u8;
                loop {
                    tokio::time::sleep(governor_interval).await;
                    if cancel_flag.load(Ordering::Relaxed) {
                        break;
                    }

                    let pending_now = pending.load(Ordering::Relaxed);
                    let in_flight = governor.in_flight.load(Ordering::Relaxed);
                    if pending_now == 0 && in_flight == 0 {
                        idle_rounds = idle_rounds.saturating_add(1);
                        if idle_rounds >= 2 {
                            break;
                        }
                    } else {
                        idle_rounds = 0;
                    }

                    if let Some((next, pressure)) = governor.rebalance(pending_now) {
                        let _ = app.emit(
                            "log",
                            format!(
                                "[Qilin] Adaptive page governor adjusted to {} active workers (pending={}, in_flight={}, pressure={:.2})",
                                next, pending_now, in_flight, pressure
                            ),
                        );
                    }
                }
            });
        }

        let parsed_url = reqwest::Url::parse(current_url)?;
        let base_domain = format!(
            "{}://{}",
            parsed_url.scheme(),
            parsed_url.host_str().unwrap_or("")
        );

        for _ in 0..max_concurrent {
            let f = frontier.clone();
            let q_clone = queue.clone();
            let retry_q_clone = retry_queue.clone();
            let degraded_retry_q_clone = degraded_retry_queue.clone();
            let ui_tx_clone = ui_tx.clone();
            let ui_app_clone = app.clone();
            let pending_clone = pending.clone();
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

            let df_clone = discovered_folders.clone();
            let vf_clone = visited_folders.clone();

            workers.spawn(async move {
                let mut ddos = crate::adapters::qilin_ddos_guard::DdosGuard::new();
                let mut idle_sleep_ms: u64 = 50;
                let mut worker_client: Option<(usize, crate::arti_client::ArtiClient)> = None;
                loop {
                    if f.is_cancelled() {
                        break;
                    }

                    let mut degraded_lane_permit = None;
                    let (next_url, current_attempt) = match q_clone.pop() {
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
                                            idle_sleep_ms = 50;
                                            (payload.url, payload.attempt)
                                        } else {
                                            if pending_clone.load(Ordering::SeqCst) == 0
                                                && retry_q_clone.is_empty()
                                                && degraded_retry_q_clone.is_empty()
                                            {
                                                break;
                                            }
                                            tokio::time::sleep(std::time::Duration::from_millis(idle_sleep_ms)).await;
                                            idle_sleep_ms = std::cmp::min(idle_sleep_ms * 2, 800);
                                            continue;
                                        }
                                    }
                                } else if let Some(payload) = take_due_retry(&retry_q_clone) {
                                    idle_sleep_ms = 50;
                                    (payload.url, payload.attempt)
                                } else {
                                    if pending_clone.load(Ordering::SeqCst) == 0
                                        && retry_q_clone.is_empty()
                                        && degraded_retry_q_clone.is_empty()
                                    {
                                        break;
                                    }
                                    tokio::time::sleep(std::time::Duration::from_millis(idle_sleep_ms)).await;
                                    idle_sleep_ms = std::cmp::min(idle_sleep_ms * 2, 800);
                                    continue;
                                }
                            } else if let Some(payload) = take_due_retry(&retry_q_clone) {
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
                                    tokio::time::sleep(std::time::Duration::from_millis(idle_sleep_ms)).await;
                                    idle_sleep_ms = std::cmp::min(idle_sleep_ms * 2, 800);
                                    continue;
                                }
                            } else {
                                if pending_clone.load(Ordering::SeqCst) == 0
                                    && retry_q_clone.is_empty()
                                    && degraded_retry_q_clone.is_empty()
                                {
                                    break;
                                }
                                tokio::time::sleep(std::time::Duration::from_millis(idle_sleep_ms)).await;
                                idle_sleep_ms = std::cmp::min(idle_sleep_ms * 2, 800);
                                continue;
                            }
                        }
                    };
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
                        tokio::time::sleep(delay).await;
                    }

                    struct TaskGuard {
                        counter: Arc<std::sync::atomic::AtomicUsize>,
                    }
                    impl Drop for TaskGuard {
                        fn drop(&mut self) {
                            self.counter.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                        }
                    }
                    let _guard = TaskGuard { counter: pending_clone.clone() };
                    let active_seed_url = route_plan.current_seed_url().await;
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

                    // 7-pass Exponential Retry Pattern (Inverted Worker-Stealing)
                    let start_time = std::time::Instant::now();
                    let resp_result = tokio::time::timeout(
                        std::time::Duration::from_secs(45),
                        client.get(&next_url).send()
                    ).await;

                    let mut html = None;
                    let mut effective_url = next_url.clone();
                    let mut should_retry = false;
                    let mut retry_failure_kind = CrawlFailureKind::Http;

                    if let Ok(Ok(resp)) = resp_result {
                        let elapsed_ms = start_time.elapsed().as_millis() as u64;
                        let status = resp.status();
                        
                        if let Some(delay) = ddos.record_response(status.as_u16()) {
                            tokio::time::sleep(delay).await;
                        }

                        effective_url = resp.url().as_str().to_string();

                        if status.is_success() {
                            f.record_success(cid, 4096, elapsed_ms);
                            crawl_governor.record_success();
                            if let Ok(body) = resp.text().await {
                                html = Some(body);
                            } else {
                                f.record_failure(cid);
                                crawl_governor.record_failure(CrawlFailureKind::Http);
                                retry_failure_kind = CrawlFailureKind::Http;
                                should_retry = true;
                            }
                        } else if status == 404 {
                            f.record_success(cid, 512, elapsed_ms);
                            crawl_governor.record_success();
                        } else {
                            f.record_failure(cid);
                            let failure_kind = if status == reqwest::StatusCode::TOO_MANY_REQUESTS
                                || status == reqwest::StatusCode::SERVICE_UNAVAILABLE
                                || status == reqwest::StatusCode::FORBIDDEN
                                || status == reqwest::StatusCode::BAD_REQUEST
                            {
                                f.trigger_circuit_isolation(cid).await; // 403/400 means our Tor IP is blocked
                                CrawlFailureKind::Throttle
                            } else {
                                CrawlFailureKind::Http
                            };
                            crawl_governor.record_failure(failure_kind);
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
                        // Phase 46: Aerospace Grade Intelligent Healing
                        // Assess if this is a structurally collapsed Tor circuit
                        let is_collapsed = if resp_result.is_err() {
                            true // Outer timeout
                        } else if let Ok(Err(ref req_err)) = resp_result {
                            let err_str = req_err.to_string().to_lowercase();
                            err_str.contains("timeout") ||
                            err_str.contains("connection reset") ||
                            err_str.contains("broken pipe") ||
                            err_str.contains("eos") ||
                            err_str.contains("eof")
                        } else {
                            false
                        };

                        if is_collapsed {
                            f.record_failure(cid);
                            let failure_kind = match &resp_result {
                                Ok(Err(req_err)) => classify_request_error(req_err),
                                Err(_) => CrawlFailureKind::Timeout,
                                _ => CrawlFailureKind::Circuit,
                            };
                            crawl_governor.record_failure(failure_kind);
                            retry_failure_kind = failure_kind;
                            f.trigger_circuit_isolation(cid).await;
                        } else {
                            f.record_failure(cid);
                            retry_failure_kind = match &resp_result {
                                Ok(Err(req_err)) => classify_request_error(req_err),
                                Err(_) => CrawlFailureKind::Timeout,
                                _ => CrawlFailureKind::Http,
                            };
                            crawl_governor.record_failure(retry_failure_kind);
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
                                .failover_url(&next_url, retry_failure_kind, current_attempt, &ui_app_clone)
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
                            pending_clone.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
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

                    // Phase 44: Mark folder successfully visited
                    {
                        let mut vf = vf_clone.lock().await;
                        vf.insert(next_url.clone());
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

                                // PHASE 32: QData HTML V3 Parser
                                static V3_ROW_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
                                    regex::Regex::new(r#"<td class="link"><a href="([^"]+)"[^>]*>.*?</a></td><td class="size">([^<]*)</td>"#).unwrap()
                                });
                                let v3_row_re = &*V3_ROW_RE;

                                for cap in v3_row_re.captures_iter(&html) {
                                    found_any = true;
                                    if let (Some(href), Some(size_str)) = (cap.get(1), cap.get(2)) {
                                        let href_str = href.as_str();
                                        if href_str == "../" || href_str == "/" || href_str.starts_with("?") {
                                            continue;
                                        }

                                        let is_dir = href_str.ends_with('/');
                                        let clean_name = listing_entry_name(href_str);
                                        let child_url = resolve_listing_child_url(&effective_url, href_str, is_dir);

                                        if is_dir {
                                            let sanitized_name = path_utils::sanitize_path(&clean_name);
                                            let full_path = format!("{}{}", nested_path, sanitized_name);
                                            local_files.push(FileEntry {
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
                                            local_files.push(FileEntry {
                                                path: full_path,
                                                size_bytes,
                                                entry_type: EntryType::File,
                                                raw_url: child_url,
                                            });
                                        }
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
                                           local_files.push(FileEntry {
                                               path: full_path,
                                               size_bytes: None,
                                               entry_type: EntryType::Folder,
                                               raw_url: child_url.clone(),
                                           });
                                           local_folders.push(child_url);
                                       } else {
                                           let sanitized_name = path_utils::sanitize_path(&filename);
                                           let full_path = format!("{}{}", nested_path, sanitized_name);
                                           local_files.push(FileEntry {
                                               path: full_path,
                                               size_bytes: parsed_size,
                                               entry_type: EntryType::File,
                                               raw_url: child_url,
                                           });
                                       }
                                   }
                                }
                            }

                            // Always scan for the new CMS Blog layout recursively in the same block
                            for line in html.lines() {
                                if let Some(href_start) = line.find("href=\"") {
                                    let after_href = &line[href_start + 6..];
                                    if let Some(href_end) = after_href.find('"') {
                                        let raw_href = after_href[..href_end].to_string();

                                        if raw_href.starts_with("/uploads/") {
                                            let file_url = format!("{}{}", domain_clone, raw_href);
                                            let file_path = path_utils::sanitize_path(&raw_href);
                                            local_files.push(FileEntry {
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

                    new_files.extend(spawned_files);

                            {
                                let mut df = df_clone.lock().await;
                                for sub_url in &spawned_folders {
                                    df.insert(sub_url.clone());
                                }
                            }

                    for sub_url in spawned_folders {
                        if f.mark_visited(&sub_url) {
                            pending_clone.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
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

                if reconciliation_rounds >= 6 || stagnant_reconciliation_rounds >= 3 {
                    println!(
                        "[Qilin Phase 44] Reconciliation stalled at {} missing folders after {} rounds. Returning partial crawl instead of re-queueing forever.",
                        missing_count,
                        reconciliation_rounds
                    );
                    let _ = app.emit(
                        "log",
                        format!(
                            "[Qilin] Reconciliation stalled at {} missing folders after {} rounds. Returning partial results.",
                            missing_count,
                            reconciliation_rounds
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

                for slot_idx in 0..crate::tor::active_client_count() {
                    let _ = crate::tor::request_newnym_slot(slot_idx).await;
                }
                tokio::time::sleep(Duration::from_millis(1500)).await;

                // Re-inject the failed folders into the primary queue and revive the workers
                for folder in missing_folders {
                    pending.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    let route_to_degraded = subtree_shaping_enabled && {
                        let heatmap_key = SubtreeHeatmap::subtree_key(&actual_seed_url, &folder);
                        if let Some(key) = heatmap_key {
                            heatmap.lock().await.should_route_to_degraded(&key)
                        } else {
                            false
                        }
                    };
                    if route_to_degraded {
                        degraded_retry_queue.push(RetryPayload {
                            url: folder,
                            attempt: 1,
                            unlock_timestamp: std::time::Instant::now(),
                        });
                    } else {
                        queue.push(folder);
                    }
                }

                // Re-spawn the workers for the Tail-End Sweep
                for _ in 0..max_concurrent {
                    let f = frontier.clone();
                    let q_clone = queue.clone();
                    let retry_q_clone = retry_queue.clone();
                    let degraded_retry_q_clone = degraded_retry_queue.clone();
                    let ui_tx_clone = ui_tx.clone();
                    let ui_app_clone = app.clone();
                    let pending_clone = pending.clone();
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

                    let df_clone = discovered_folders.clone();
                    let vf_clone = visited_folders.clone();

                    workers.spawn(async move {
                        let mut ddos = crate::adapters::qilin_ddos_guard::DdosGuard::new();
                        let mut idle_sleep_ms: u64 = 50;
                        let mut worker_client: Option<(usize, crate::arti_client::ArtiClient)> = None;
                        loop {
                            if f.is_cancelled() { break; }

                            let mut degraded_lane_permit = None;
                            let (next_url, current_attempt) = match q_clone.pop() {
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
                                                    idle_sleep_ms = 50;
                                                    (payload.url, payload.attempt)
                                                } else {
                                                    if pending_clone.load(Ordering::SeqCst) == 0
                                                        && retry_q_clone.is_empty()
                                                        && degraded_retry_q_clone.is_empty()
                                                    {
                                                        break;
                                                    }
                                                    tokio::time::sleep(std::time::Duration::from_millis(idle_sleep_ms)).await;
                                                    idle_sleep_ms = std::cmp::min(idle_sleep_ms * 2, 800);
                                                    continue;
                                                }
                                            }
                                        } else if let Some(payload) = take_due_retry(&retry_q_clone) {
                                            idle_sleep_ms = 50;
                                            (payload.url, payload.attempt)
                                        } else {
                                            if pending_clone.load(Ordering::SeqCst) == 0
                                                && retry_q_clone.is_empty()
                                                && degraded_retry_q_clone.is_empty()
                                            {
                                                break;
                                            }
                                            tokio::time::sleep(std::time::Duration::from_millis(idle_sleep_ms)).await;
                                            idle_sleep_ms = std::cmp::min(idle_sleep_ms * 2, 800);
                                            continue;
                                        }
                                    } else if let Some(payload) = take_due_retry(&retry_q_clone) {
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
                                            tokio::time::sleep(std::time::Duration::from_millis(idle_sleep_ms)).await;
                                            idle_sleep_ms = std::cmp::min(idle_sleep_ms * 2, 800);
                                            continue;
                                        }
                                    } else {
                                        if pending_clone.load(Ordering::SeqCst) == 0
                                            && retry_q_clone.is_empty()
                                            && degraded_retry_q_clone.is_empty()
                                        {
                                            break;
                                        }
                                        tokio::time::sleep(std::time::Duration::from_millis(idle_sleep_ms)).await;
                                        idle_sleep_ms = std::cmp::min(idle_sleep_ms * 2, 800);
                                        continue;
                                    }
                                }
                            };
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
                                tokio::time::sleep(delay).await;
                            }

                            struct TaskGuard {
                                counter: Arc<std::sync::atomic::AtomicUsize>,
                            }
                            impl Drop for TaskGuard {
                                fn drop(&mut self) {
                                    self.counter.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                                }
                            }
                            let _guard = TaskGuard { counter: pending_clone.clone() };
                            let active_seed_url = route_plan.current_seed_url().await;
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
                                std::time::Duration::from_secs(45),
                                client.get(&next_url).send()
                            ).await;

                            let mut html = None;
                            let mut effective_url = next_url.clone();
                            let mut should_retry = false;
                            let mut retry_failure_kind = CrawlFailureKind::Http;

                            if let Ok(Ok(resp)) = resp_result {
                                let elapsed_ms = start_time.elapsed().as_millis() as u64;
                                let status = resp.status();
                                
                                if let Some(delay) = ddos.record_response(status.as_u16()) {
                                    tokio::time::sleep(delay).await;
                                }

                                effective_url = resp.url().as_str().to_string();

                                if status.is_success() {
                                    f.record_success(cid, 4096, elapsed_ms);
                                    crawl_governor.record_success();
                                    if let Ok(body) = resp.text().await {
                                        html = Some(body);
                                    } else {
                                        f.record_failure(cid);
                                        crawl_governor.record_failure(CrawlFailureKind::Http);
                                        retry_failure_kind = CrawlFailureKind::Http;
                                        should_retry = true;
                                    }
                                } else if status == 404 {
                                    f.record_success(cid, 512, elapsed_ms);
                                    crawl_governor.record_success();
                                } else {
                                    f.record_failure(cid);
                                    let failure_kind = if status == reqwest::StatusCode::TOO_MANY_REQUESTS
                                        || status == reqwest::StatusCode::SERVICE_UNAVAILABLE
                                    {
                                        CrawlFailureKind::Throttle
                                    } else {
                                        CrawlFailureKind::Http
                                    };
                                    crawl_governor.record_failure(failure_kind);
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
                                        .failover_url(&next_url, retry_failure_kind, current_attempt, &ui_app_clone)
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
                                    pending_clone.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
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

                            {
                                let mut vf = vf_clone.lock().await;
                                vf.insert(next_url.clone());
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
                                                    local_files.push(FileEntry {
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
                                                    local_files.push(FileEntry {
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
                                                   local_files.push(FileEntry {
                                                       path: full_path,
                                                       size_bytes: None,
                                                       entry_type: EntryType::Folder,
                                                       raw_url: child_url.clone(),
                                                   });
                                                   local_folders.push(child_url);
                                               } else {
                                                   let sanitized_name = path_utils::sanitize_path(&filename);
                                                   let full_path = format!("{}{}", nested_path, sanitized_name);
                                                   local_files.push(FileEntry {
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
                                                    local_files.push(FileEntry {
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

                            new_files.extend(spawned_files);

                            {
                                let mut df = df_clone.lock().await;
                                for sub_url in &spawned_folders {
                                    df.insert(sub_url.clone());
                                }
                            }

                            for sub_url in spawned_folders {
                                if f.mark_visited(&sub_url) {
                                    pending_clone.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
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
        }

        drop(ui_tx);
        let _ = ui_flush_task.await;

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

#[cfg(test)]
mod tests {
    use super::{
        remap_seed_url, retry_lane_for_failure, standby_seed_urls, CrawlFailureKind, RetryLane,
    };
    use crate::adapters::qilin_nodes::StorageNode;

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
}
