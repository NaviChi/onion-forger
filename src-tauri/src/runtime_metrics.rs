use serde::Serialize;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use sysinfo::{Pid, ProcessesToUpdate, System};
use tauri::AppHandle;

#[derive(Clone, Debug, Default, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceMetricsSnapshot {
    pub process_cpu_percent: f64,
    pub process_memory_bytes: u64,
    pub process_threads: usize,
    pub system_memory_used_bytes: u64,
    pub system_memory_total_bytes: u64,
    pub system_memory_percent: f64,
    pub active_workers: usize,
    pub worker_target: usize,
    pub active_circuits: usize,
    pub peak_active_circuits: usize,
    pub current_node_host: Option<String>,
    pub node_failovers: usize,
    pub throttle_count: usize,
    pub timeout_count: usize,
    pub uptime_seconds: u64,
    pub consensus_weight: u64,
    pub multi_client_rotations: usize,
    pub multi_client_count: usize,
    // Phase 76: Qilin DDoS guard telemetry
    pub throttle_rate_per_sec: f64,
    pub phantom_pool_depth: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub swarm_runtime_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub swarm_traffic_class: Option<String>,
    pub swarm_client_count: usize,
    pub managed_port_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub health_probe_target: Option<String>,
    pub total_requests: usize,
    pub successful_requests: usize,
    pub failed_requests: usize,
    pub fingerprint_latency_ms: u64,
    pub cached_route_hits: usize,
    pub subtree_reroutes: usize,
    pub subtree_quarantine_hits: usize,
    pub off_winner_child_requests: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub winner_host: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slowest_circuit: Option<String>,
    pub late_throttles: usize,
    pub outlier_isolations: usize,
}

#[derive(Clone, Default)]
pub struct RuntimeTelemetry {
    crawl_active: Arc<AtomicBool>,
    download_sessions: Arc<AtomicUsize>,
    active_workers: Arc<AtomicUsize>,
    worker_target: Arc<AtomicUsize>,
    active_circuits: Arc<AtomicUsize>,
    peak_active_circuits: Arc<AtomicUsize>,
    node_failovers: Arc<AtomicUsize>,
    throttle_count: Arc<AtomicUsize>,
    timeout_count: Arc<AtomicUsize>,
    current_node_host: Arc<RwLock<Option<String>>>,
    session_start: Arc<RwLock<Option<std::time::Instant>>>,
    multi_client_rotations: Arc<AtomicUsize>,
    multi_client_count: Arc<AtomicUsize>,
    // Phase 76: Qilin DDoS guard telemetry
    throttle_rate_per_sec: Arc<std::sync::RwLock<f64>>,
    phantom_pool_depth: Arc<AtomicUsize>,
    swarm_runtime_label: Arc<RwLock<Option<String>>>,
    swarm_traffic_class: Arc<RwLock<Option<String>>>,
    swarm_client_count: Arc<AtomicUsize>,
    managed_port_count: Arc<AtomicUsize>,
    health_probe_target: Arc<RwLock<Option<String>>>,
    total_requests: Arc<AtomicUsize>,
    successful_requests: Arc<AtomicUsize>,
    failed_requests: Arc<AtomicUsize>,
    discovery_requests: Arc<AtomicUsize>,
    discovery_successful_requests: Arc<AtomicUsize>,
    discovery_failed_requests: Arc<AtomicUsize>,
    fingerprint_latency_ms: Arc<std::sync::atomic::AtomicU64>,
    cached_route_hits: Arc<AtomicUsize>,
    subtree_reroutes: Arc<AtomicUsize>,
    subtree_quarantine_hits: Arc<AtomicUsize>,
    off_winner_child_requests: Arc<AtomicUsize>,
    winner_host: Arc<RwLock<Option<String>>>,
    slowest_circuit: Arc<RwLock<Option<String>>>,
    late_throttles: Arc<AtomicUsize>,
    outlier_isolations: Arc<AtomicUsize>,
}

impl RuntimeTelemetry {
    pub fn begin_crawl_session(&self) {
        self.crawl_active.store(true, Ordering::Relaxed);
        self.active_workers.store(0, Ordering::Relaxed);
        self.worker_target.store(0, Ordering::Relaxed);
        self.node_failovers.store(0, Ordering::Relaxed);
        self.throttle_count.store(0, Ordering::Relaxed);
        self.timeout_count.store(0, Ordering::Relaxed);
        self.total_requests.store(0, Ordering::Relaxed);
        self.successful_requests.store(0, Ordering::Relaxed);
        self.failed_requests.store(0, Ordering::Relaxed);
        self.discovery_requests.store(0, Ordering::Relaxed);
        self.discovery_successful_requests
            .store(0, Ordering::Relaxed);
        self.discovery_failed_requests.store(0, Ordering::Relaxed);
        self.fingerprint_latency_ms.store(0, Ordering::Relaxed);
        self.cached_route_hits.store(0, Ordering::Relaxed);
        self.subtree_reroutes.store(0, Ordering::Relaxed);
        self.subtree_quarantine_hits.store(0, Ordering::Relaxed);
        self.off_winner_child_requests.store(0, Ordering::Relaxed);
        self.late_throttles.store(0, Ordering::Relaxed);
        self.outlier_isolations.store(0, Ordering::Relaxed);
        if let Ok(mut host) = self.current_node_host.write() {
            *host = None;
        }
        if let Ok(mut winner) = self.winner_host.write() {
            *winner = None;
        }
        if let Ok(mut slowest) = self.slowest_circuit.write() {
            *slowest = None;
        }
        if let Ok(mut start) = self.session_start.write() {
            *start = Some(std::time::Instant::now());
        }
    }

    pub fn end_crawl_session(&self) {
        self.crawl_active.store(false, Ordering::Relaxed);
        self.active_workers.store(0, Ordering::Relaxed);
        self.worker_target.store(0, Ordering::Relaxed);
    }

    pub fn begin_download_session(&self) {
        let previous = self.download_sessions.fetch_add(1, Ordering::Relaxed);
        if previous == 0 {
            self.active_circuits.store(0, Ordering::Relaxed);
            self.peak_active_circuits.store(0, Ordering::Relaxed);
            if let Ok(mut start) = self.session_start.write() {
                if start.is_none() {
                    *start = Some(std::time::Instant::now());
                }
            }
        }
    }

    pub fn end_download_session(&self) {
        let previous = self.download_sessions.load(Ordering::Relaxed);
        if previous == 0 {
            return;
        }
        if self
            .download_sessions
            .fetch_sub(1, Ordering::Relaxed)
            .saturating_sub(1)
            == 0
        {
            self.active_circuits.store(0, Ordering::Relaxed);
        }
    }

    pub fn is_active(&self) -> bool {
        self.crawl_active.load(Ordering::Relaxed)
            || self.download_sessions.load(Ordering::Relaxed) > 0
    }

    pub fn set_worker_metrics(&self, active_workers: usize, worker_target: usize) {
        self.active_workers.store(active_workers, Ordering::Relaxed);
        self.worker_target.store(worker_target, Ordering::Relaxed);
    }

    pub fn set_active_circuits(&self, active_circuits: usize) {
        self.active_circuits
            .store(active_circuits, Ordering::Relaxed);
        loop {
            let current_peak = self.peak_active_circuits.load(Ordering::Relaxed);
            if active_circuits <= current_peak {
                break;
            }
            if self
                .peak_active_circuits
                .compare_exchange(
                    current_peak,
                    active_circuits,
                    Ordering::AcqRel,
                    Ordering::Relaxed,
                )
                .is_ok()
            {
                break;
            }
        }
    }

    pub fn set_current_node_host<S: Into<String>>(&self, host: S) {
        if let Ok(mut current) = self.current_node_host.write() {
            *current = Some(host.into());
        }
    }

    pub fn record_failover<S: Into<String>>(&self, host: S) {
        self.node_failovers.fetch_add(1, Ordering::Relaxed);
        self.set_current_node_host(host);
    }

    pub fn record_throttle(&self) {
        self.throttle_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_timeout(&self) {
        self.timeout_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn set_multi_client_metrics(&self, rotations: usize, count: usize) {
        self.multi_client_rotations
            .store(rotations, Ordering::Relaxed);
        self.multi_client_count.store(count, Ordering::Relaxed);
    }

    pub fn set_swarm_metrics(
        &self,
        runtime_label: &str,
        traffic_class: crate::tor::SwarmTrafficClass,
        client_count: usize,
        managed_port_count: usize,
        health_probe_target: impl Into<String>,
    ) {
        if let Ok(mut label) = self.swarm_runtime_label.write() {
            *label = Some(runtime_label.to_string());
        }
        if let Ok(mut traffic) = self.swarm_traffic_class.write() {
            *traffic = Some(format!("{traffic_class:?}"));
        }
        if let Ok(mut probe) = self.health_probe_target.write() {
            *probe = Some(health_probe_target.into());
        }
        self.swarm_client_count
            .store(client_count, Ordering::Relaxed);
        self.managed_port_count
            .store(managed_port_count, Ordering::Relaxed);
    }

    pub fn set_request_metrics(
        &self,
        total_requests: usize,
        successful_requests: usize,
        failed_requests: usize,
    ) {
        self.total_requests.store(total_requests, Ordering::Relaxed);
        self.successful_requests
            .store(successful_requests, Ordering::Relaxed);
        self.failed_requests
            .store(failed_requests, Ordering::Relaxed);
    }

    pub fn record_discovery_request_success(&self) {
        self.discovery_requests.fetch_add(1, Ordering::Relaxed);
        self.discovery_successful_requests
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_discovery_request_failure(&self) {
        self.discovery_requests.fetch_add(1, Ordering::Relaxed);
        self.discovery_failed_requests
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn set_fingerprint_latency_ms(&self, latency_ms: u64) {
        self.fingerprint_latency_ms
            .store(latency_ms, Ordering::Relaxed);
    }

    pub fn record_cached_route_hit(&self) {
        self.cached_route_hits.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_subtree_reroute(&self) {
        self.subtree_reroutes.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_subtree_quarantine_hit(&self) {
        self.subtree_quarantine_hits.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_off_winner_child_request(&self) {
        self.off_winner_child_requests
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn set_winner_host<S: Into<String>>(&self, host: S) {
        if let Ok(mut winner) = self.winner_host.write() {
            *winner = Some(host.into());
        }
    }

    pub fn set_slowest_circuit<S: Into<String>>(&self, summary: S) {
        if let Ok(mut slowest) = self.slowest_circuit.write() {
            *slowest = Some(summary.into());
        }
    }

    pub fn record_late_throttle(&self) {
        self.late_throttles.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_outlier_isolation(&self) {
        self.outlier_isolations.fetch_add(1, Ordering::Relaxed);
    }

    /// Phase 76: Update throttle rate telemetry
    pub fn set_throttle_rate_per_sec(&self, rate: f64) {
        if let Ok(mut guard) = self.throttle_rate_per_sec.write() {
            *guard = rate;
        }
    }

    /// Phase 76: Update phantom pool depth telemetry
    pub fn set_phantom_pool_depth(&self, depth: usize) {
        self.phantom_pool_depth.store(depth, Ordering::Relaxed);
    }

    fn build_snapshot(
        &self,
        process_cpu_percent: f64,
        process_memory_bytes: u64,
        process_threads: usize,
        system_memory_used_bytes: u64,
        system_memory_total_bytes: u64,
    ) -> ResourceMetricsSnapshot {
        let system_memory_percent = if system_memory_total_bytes > 0 {
            (system_memory_used_bytes as f64 / system_memory_total_bytes as f64) * 100.0
        } else {
            0.0
        };

        ResourceMetricsSnapshot {
            process_cpu_percent,
            process_memory_bytes,
            process_threads,
            system_memory_used_bytes,
            system_memory_total_bytes,
            system_memory_percent,
            active_workers: self.active_workers.load(Ordering::Relaxed),
            worker_target: self.worker_target.load(Ordering::Relaxed),
            active_circuits: self.active_circuits.load(Ordering::Relaxed),
            peak_active_circuits: self.peak_active_circuits.load(Ordering::Relaxed),
            current_node_host: self
                .current_node_host
                .read()
                .ok()
                .and_then(|host| host.clone()),
            node_failovers: self.node_failovers.load(Ordering::Relaxed),
            throttle_count: self.throttle_count.load(Ordering::Relaxed),
            timeout_count: self.timeout_count.load(Ordering::Relaxed),
            uptime_seconds: self
                .session_start
                .read()
                .ok()
                .and_then(|t| t.map(|i| i.elapsed().as_secs()))
                .unwrap_or(0),
            consensus_weight: self.active_circuits.load(Ordering::Relaxed) as u64 * 8192
                + self.throttle_count.load(Ordering::Relaxed) as u64 * 256,
            multi_client_rotations: self.multi_client_rotations.load(Ordering::Relaxed),
            multi_client_count: self.multi_client_count.load(Ordering::Relaxed),
            throttle_rate_per_sec: self
                .throttle_rate_per_sec
                .read()
                .ok()
                .map(|g| *g)
                .unwrap_or(0.0),
            phantom_pool_depth: self.phantom_pool_depth.load(Ordering::Relaxed),
            swarm_runtime_label: self
                .swarm_runtime_label
                .read()
                .ok()
                .and_then(|label| label.clone()),
            swarm_traffic_class: self
                .swarm_traffic_class
                .read()
                .ok()
                .and_then(|traffic| traffic.clone()),
            swarm_client_count: self.swarm_client_count.load(Ordering::Relaxed),
            managed_port_count: self.managed_port_count.load(Ordering::Relaxed),
            health_probe_target: self
                .health_probe_target
                .read()
                .ok()
                .and_then(|probe| probe.clone()),
            total_requests: self.total_requests.load(Ordering::Relaxed)
                + self.discovery_requests.load(Ordering::Relaxed),
            successful_requests: self.successful_requests.load(Ordering::Relaxed)
                + self.discovery_successful_requests.load(Ordering::Relaxed),
            failed_requests: self.failed_requests.load(Ordering::Relaxed)
                + self.discovery_failed_requests.load(Ordering::Relaxed),
            fingerprint_latency_ms: self.fingerprint_latency_ms.load(Ordering::Relaxed),
            cached_route_hits: self.cached_route_hits.load(Ordering::Relaxed),
            subtree_reroutes: self.subtree_reroutes.load(Ordering::Relaxed),
            subtree_quarantine_hits: self.subtree_quarantine_hits.load(Ordering::Relaxed),
            off_winner_child_requests: self.off_winner_child_requests.load(Ordering::Relaxed),
            winner_host: self.winner_host.read().ok().and_then(|host| host.clone()),
            slowest_circuit: self
                .slowest_circuit
                .read()
                .ok()
                .and_then(|summary| summary.clone()),
            late_throttles: self.late_throttles.load(Ordering::Relaxed),
            outlier_isolations: self.outlier_isolations.load(Ordering::Relaxed),
        }
    }

    pub fn snapshot_with_system(&self, sys: &mut System) -> ResourceMetricsSnapshot {
        let pid = Pid::from(std::process::id() as usize);
        sys.refresh_cpu_usage();
        sys.refresh_memory();
        sys.refresh_processes(ProcessesToUpdate::Some(&[pid]), true);

        let (process_cpu_percent, process_memory_bytes, process_threads) =
            if let Some(process) = sys.process(pid) {
                (
                    process.cpu_usage() as f64,
                    process.memory(),
                    process_thread_count(process),
                )
            } else {
                (0.0, 0, 0)
            };

        self.build_snapshot(
            process_cpu_percent,
            process_memory_bytes,
            process_threads,
            sys.used_memory(),
            sys.total_memory(),
        )
    }

    pub fn snapshot_counters(&self) -> ResourceMetricsSnapshot {
        self.build_snapshot(0.0, 0, 0, 0, 0)
    }
}

fn process_thread_count(process: &sysinfo::Process) -> usize {
    #[cfg(any(target_os = "linux", target_os = "android", target_os = "freebsd"))]
    {
        process.tasks().map(|tasks| tasks.len()).unwrap_or(0)
    }

    #[cfg(not(any(target_os = "linux", target_os = "android", target_os = "freebsd")))]
    {
        let _ = process;
        0
    }
}

pub fn spawn_metrics_emitter(app: AppHandle, telemetry: RuntimeTelemetry) {
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        let mut sys = System::new_all();
        sys.refresh_cpu_usage();

        loop {
            interval.tick().await;

            if !telemetry.is_active() {
                continue;
            }

            let snapshot = telemetry.snapshot_with_system(&mut sys);
            crate::telemetry_bridge::publish_resource_metrics(&app, snapshot);
        }
    });
}

pub struct CrawlSessionGuard {
    telemetry: RuntimeTelemetry,
}

impl CrawlSessionGuard {
    pub fn new(telemetry: RuntimeTelemetry) -> Self {
        telemetry.begin_crawl_session();
        Self { telemetry }
    }
}

impl Drop for CrawlSessionGuard {
    fn drop(&mut self) {
        self.telemetry.end_crawl_session();
    }
}

pub struct DownloadSessionGuard {
    telemetry: RuntimeTelemetry,
}

impl DownloadSessionGuard {
    pub fn new(telemetry: RuntimeTelemetry) -> Self {
        telemetry.begin_download_session();
        Self { telemetry }
    }
}

impl Drop for DownloadSessionGuard {
    fn drop(&mut self) {
        self.telemetry.end_download_session();
    }
}

#[cfg(test)]
mod tests {
    use super::RuntimeTelemetry;

    #[test]
    fn peak_active_circuits_tracks_maximum() {
        let telemetry = RuntimeTelemetry::default();
        telemetry.begin_download_session();
        telemetry.set_active_circuits(4);
        telemetry.set_active_circuits(2);
        telemetry.set_active_circuits(9);

        let snapshot = telemetry.build_snapshot(0.0, 0, 0, 0, 0);
        assert_eq!(snapshot.active_circuits, 9);
        assert_eq!(snapshot.peak_active_circuits, 9);
    }

    #[test]
    fn snapshot_maps_counters_and_memory_ratio() {
        let telemetry = RuntimeTelemetry::default();
        telemetry.begin_crawl_session();
        telemetry.set_worker_metrics(6, 12);
        telemetry.set_active_circuits(3);
        telemetry.set_current_node_host("cache-primary.onion");
        telemetry.record_failover("cache-standby.onion");
        telemetry.record_throttle();
        telemetry.record_timeout();

        let snapshot = telemetry.build_snapshot(18.5, 512 * 1024 * 1024, 7, 3_000, 6_000);
        assert_eq!(snapshot.active_workers, 6);
        assert_eq!(snapshot.worker_target, 12);
        assert_eq!(snapshot.active_circuits, 3);
        assert_eq!(snapshot.peak_active_circuits, 3);
        assert_eq!(
            snapshot.current_node_host.as_deref(),
            Some("cache-standby.onion")
        );
        assert_eq!(snapshot.node_failovers, 1);
        assert_eq!(snapshot.throttle_count, 1);
        assert_eq!(snapshot.timeout_count, 1);
        assert_eq!(snapshot.system_memory_percent, 50.0);
    }

    #[test]
    fn snapshot_includes_subtree_route_counters() {
        let telemetry = RuntimeTelemetry::default();
        telemetry.begin_crawl_session();
        telemetry.record_subtree_reroute();
        telemetry.record_subtree_quarantine_hit();
        telemetry.record_subtree_quarantine_hit();
        telemetry.record_off_winner_child_request();
        telemetry.set_winner_host("winner.onion");
        telemetry.set_slowest_circuit("c7:8450ms");
        telemetry.record_late_throttle();
        telemetry.record_outlier_isolation();
        telemetry.record_outlier_isolation();

        let snapshot = telemetry.build_snapshot(0.0, 0, 0, 0, 0);
        assert_eq!(snapshot.subtree_reroutes, 1);
        assert_eq!(snapshot.subtree_quarantine_hits, 2);
        assert_eq!(snapshot.off_winner_child_requests, 1);
        assert_eq!(snapshot.winner_host.as_deref(), Some("winner.onion"));
        assert_eq!(snapshot.slowest_circuit.as_deref(), Some("c7:8450ms"));
        assert_eq!(snapshot.late_throttles, 1);
        assert_eq!(snapshot.outlier_isolations, 2);
    }
}
