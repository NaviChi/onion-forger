use prost::Message;
use std::fs::OpenOptions;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

pub static TELEMETRY_ENABLED: AtomicBool = AtomicBool::new(false);

struct TelemetrySink {
    file: std::fs::File,
    pending_frames: usize,
    last_flush: Instant,
}

static TELEMETRY_SINK: OnceLock<Option<Mutex<TelemetrySink>>> = OnceLock::new();

#[derive(Clone, Copy)]
pub enum EventKind {
    ResourceMetrics = 1,
    CrawlStatus = 2,
    BatchProgress = 3,
    DownloadStatus = 4,
}

#[derive(Clone, PartialEq, Message)]
pub struct TelemetryFrame {
    #[prost(uint64, tag = "1")]
    pub ts_ms: u64,
    #[prost(uint32, tag = "2")]
    pub kind: u32,
    #[prost(bytes, tag = "3")]
    pub payload: Vec<u8>,
}

#[derive(Clone, PartialEq, Message)]
pub struct ResourceMetricsFrame {
    #[prost(double, tag = "1")]
    pub process_cpu_percent: f64,
    #[prost(uint64, tag = "2")]
    pub process_memory_bytes: u64,
    #[prost(uint64, tag = "3")]
    pub system_memory_used_bytes: u64,
    #[prost(uint64, tag = "4")]
    pub system_memory_total_bytes: u64,
    #[prost(uint32, tag = "5")]
    pub active_workers: u32,
    #[prost(uint32, tag = "6")]
    pub worker_target: u32,
    #[prost(uint32, tag = "7")]
    pub active_circuits: u32,
    #[prost(uint32, tag = "8")]
    pub peak_active_circuits: u32,
    #[prost(string, optional, tag = "9")]
    pub current_node_host: Option<String>,
    #[prost(uint32, tag = "10")]
    pub node_failovers: u32,
    #[prost(uint32, tag = "11")]
    pub throttle_count: u32,
    #[prost(uint32, tag = "12")]
    pub timeout_count: u32,
    // Phase 76: DDoS guard telemetry
    #[prost(double, tag = "13")]
    pub throttle_rate_per_sec: f64,
    #[prost(uint32, tag = "14")]
    pub phantom_pool_depth: u32,
    #[prost(uint32, tag = "15")]
    pub subtree_reroutes: u32,
    #[prost(uint32, tag = "16")]
    pub subtree_quarantine_hits: u32,
    #[prost(uint32, tag = "17")]
    pub off_winner_child_requests: u32,
    #[prost(string, optional, tag = "18")]
    pub winner_host: Option<String>,
    #[prost(string, optional, tag = "19")]
    pub slowest_circuit: Option<String>,
    #[prost(uint32, tag = "20")]
    pub late_throttles: u32,
    #[prost(uint32, tag = "21")]
    pub outlier_isolations: u32,
}

#[derive(Clone, PartialEq, Message)]
pub struct CrawlStatusFrame {
    #[prost(string, tag = "1")]
    pub phase: String,
    #[prost(double, tag = "2")]
    pub progress_percent: f64,
    #[prost(uint64, tag = "3")]
    pub visited_nodes: u64,
    #[prost(uint64, tag = "4")]
    pub processed_nodes: u64,
    #[prost(uint64, tag = "5")]
    pub queued_nodes: u64,
    #[prost(uint32, tag = "6")]
    pub active_workers: u32,
    #[prost(uint32, tag = "7")]
    pub worker_target: u32,
    #[prost(uint64, optional, tag = "8")]
    pub eta_seconds: Option<u64>,
    #[prost(uint64, tag = "9")]
    pub delta_new_files: u64,
}

#[derive(Clone, PartialEq, Message)]
pub struct BatchProgressFrame {
    #[prost(uint64, tag = "1")]
    pub completed: u64,
    #[prost(uint64, tag = "2")]
    pub failed: u64,
    #[prost(uint64, tag = "3")]
    pub total: u64,
    #[prost(string, tag = "4")]
    pub current_file: String,
    #[prost(uint64, tag = "5")]
    pub downloaded_bytes: u64,
    #[prost(uint32, optional, tag = "6")]
    pub active_circuits: Option<u32>,
}

#[derive(Clone, PartialEq, Message)]
pub struct DownloadStatusFrame {
    #[prost(string, tag = "1")]
    pub phase: String,
    #[prost(string, tag = "2")]
    pub message: String,
    #[prost(double, optional, tag = "3")]
    pub download_time_secs: Option<f64>,
    #[prost(double, optional, tag = "4")]
    pub percent: Option<f64>,
}

fn sink() -> &'static Option<Mutex<TelemetrySink>> {
    TELEMETRY_SINK.get_or_init(|| {
        let path = std::env::var("CRAWLI_PROTOBUF_TELEMETRY_PATH").ok()?;
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .ok()?;
        Some(Mutex::new(TelemetrySink {
            file,
            pending_frames: 0,
            last_flush: Instant::now(),
        }))
    })
}

// In-memory Shared Ring Buffer for Front-End Polling (LMAX Disruptor style)
use crossbeam_queue::ArrayQueue;
static TELEMETRY_RING: OnceLock<ArrayQueue<Vec<u8>>> = OnceLock::new();

fn ring() -> &'static ArrayQueue<Vec<u8>> {
    // 4096 frames ensures no missed telemetry under high load given UI 250ms polling
    TELEMETRY_RING.get_or_init(|| ArrayQueue::new(4096))
}

#[tauri::command]
pub fn drain_telemetry_ring() -> Vec<u8> {
    let q = ring();
    let mut payload = Vec::new();
    while let Some(mut frame) = q.pop() {
        payload.append(&mut frame);
    }
    payload
}

pub fn emit_frame(kind: EventKind, payload: impl Message) {
    let payload_bytes = payload.encode_to_vec();
    let frame = TelemetryFrame {
        ts_ms: unix_now_ms(),
        kind: kind as u32,
        payload: payload_bytes,
    };
    let encoded = frame.encode_length_delimited_to_vec();

    // 1. Always push to Shared Memory Ring Buffer for UI
    let q = ring();
    // Force push: act as true ring buffer (overwrite oldest if full)
    let cloned = encoded.clone();
    if let Err(element) = q.push(cloned) {
        let _ = q.pop(); // drop oldest
        let _ = q.push(element); // retry push
    }

    // 2. Optionally write to trace file
    if TELEMETRY_ENABLED.load(Ordering::Relaxed) {
        if let Some(file_mutex) = sink().as_ref() {
            if let Ok(mut sink) = file_mutex.lock() {
                let _ = sink.file.write_all(&encoded);
                sink.pending_frames = std::cmp::max(sink.pending_frames + 1, 1);
                if sink.pending_frames >= 16
                    || sink.last_flush.elapsed() >= Duration::from_millis(250)
                {
                    let _ = sink.file.flush();
                    sink.pending_frames = 0;
                    sink.last_flush = Instant::now();
                }
            }
        }
    }
}

fn unix_now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_metrics_frame_round_trips_subtree_counters() {
        let payload = ResourceMetricsFrame {
            process_cpu_percent: 3.5,
            process_memory_bytes: 1024,
            system_memory_used_bytes: 2048,
            system_memory_total_bytes: 4096,
            active_workers: 6,
            worker_target: 16,
            active_circuits: 4,
            peak_active_circuits: 8,
            current_node_host: Some("winner.onion".to_string()),
            node_failovers: 2,
            throttle_count: 1,
            timeout_count: 0,
            throttle_rate_per_sec: 0.25,
            phantom_pool_depth: 5,
            subtree_reroutes: 7,
            subtree_quarantine_hits: 3,
            off_winner_child_requests: 1,
            winner_host: Some("winner.onion".to_string()),
            slowest_circuit: Some("c7:8450ms".to_string()),
            late_throttles: 2,
            outlier_isolations: 1,
        };
        let frame = TelemetryFrame {
            ts_ms: 42,
            kind: EventKind::ResourceMetrics as u32,
            payload: payload.encode_to_vec(),
        };
        let encoded = frame.encode_length_delimited_to_vec();

        let decoded_frame = TelemetryFrame::decode_length_delimited(encoded.as_slice()).unwrap();
        let decoded_payload =
            ResourceMetricsFrame::decode(decoded_frame.payload.as_slice()).unwrap();

        assert_eq!(decoded_payload.throttle_rate_per_sec, 0.25);
        assert_eq!(decoded_payload.phantom_pool_depth, 5);
        assert_eq!(decoded_payload.subtree_reroutes, 7);
        assert_eq!(decoded_payload.subtree_quarantine_hits, 3);
        assert_eq!(decoded_payload.off_winner_child_requests, 1);
        assert_eq!(decoded_payload.winner_host.as_deref(), Some("winner.onion"));
        assert_eq!(
            decoded_payload.slowest_circuit.as_deref(),
            Some("c7:8450ms")
        );
        assert_eq!(decoded_payload.late_throttles, 2);
        assert_eq!(decoded_payload.outlier_isolations, 1);
    }
}
