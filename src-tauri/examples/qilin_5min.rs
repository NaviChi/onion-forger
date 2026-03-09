use anyhow::Result;
use crawli_lib::adapters::qilin::QilinAdapter;
use crawli_lib::adapters::CrawlerAdapter;
use crawli_lib::frontier::{CrawlOptions, CrawlerFrontier};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::Manager;

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> Result<()> {
    let app = tauri::Builder::default()
        .manage(crawli_lib::AppState::default())
        .build(tauri::generate_context!())?;

    let state = app.handle().state::<crawli_lib::AppState>();
    let vfs_path = std::env::temp_dir().join(format!("crawli_5min_{}", std::process::id()));
    state.vfs.initialize(&vfs_path.to_string_lossy()).await?;

    let seed_url = "http://25j35d6uf37tvfqt5pmz457yicgu35yhizojqxbfzv33dni2d73q3oad.onion/80349839-d06f-41a8-b954-3602fe60725a/";

    let options = CrawlOptions {
        listing: true,
        sizes: true,
        download: false,
        circuits: Some(6),
        daemons: Some(1),
        agnostic_state: false,
        resume: false,
        resume_index: None,
        mega_password: None,
    };

    println!("Initializing Tor & Crawler components...");
    let init_start = Instant::now();

    // Explicitly bootstrap the Tor circuit pool so CrawlerFrontier detects live clients
    let _guard = crawli_lib::tor_native::bootstrap_arti_cluster(app.handle().clone(), 6).await?;

    // Since we are creating a single-swarm (1 daemon), we just use one port, say 9051.
    // If arti is embedded, it will handle it via frontier.
    let frontier = Arc::new(CrawlerFrontier::new(
        Some(app.handle().clone()),
        seed_url.to_string(),
        6,                                            // num_daemons (circuits/swarms)
        true,                                         // is_onion
        vec![9051],                                   // active_ports
        crawli_lib::tor_native::active_tor_clients(), // arti_clients
        options,
        None,
    ));

    let adapter = QilinAdapter;
    let app_handle = app.handle().clone();

    let init_elapsed = init_start.elapsed();
    println!("Initialization took: {:.2?}", init_elapsed);
    println!("Starting 5-minute crawl benchmark with 1 Swarm (6 circuits)...");

    let crawl_start = Instant::now();
    let monitor_handle = app.handle().clone();

    // Spawn monitor task
    tokio::spawn(async move {
        for minute in 1..=5 {
            tokio::time::sleep(Duration::from_secs(60)).await;
            let monitor_state = monitor_handle.state::<crawli_lib::AppState>();
            if let Ok(summary) = monitor_state.vfs.summarize_entries().await {
                println!(
                    "--- MINUTE {} --- Discovered: {} | Files: {} | Folders: {} | Rate: {:.2} entries/sec",
                    minute,
                    summary.discovered_count,
                    summary.file_count,
                    summary.folder_count,
                    summary.discovered_count as f64 / (minute * 60) as f64
                );
            }
        }
    });

    let crawl_future = adapter.crawl(&seed_url, frontier, app_handle);

    tokio::select! {
        res = crawl_future => {
            println!("Crawl completed before 5 minutes! Result: {:?}", res.is_ok());
        }
        _ = tokio::time::sleep(Duration::from_secs(300)) => {
            println!("5 minutes elapsed. Halting crawl.");
        }
    }

    let total_elapsed = crawl_start.elapsed();
    let final_summary = state.vfs.summarize_entries().await?;

    println!("=====================================================");
    println!("FINAL STATISTICS (5 Minutes)");
    println!("=====================================================");
    println!("Total Time Elapsed: {:.2?}", total_elapsed);
    println!("Discovered Nodes: {}", final_summary.discovered_count);
    println!("Files Identified: {}", final_summary.file_count);
    println!("Folders Identified: {}", final_summary.folder_count);
    println!(
        "Overall Throughput: {:.2} entries/sec",
        final_summary.discovered_count as f64 / total_elapsed.as_secs_f64()
    );
    println!("=====================================================");
    Ok(())
}
