use crate::binary_telemetry::{
    self, BatchProgressFrame, CrawlStatusFrame, EventKind, ResourceMetricsFrame,
};
use crate::runtime_metrics::ResourceMetricsSnapshot;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager};

const TELEMETRY_BRIDGE_INTERVAL_MS: u64 = 250;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgeCrawlStatus {
    pub phase: String,
    pub progress_percent: f64,
    pub visited_nodes: usize,
    pub processed_nodes: usize,
    pub queued_nodes: usize,
    pub active_workers: usize,
    pub worker_target: usize,
    pub eta_seconds: Option<u64>,
    pub estimation: String,
    pub delta_new_files: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vanguard: Option<crate::VanguardTelemetry>,
}

impl From<crate::CrawlStatusUpdate> for BridgeCrawlStatus {
    fn from(value: crate::CrawlStatusUpdate) -> Self {
        Self {
            phase: value.phase,
            progress_percent: value.progress_percent,
            visited_nodes: value.visited_nodes,
            processed_nodes: value.processed_nodes,
            queued_nodes: value.queued_nodes,
            active_workers: value.active_workers,
            worker_target: value.worker_target,
            eta_seconds: value.eta_seconds,
            estimation: value.estimation,
            delta_new_files: value.delta_new_files,
            vanguard: value.vanguard,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgeBatchProgress {
    pub completed: usize,
    pub failed: usize,
    pub total: usize,
    pub current_file: String,
    pub speed_mbps: f64,
    pub downloaded_bytes: u64,
    pub active_circuits: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bbr_bottleneck_mbps: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ekf_covariance: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgeDownloadProgress {
    pub path: String,
    pub bytes_downloaded: u64,
    pub total_bytes: Option<u64>,
    pub speed_bps: u64,
    pub active_circuits: usize,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TelemetryBridgeUpdate {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub crawl_status: Option<BridgeCrawlStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_metrics: Option<ResourceMetricsSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub batch_progress: Option<BridgeBatchProgress>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub download_progress: Vec<BridgeDownloadProgress>,
}

impl TelemetryBridgeUpdate {
    fn has_payload(&self) -> bool {
        self.crawl_status.is_some()
            || self.resource_metrics.is_some()
            || self.batch_progress.is_some()
            || !self.download_progress.is_empty()
    }
}

#[derive(Default)]
struct TelemetryBridgeState {
    crawl_status: Option<BridgeCrawlStatus>,
    resource_metrics: Option<ResourceMetricsSnapshot>,
    batch_progress: Option<BridgeBatchProgress>,
    download_progress: HashMap<String, BridgeDownloadProgress>,
}

#[derive(Clone, Default)]
pub struct TelemetryBridge {
    state: Arc<Mutex<TelemetryBridgeState>>,
    crawl_seq: Arc<AtomicU64>,
    resource_seq: Arc<AtomicU64>,
    batch_seq: Arc<AtomicU64>,
    download_seq: Arc<AtomicU64>,
    emitter_started: Arc<AtomicBool>,
}

impl TelemetryBridge {
    pub fn publish_crawl_status(&self, status: BridgeCrawlStatus) {
        if let Ok(mut state) = self.state.lock() {
            state.crawl_status = Some(status);
            self.crawl_seq.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn publish_resource_metrics(&self, snapshot: ResourceMetricsSnapshot) {
        if let Ok(mut state) = self.state.lock() {
            state.resource_metrics = Some(snapshot);
            self.resource_seq.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn publish_batch_progress(&self, progress: BridgeBatchProgress) {
        if let Ok(mut state) = self.state.lock() {
            state.batch_progress = Some(progress);
            self.batch_seq.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn publish_download_progress(&self, progress: BridgeDownloadProgress) {
        if let Ok(mut state) = self.state.lock() {
            state
                .download_progress
                .insert(progress.path.clone(), progress);
            self.download_seq.fetch_add(1, Ordering::Relaxed);
        }
    }
}

pub fn spawn_bridge_emitter(app: AppHandle, bridge: TelemetryBridge) {
    if bridge.emitter_started.swap(true, Ordering::AcqRel) {
        return;
    }

    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(
            TELEMETRY_BRIDGE_INTERVAL_MS,
        ));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        let mut last_crawl_seq = 0u64;
        let mut last_resource_seq = 0u64;
        let mut last_batch_seq = 0u64;
        let mut last_download_seq = 0u64;

        loop {
            interval.tick().await;

            let crawl_seq = bridge.crawl_seq.load(Ordering::Relaxed);
            let resource_seq = bridge.resource_seq.load(Ordering::Relaxed);
            let batch_seq = bridge.batch_seq.load(Ordering::Relaxed);
            let download_seq = bridge.download_seq.load(Ordering::Relaxed);

            if crawl_seq == last_crawl_seq
                && resource_seq == last_resource_seq
                && batch_seq == last_batch_seq
                && download_seq == last_download_seq
            {
                continue;
            }

            let mut update = TelemetryBridgeUpdate::default();
            if let Ok(mut state) = bridge.state.lock() {
                if crawl_seq != last_crawl_seq {
                    update.crawl_status = state.crawl_status.clone();
                }
                if resource_seq != last_resource_seq {
                    update.resource_metrics = state.resource_metrics.clone();
                }
                if batch_seq != last_batch_seq {
                    update.batch_progress = state.batch_progress.clone();
                }
                if download_seq != last_download_seq {
                    update.download_progress = std::mem::take(&mut state.download_progress)
                        .into_values()
                        .collect();
                }
            }

            last_crawl_seq = crawl_seq;
            last_resource_seq = resource_seq;
            last_batch_seq = batch_seq;
            last_download_seq = download_seq;

            if update.has_payload() {
                let _ = app.emit("telemetry_bridge_update", update);
            }
        }
    });
}

pub(crate) fn publish_crawl_status(app: &AppHandle, status: crate::CrawlStatusUpdate) {
    binary_telemetry::emit_frame(
        EventKind::CrawlStatus,
        CrawlStatusFrame {
            phase: status.phase.clone(),
            progress_percent: status.progress_percent,
            visited_nodes: status.visited_nodes as u64,
            processed_nodes: status.processed_nodes as u64,
            queued_nodes: status.queued_nodes as u64,
            active_workers: status.active_workers as u32,
            worker_target: status.worker_target as u32,
            eta_seconds: status.eta_seconds,
            delta_new_files: status.delta_new_files as u64,
        },
    );

    if let Some(state) = app.try_state::<crate::AppState>() {
        state.telemetry_bridge.publish_crawl_status(status.into());
    }
}

pub(crate) fn publish_resource_metrics(app: &AppHandle, snapshot: ResourceMetricsSnapshot) {
    binary_telemetry::emit_frame(
        EventKind::ResourceMetrics,
        ResourceMetricsFrame {
            process_cpu_percent: snapshot.process_cpu_percent,
            process_memory_bytes: snapshot.process_memory_bytes,
            system_memory_used_bytes: snapshot.system_memory_used_bytes,
            system_memory_total_bytes: snapshot.system_memory_total_bytes,
            active_workers: snapshot.active_workers as u32,
            worker_target: snapshot.worker_target as u32,
            active_circuits: snapshot.active_circuits as u32,
            peak_active_circuits: snapshot.peak_active_circuits as u32,
            current_node_host: snapshot.current_node_host.clone(),
            node_failovers: snapshot.node_failovers as u32,
            throttle_count: snapshot.throttle_count as u32,
            timeout_count: snapshot.timeout_count as u32,
            // Phase 76: DDoS guard telemetry
            throttle_rate_per_sec: snapshot.throttle_rate_per_sec,
            phantom_pool_depth: snapshot.phantom_pool_depth as u32,
            subtree_reroutes: snapshot.subtree_reroutes as u32,
            subtree_quarantine_hits: snapshot.subtree_quarantine_hits as u32,
            off_winner_child_requests: snapshot.off_winner_child_requests as u32,
            winner_host: snapshot.winner_host.clone(),
            slowest_circuit: snapshot.slowest_circuit.clone(),
            late_throttles: snapshot.late_throttles as u32,
            outlier_isolations: snapshot.outlier_isolations as u32,
            download_host_cache_hits: snapshot.download_host_cache_hits as u32,
            download_probe_promotion_hits: snapshot.download_probe_promotion_hits as u32,
            download_low_speed_aborts: snapshot.download_low_speed_aborts as u32,
            download_probe_quarantine_hits: snapshot.download_probe_quarantine_hits as u32,
            download_probe_candidate_exhaustions: snapshot.download_probe_candidate_exhaustions
                as u32,
            qilin_fresh_redirect_candidates: snapshot.qilin_fresh_redirect_candidates as u32,
            qilin_stale_host_only_candidates: snapshot.qilin_stale_host_only_candidates as u32,
            qilin_degraded_stage_d_activations: snapshot
                .qilin_degraded_stage_d_activations as u32,
        },
    );

    if let Some(state) = app.try_state::<crate::AppState>() {
        state.telemetry_bridge.publish_resource_metrics(snapshot);
    }
}

pub(crate) fn publish_batch_progress(app: &AppHandle, progress: BridgeBatchProgress) {
    binary_telemetry::emit_frame(
        EventKind::BatchProgress,
        BatchProgressFrame {
            completed: progress.completed as u64,
            failed: progress.failed as u64,
            total: progress.total as u64,
            current_file: progress.current_file.clone(),
            downloaded_bytes: progress.downloaded_bytes,
            active_circuits: progress.active_circuits.map(|value| value as u32),
        },
    );

    if let Some(state) = app.try_state::<crate::AppState>() {
        state.telemetry_bridge.publish_batch_progress(progress);
    }
}

pub(crate) fn publish_download_progress(app: &AppHandle, progress: BridgeDownloadProgress) {
    if let Some(state) = app.try_state::<crate::AppState>() {
        state.telemetry_bridge.publish_download_progress(progress);
    }
}
