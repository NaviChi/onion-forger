use crawli_lib::aria_downloader::{self, BatchFileEntry};
use crawli_lib::arti_client::ArtiClient;
use crawli_lib::resource_governor;
use crawli_lib::AppState;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::Manager;

#[derive(Clone, Debug, serde::Deserialize, Default)]
struct PersistedDownloadState {
    completed_chunks: Vec<bool>,
    #[serde(default)]
    current_offsets: Vec<u64>,
    num_circuits: usize,
    chunk_size: u64,
    content_length: u64,
    #[serde(default)]
    piece_mode: bool,
    #[serde(default)]
    completed_pieces: Vec<bool>,
    #[serde(default)]
    total_pieces: usize,
}

fn benchmark_url() -> String {
    std::env::var("DIRECT_BENCH_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "https://proof.ovh.net/files/10Gb.dat".to_string())
}

fn benchmark_duration_secs() -> u64 {
    std::env::var("DIRECT_BENCH_DURATION")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(60)
        .clamp(5, 600)
}

fn benchmark_connections() -> usize {
    std::env::var("DIRECT_BENCH_CONNECTIONS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(120)
        .max(1)
}

fn output_root() -> PathBuf {
    if let Ok(path) = std::env::var("DIRECT_BENCH_OUTPUT_ROOT") {
        let trimmed = path.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }

    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("tmp")
        .join(format!("direct_download_benchmark_{stamp}"))
}

fn output_filename(url: &str) -> String {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|parsed| {
            parsed
                .path_segments()
                .and_then(|mut segments| segments.next_back().map(|segment| segment.to_string()))
        })
        .filter(|segment| !segment.is_empty())
        .unwrap_or_else(|| "benchmark.bin".to_string())
}

fn format_resume_validator(probe: &aria_downloader::ProbeResult) -> &'static str {
    if probe.etag.is_some() {
        "etag"
    } else if probe.last_modified.is_some() {
        "last-modified"
    } else {
        "none"
    }
}

fn piece_len_for_index(content_length: u64, piece_size: u64, piece_idx: usize) -> u64 {
    let start = piece_idx as u64 * piece_size;
    if start >= content_length {
        return 0;
    }
    (((piece_idx as u64) + 1) * piece_size).min(content_length) - start
}

fn estimate_downloaded_bytes_from_state(path: &Path) -> Option<u64> {
    let state_path = format!("{}.ariaforge_state", path.display());
    let Ok(content) = std::fs::read_to_string(&state_path) else {
        return None;
    };
    let Ok(state) = serde_json::from_str::<PersistedDownloadState>(&content) else {
        return None;
    };

    if state.piece_mode && state.total_pieces > 0 && state.chunk_size > 0 {
        let mut downloaded = 0u64;
        for piece_idx in 0..state.total_pieces {
            let piece_len = piece_len_for_index(state.content_length, state.chunk_size, piece_idx);
            if piece_len == 0 {
                continue;
            }
            if state
                .completed_pieces
                .get(piece_idx)
                .copied()
                .unwrap_or(false)
            {
                downloaded = downloaded.saturating_add(piece_len);
            } else {
                let offset = state.current_offsets.get(piece_idx).copied().unwrap_or(0);
                downloaded = downloaded.saturating_add(offset.min(piece_len));
            }
        }
        return Some(downloaded.min(state.content_length));
    }

    if !state.completed_chunks.is_empty() && state.chunk_size > 0 {
        let mut downloaded = 0u64;
        for chunk_idx in 0..state.num_circuits {
            let chunk_start = chunk_idx as u64 * state.chunk_size;
            if chunk_start >= state.content_length {
                continue;
            }
            let chunk_len = (((chunk_idx as u64) + 1) * state.chunk_size).min(state.content_length)
                - chunk_start;
            if state
                .completed_chunks
                .get(chunk_idx)
                .copied()
                .unwrap_or(false)
            {
                downloaded = downloaded.saturating_add(chunk_len);
            } else {
                let offset = state.current_offsets.get(chunk_idx).copied().unwrap_or(0);
                downloaded = downloaded.saturating_add(offset.min(chunk_len));
            }
        }
        return Some(downloaded.min(state.content_length));
    }

    Some(0)
}

fn best_observed_downloaded_bytes(path: &Path) -> u64 {
    if let Some(state_bytes) = estimate_downloaded_bytes_from_state(path) {
        return state_bytes;
    }
    // Phase 135: Files now download directly to final path (no .ariaforge temp extension)
    actual_allocated_bytes(path)
}

#[cfg(unix)]
fn actual_allocated_bytes(path: &Path) -> u64 {
    use std::os::unix::fs::MetadataExt;

    std::fs::metadata(path)
        .map(|meta| meta.blocks().saturating_mul(512))
        .unwrap_or(0)
}

#[cfg(windows)]
fn actual_allocated_bytes(path: &Path) -> u64 {
    std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0)
}

#[tokio::main(flavor = "multi_thread", worker_threads = 8)]
async fn main() -> anyhow::Result<()> {
    let url = benchmark_url();
    let duration_secs = benchmark_duration_secs();
    let connections = benchmark_connections();
    let output_root = output_root();
    std::fs::create_dir_all(&output_root)?;

    let artifact_name = output_filename(&url);
    let target_path = output_root.join(&artifact_name);
    let target_path_string = target_path.to_string_lossy().to_string();

    let app = tauri::Builder::default()
        .manage(AppState::default())
        .build(tauri::generate_context!())
        .expect("build tauri app");
    let app_handle = app.handle().clone();

    let clearnet_probe_client = ArtiClient::new_clearnet();
    let probe = aria_downloader::probe_target(&clearnet_probe_client, &url, &app_handle).await?;
    let content_length = probe.content_length;
    let range_mode = probe.supports_ranges;
    let budget = resource_governor::recommend_download_budget(
        connections,
        Some(content_length),
        false,
        Some(&target_path),
        None,
    );
    let host_cap = aria_downloader::download_host_connection_cap_for_url(&url, false);

    println!(
        "[direct-bench] url={url} duration={}s requested_connections={} range_mode={} content_length={} resume_validator={} circuit_cap={} host_cap={} active_start={} tournament_cap={}",
        duration_secs,
        connections,
        range_mode,
        content_length,
        format_resume_validator(&probe),
        budget.circuit_cap,
        host_cap,
        budget.initial_active_cap,
        budget.tournament_cap
    );

    let control = aria_downloader::activate_download_control()
        .ok_or_else(|| anyhow::anyhow!("a download is already active"))?;
    let jwt_cache: Arc<tokio::sync::RwLock<std::collections::HashMap<String, String>>> =
        Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()));
    let entry = BatchFileEntry {
        url: url.clone(),
        path: target_path_string.clone(),
        size_hint: None,
        jwt_exp: None,
        alternate_urls: Vec::new(),
    };

    let started_at = Instant::now();
    let download_task = tokio::spawn(aria_downloader::start_download(
        app_handle.clone(),
        entry,
        connections,
        false,
        Some(output_root.to_string_lossy().to_string()),
        control,
        jwt_cache,
    ));

    tokio::time::sleep(Duration::from_secs(duration_secs)).await;
    let stopped = aria_downloader::request_stop();
    let task_result = download_task.await?;
    aria_downloader::clear_download_control();

    let downloaded_bytes = best_observed_downloaded_bytes(&target_path);
    let elapsed_secs = started_at.elapsed().as_secs_f64().max(0.001);
    let throughput_mib = downloaded_bytes as f64 / 1_048_576.0 / elapsed_secs;
    let throughput_mbps = downloaded_bytes as f64 * 8.0 / 1_000_000.0 / elapsed_secs;
    let metrics = app_handle.state::<AppState>().telemetry.snapshot_counters();

    println!(
        "[direct-bench] result={} stopped={} bytes={} elapsed_secs={:.2} throughput_mib_per_sec={:.2} throughput_mbps={:.2} transport={}/{}/{} output_root={}",
        if task_result.is_ok() { "ok" } else { "interrupted" },
        stopped,
        downloaded_bytes,
        elapsed_secs,
        throughput_mib,
        throughput_mbps,
        metrics.download_host_cache_hits,
        metrics.download_probe_promotion_hits,
        metrics.download_low_speed_aborts,
        output_root.display()
    );

    if let Err(err) = task_result {
        println!("[direct-bench] interrupted_reason={err}");
    }

    Ok(())
}
