use prost::Message;
use std::fs::OpenOptions;
use std::io::Write;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

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

pub fn emit_frame(kind: EventKind, payload: impl Message) {
    let Some(file_mutex) = sink().as_ref() else {
        return;
    };
    let payload = payload.encode_to_vec();
    let frame = TelemetryFrame {
        ts_ms: unix_now_ms(),
        kind: kind as u32,
        payload,
    };
    let encoded = frame.encode_length_delimited_to_vec();
    if let Ok(mut sink) = file_mutex.lock() {
        let _ = sink.file.write_all(&encoded);
        sink.pending_frames = sink.pending_frames.saturating_add(1);
        if sink.pending_frames >= 16 || sink.last_flush.elapsed() >= Duration::from_millis(250) {
            let _ = sink.file.flush();
            sink.pending_frames = 0;
            sink.last_flush = Instant::now();
        }
    }
}

fn unix_now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
