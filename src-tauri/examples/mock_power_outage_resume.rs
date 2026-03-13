use anyhow::{anyhow, Result};
use crawli_lib::frontier::CrawlOptions;
use crawli_lib::{start_crawl_for_example, AppState};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::Manager;

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> Result<()> {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let app = tauri::Builder::default()
        .manage(AppState::default())
        .build(tauri::generate_context!())
        .map_err(|e| anyhow!("build tauri app: {}", e))?;

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let output_dir = PathBuf::from(format!(
        "/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/tmp/mock_power_outage_{}",
        timestamp
    ));
    std::fs::create_dir_all(&output_dir)?;

    let url = "http://lockbit6vhrjaqzsdj6pqalyideigxv4xycfeyunpx35znogiwmojnid.onion/".to_string();

    println!("=== PHASE 1: INITIATING CRAWL (MOCKING POWER OUTAGE IN 15 SECONDS) ===");

    let crawl_result = tokio::time::timeout(
        Duration::from_secs(15),
        start_crawl_for_example(
            url.clone(),
            CrawlOptions {
                listing: true,
                sizes: true,
                download: false,
                circuits: Some(15),
                agnostic_state: false,
                resume: false,
                resume_index: None,
                mega_password: None,
                stealth_ramp: false, parallel_download: false,
            download_mode: crawli_lib::frontier::DownloadMode::Medium,
            force_clearnet: false,
            },
            output_dir.to_string_lossy().to_string(),
            app.handle().clone(),
        ),
    )
    .await;

    match crawl_result {
        Ok(Ok(_)) => {
            println!("Crawl completed too quickly to simulate a power outage.");
        }
        Ok(Err(e)) => {
            println!("Crawl failed early: {}", e);
        }
        Err(_) => {
            println!("=== SYSTEM CRASH (TIMEOUT) TRIGGERED ===");
            println!("Simulating hardware reboot... waiting 5 seconds.");
            tokio::time::sleep(Duration::from_secs(5)).await;

            println!("=== PHASE 2: SYSTEM RECOVERED. EXECUTING MULTI-STAGE RESUME ===");

            let resume_result = tokio::time::timeout(
                Duration::from_secs(45),
                start_crawl_for_example(
                    url.clone(),
                    CrawlOptions {
                        listing: true,
                        sizes: true,
                        download: false,
                        circuits: Some(15),
                        agnostic_state: false,
                        resume: true, // AUTO RESUME FLAG ENABLED
                        resume_index: None,
                        mega_password: None,
                        stealth_ramp: false, parallel_download: false,
            download_mode: crawli_lib::frontier::DownloadMode::Medium,
            force_clearnet: false,
                    },
                    output_dir.to_string_lossy().to_string(),
                    app.handle().clone(),
                ),
            )
            .await;

            match resume_result {
                Ok(Ok(res)) => {
                    println!("Resume completed successfully.");
                    println!("Crawl Session Result: {:?}", res);
                }
                Ok(Err(e)) => {
                    println!("Resume failed: {}", e);
                }
                Err(_) => {
                    println!("Resume also timed out.");
                }
            }
        }
    }

    Ok(())
}
