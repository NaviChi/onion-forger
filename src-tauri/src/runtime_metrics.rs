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
}

impl RuntimeTelemetry {
    pub fn begin_crawl_session(&self) {
        self.crawl_active.store(true, Ordering::Relaxed);
        self.active_workers.store(0, Ordering::Relaxed);
        self.worker_target.store(0, Ordering::Relaxed);
        self.node_failovers.store(0, Ordering::Relaxed);
        self.throttle_count.store(0, Ordering::Relaxed);
        self.timeout_count.store(0, Ordering::Relaxed);
        if let Ok(mut host) = self.current_node_host.write() {
            *host = None;
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
}
