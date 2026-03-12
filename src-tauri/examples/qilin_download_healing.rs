use anyhow::{anyhow, Context, Result};
use crawli_lib::aria_downloader;
use crawli_lib::aria_downloader::{BatchFileEntry, DownloadState};
use crawli_lib::AppState;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug)]
struct HealingConfig {
    url: String,
    output_path: String,
    circuits: usize,
    pause_after_secs: u64,
}

#[tokio::main(flavor = "multi_thread", worker_threads = 8)]
async fn main() -> Result<()> {
    let config = parse_args()?;
    let app = tauri::Builder::default()
        .manage(AppState::default())
        .build(tauri::generate_context!())
        .context("build tauri app")?;

    let output_path = PathBuf::from(&config.output_path);
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let temp_target = format!("{}.ariaforge", config.output_path);
    let state_target = format!("{}.ariaforge_state", config.output_path);
    let _ = std::fs::remove_file(&config.output_path);
    let _ = std::fs::remove_file(&temp_target);
    let _ = std::fs::remove_file(&state_target);

    println!("[1/4] Starting download and forcing a pause...");
    let control = aria_downloader::activate_download_control()
        .ok_or_else(|| anyhow!("download control already active"))?;
    let jwt_cache = Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()));
    let download_task = tokio::spawn(aria_downloader::start_download(
        app.handle().clone(),
        BatchFileEntry {
            url: config.url.clone(),
            alternate_urls: Vec::new(),
            path: config.output_path.clone(),
            size_hint: None,
            jwt_exp: None,
        },
        config.circuits,
        true,
        None,
        control,
        Arc::clone(&jwt_cache),
    ));

    let mut waited = 0u64;
    let mut checkpoint_pieces = None;
    while waited < config.pause_after_secs {
        if let Ok(raw) = std::fs::read_to_string(&state_target) {
            if let Ok(state) = serde_json::from_str::<DownloadState>(&raw) {
                if state.piece_mode && state.total_pieces > 0 {
                    let completed = state.completed_pieces.iter().filter(|done| **done).count();
                    if completed > 0 && completed < state.total_pieces {
                        checkpoint_pieces = Some((completed, state.total_pieces));
                        break;
                    }
                }
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
        waited += 1;
    }
    if let Some((completed, total)) = checkpoint_pieces {
        println!(
            "[*] Checkpoint detected before pause: {}/{} pieces complete",
            completed, total
        );
    } else {
        println!(
            "[!] No partial piece checkpoint detected before pause window expired. Probe may only validate restart-style recovery."
        );
    }
    let pause_requested = aria_downloader::request_pause();
    println!("[*] Pause requested: {}", pause_requested);

    let first_result = download_task.await.context("join paused download task")?;
    aria_downloader::clear_download_control();
    println!("[*] First pass result: {:?}", first_result);

    let temp_size = std::fs::metadata(&temp_target)
        .map(|meta| meta.len())
        .unwrap_or(0);
    let state_exists = PathBuf::from(&state_target).exists();
    let state_snapshot = if state_exists {
        std::fs::read_to_string(&state_target)
            .ok()
            .and_then(|raw| serde_json::from_str::<DownloadState>(&raw).ok())
    } else {
        None
    };
    println!(
        "[*] After pause: temp_size={} bytes, state_exists={}",
        temp_size, state_exists
    );
    if let Some(state) = &state_snapshot {
        let completed = state.completed_pieces.iter().filter(|done| **done).count();
        println!(
            "[*] Checkpoint snapshot: piece_mode={}, completed_pieces={}/{}",
            state.piece_mode, completed, state.total_pieces
        );
    }

    println!("[2/4] Resuming download...");
    let control = aria_downloader::activate_download_control()
        .ok_or_else(|| anyhow!("download control already active before resume"))?;
    let jwt_cache = Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()));
    let second_result = aria_downloader::start_download(
        app.handle().clone(),
        BatchFileEntry {
            url: config.url.clone(),
            alternate_urls: Vec::new(),
            path: config.output_path.clone(),
            size_hint: None,
            jwt_exp: None,
        },
        config.circuits,
        true,
        None,
        control,
        jwt_cache,
    )
    .await;
    aria_downloader::clear_download_control();
    second_result?;

    println!("[3/4] Resume completed.");
    let final_size = std::fs::metadata(&config.output_path)
        .map(|meta| meta.len())
        .unwrap_or(0);
    let temp_exists = PathBuf::from(&temp_target).exists();
    let state_exists = PathBuf::from(&state_target).exists();
    println!(
        "[4/4] Final state: final_size={} bytes, temp_exists={}, state_exists={}",
        final_size, temp_exists, state_exists
    );
    Ok(())
}

fn parse_args() -> Result<HealingConfig> {
    let mut url = None;
    let mut output_path = None;
    let mut circuits = 16usize;
    let mut pause_after_secs = 12u64;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--url" => url = args.next(),
            "--output" => output_path = args.next(),
            "--circuits" => {
                circuits = args
                    .next()
                    .ok_or_else(|| anyhow!("missing value for --circuits"))?
                    .parse()
                    .context("parse --circuits")?;
            }
            "--pause-after-secs" => {
                pause_after_secs = args
                    .next()
                    .ok_or_else(|| anyhow!("missing value for --pause-after-secs"))?
                    .parse()
                    .context("parse --pause-after-secs")?;
            }
            other => return Err(anyhow!("unknown arg: {}", other)),
        }
    }

    Ok(HealingConfig {
        url: url.ok_or_else(|| anyhow!("--url is required"))?,
        output_path: output_path.ok_or_else(|| anyhow!("--output is required"))?,
        circuits,
        pause_after_secs,
    })
}
