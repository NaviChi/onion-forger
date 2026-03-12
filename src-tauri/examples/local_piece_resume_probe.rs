use anyhow::{anyhow, Context, Result};
use crawli_lib::aria_downloader;
use crawli_lib::aria_downloader::{BatchFileEntry, DownloadState};
use crawli_lib::AppState;
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const PAYLOAD_SIZE: usize = 128 * 1024 * 1024;
const CHUNK_BYTES: usize = 64 * 1024;
const CHUNK_DELAY_MS: u64 = 120;
const ETAG: &str = "\"crawli-piece-resume-v1\"";
const LAST_MODIFIED: &str = "Thu, 06 Mar 2026 12:00:00 GMT";

#[derive(Default)]
struct ServerCounters {
    range_gets: AtomicUsize,
}

#[tokio::main(flavor = "multi_thread", worker_threads = 8)]
async fn main() -> Result<()> {
    let app = tauri::Builder::default()
        .manage(AppState::default())
        .build(tauri::generate_context!())
        .context("build tauri app")?;

    let payload = build_payload();
    let payload_hash = sha256_hex(&payload);
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let counters = Arc::new(ServerCounters::default());
    let server_task = tokio::spawn(run_range_server(
        listener,
        payload.clone(),
        Arc::clone(&counters),
    ));

    let output_path = "/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/tmp/local_piece_resume_probe/payload.bin".to_string();
    let output = PathBuf::from(&output_path);
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temp_target = format!("{}.ariaforge", output_path);
    let state_target = format!("{}.ariaforge_state", output_path);
    let _ = std::fs::remove_file(&output_path);
    let _ = std::fs::remove_file(&temp_target);
    let _ = std::fs::remove_file(&state_target);

    let url = format!("http://{}/payload.bin", addr);

    println!("[1/5] Starting piece-mode download against local deterministic range server...");
    let control = aria_downloader::activate_download_control()
        .ok_or_else(|| anyhow!("download control already active"))?;
    let jwt_cache = Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()));
    let first_pass = tokio::spawn(aria_downloader::start_download(
        app.handle().clone(),
        BatchFileEntry {
            url: url.clone(),
            alternate_urls: Vec::new(),
            path: output_path.clone(),
            size_hint: None,
            jwt_exp: None,
        },
        4,
        false,
        None,
        control,
        Arc::clone(&jwt_cache),
    ));

    let checkpoint = wait_for_piece_checkpoint(&state_target, Duration::from_secs(45)).await?;
    println!(
        "[*] Piece checkpoint detected: {}/{} pieces complete",
        checkpoint.0, checkpoint.1
    );

    let paused = aria_downloader::request_pause();
    println!("[*] Pause requested: {}", paused);
    let first_result = first_pass.await.context("join paused local probe")?;
    aria_downloader::clear_download_control();
    println!("[*] First pass result: {:?}", first_result);

    let state_after_pause = std::fs::read_to_string(&state_target)
        .ok()
        .and_then(|raw| serde_json::from_str::<DownloadState>(&raw).ok())
        .ok_or_else(|| anyhow!("expected checkpoint state after pause"))?;
    let completed_before_resume = state_after_pause
        .completed_pieces
        .iter()
        .filter(|done| **done)
        .count();
    let range_gets_before_resume = counters.range_gets.load(Ordering::Relaxed);
    println!(
        "[2/5] Saved state after pause: piece_mode={}, completed_pieces={}/{}",
        state_after_pause.piece_mode, completed_before_resume, state_after_pause.total_pieces
    );

    println!("[3/5] Resuming piece-mode download...");
    let control = aria_downloader::activate_download_control()
        .ok_or_else(|| anyhow!("download control already active before resume"))?;
    let jwt_cache = Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()));
    aria_downloader::start_download(
        app.handle().clone(),
        BatchFileEntry {
            url: url.clone(),
            alternate_urls: Vec::new(),
            path: output_path.clone(),
            size_hint: None,
            jwt_exp: None,
        },
        4,
        false,
        None,
        control,
        jwt_cache,
    )
    .await?;
    aria_downloader::clear_download_control();

    let final_bytes = std::fs::read(&output_path)?;
    let final_hash = sha256_hex(&final_bytes);
    let resume_range_gets = counters
        .range_gets
        .load(Ordering::Relaxed)
        .saturating_sub(range_gets_before_resume);
    println!(
        "[4/5] Final file size={} bytes, hash_match={}",
        final_bytes.len(),
        final_hash == payload_hash
    );
    println!("[*] Resume phase range GET requests={}", resume_range_gets);

    println!(
        "[5/5] Resume carried piece-mode checkpoint forward successfully: {}",
        completed_before_resume > 0 && final_hash == payload_hash
    );

    server_task.abort();
    let _ = server_task.await;
    let _ = std::io::stdout().flush();
    std::process::exit(0);
}

fn build_payload() -> Vec<u8> {
    (0..PAYLOAD_SIZE)
        .map(|idx| ((idx * 31 + 7) % 251) as u8)
        .collect()
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

async fn wait_for_piece_checkpoint(state_path: &str, timeout: Duration) -> Result<(usize, usize)> {
    let started = tokio::time::Instant::now();
    loop {
        if started.elapsed() > timeout {
            return Err(anyhow!("timed out waiting for piece checkpoint"));
        }
        if let Ok(raw) = std::fs::read_to_string(state_path) {
            if let Ok(state) = serde_json::from_str::<DownloadState>(&raw) {
                if state.piece_mode && state.total_pieces > 0 {
                    let completed = state.completed_pieces.iter().filter(|done| **done).count();
                    if completed > 0 && completed < state.total_pieces {
                        return Ok((completed, state.total_pieces));
                    }
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

async fn run_range_server(
    listener: TcpListener,
    payload: Vec<u8>,
    counters: Arc<ServerCounters>,
) -> Result<()> {
    loop {
        let (mut socket, _) = listener.accept().await?;
        let payload = payload.clone();
        let counters = Arc::clone(&counters);
        tokio::spawn(async move {
            let _ = handle_connection(&mut socket, &payload, counters).await;
        });
    }
}

async fn handle_connection(
    socket: &mut tokio::net::TcpStream,
    payload: &[u8],
    counters: Arc<ServerCounters>,
) -> Result<()> {
    let mut buffer = vec![0u8; 8192];
    let read = socket.read(&mut buffer).await?;
    if read == 0 {
        return Ok(());
    }
    let request = String::from_utf8_lossy(&buffer[..read]);
    let mut lines = request.lines();
    let request_line = lines
        .next()
        .ok_or_else(|| anyhow!("missing request line"))?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("/");

    if path != "/payload.bin" {
        socket
            .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n")
            .await?;
        return Ok(());
    }

    let mut range = None;
    let mut saw_range = false;
    for line in lines {
        let lower = line.to_ascii_lowercase();
        if lower.starts_with("range:") {
            saw_range = true;
            if let Some(spec) = line.split(':').nth(1) {
                range = parse_range(spec.trim(), payload.len());
            }
        }
    }
    if method == "GET" && saw_range {
        counters.range_gets.fetch_add(1, Ordering::Relaxed);
    }

    let (status_line, start, end) = if let Some((start, end)) = range {
        ("HTTP/1.1 206 Partial Content", start, end)
    } else {
        ("HTTP/1.1 200 OK", 0usize, payload.len().saturating_sub(1))
    };
    let body = &payload[start..=end];

    let mut headers = format!(
        "{status_line}\r\nAccept-Ranges: bytes\r\nETag: {ETAG}\r\nLast-Modified: {LAST_MODIFIED}\r\nContent-Length: {}\r\n",
        body.len()
    );
    if status_line.contains("206") {
        headers.push_str(&format!(
            "Content-Range: bytes {}-{}/{}\r\n",
            start,
            end,
            payload.len()
        ));
    }
    headers.push_str("Connection: close\r\n\r\n");
    socket.write_all(headers.as_bytes()).await?;

    if method != "HEAD" {
        for chunk in body.chunks(CHUNK_BYTES) {
            socket.write_all(chunk).await?;
            tokio::time::sleep(Duration::from_millis(CHUNK_DELAY_MS)).await;
        }
    }
    Ok(())
}

fn parse_range(header_value: &str, total_len: usize) -> Option<(usize, usize)> {
    let spec = header_value.strip_prefix("bytes=")?;
    let (start, end) = spec.split_once('-')?;
    let start = start.parse::<usize>().ok()?;
    let end = if end.is_empty() {
        total_len.saturating_sub(1)
    } else {
        end.parse::<usize>().ok()?
    };
    if start >= total_len {
        return None;
    }
    Some((start, end.min(total_len.saturating_sub(1))))
}
