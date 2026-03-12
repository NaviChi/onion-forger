/// Phase 76C: 30-minute soak test with `reserve_for_downloads=true` to validate
/// traffic separation (circuit partitioning) effectiveness.
///
/// Key metrics to watch:
/// - listing_cids=[0-N] vs download_cids=[N-M] in governor logs
/// - Entries/sec throughput compared to unified mode baselines
/// - Throttle count (should be ≤ baseline due to partitioned pressure)
/// - Memory stability (RSS should plateau, not grow)
///
/// Run: CRAWLI_MULTI_CLIENTS=8 CRAWLI_CIRCUITS_PER_CLIENT=6 cargo run --example qilin_30min_partition_soak --release
use anyhow::Result;
use crawli_lib::adapters::qilin::QilinAdapter;
use crawli_lib::adapters::CrawlerAdapter;
use crawli_lib::frontier::{CrawlOptions, CrawlerFrontier};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::Manager;

const SOAK_DURATION_SECS: u64 = 30 * 60; // 30 minutes
const CIRCUITS: usize = 8;

#[tokio::main(flavor = "multi_thread", worker_threads = 8)]
async fn main() -> Result<()> {
    let app = tauri::Builder::default()
        .manage(crawli_lib::AppState::default())
        .build(tauri::generate_context!())?;

    let state = app.handle().state::<crawli_lib::AppState>();
    let vfs_path =
        std::env::temp_dir().join(format!("crawli_76c_partition_{}", std::process::id()));
    state.vfs.initialize(&vfs_path.to_string_lossy()).await?;

    let seed_url = "http://ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion/f0668431-ee3f-3570-99cb-ea7d9c0691c6/";

    // KEY: download=true sets reserve_for_downloads=true, activating circuit partitioning
    let options = CrawlOptions {
        listing: true,
        sizes: true,
        download: true, // ← Phase 76C: THIS activates traffic separation
        circuits: Some(CIRCUITS),
        agnostic_state: true,
        resume: false,
        resume_index: None,
        mega_password: None,
        stealth_ramp: true, parallel_download: false,
            force_clearnet: false,
    };

    println!("=======================================================");
    println!("  Phase 76C: Traffic Separation 30-Minute Soak Test");
    println!("=======================================================");
    println!("  Seed:            {}", &seed_url[..60]);
    println!("  Circuits:        {}", CIRCUITS);
    println!("  reserve_for_dl:  true (circuit partitioning ACTIVE)");
    println!("  Duration:        {} minutes", SOAK_DURATION_SECS / 60);
    println!("  Expected:        listing_cids=[0-4] download_cids=[5-7]");
    println!("=======================================================\n");

    let init_start = Instant::now();
    println!(
        "[INIT] Bootstrapping Tor cluster with {} circuits...",
        CIRCUITS
    );

    crawli_lib::tor::cleanup_stale_tor_daemons();
    let (guard, _ports) = crawli_lib::tor::bootstrap_tor_cluster(
        app.handle().clone(),
        CIRCUITS,
        0, // node_offset
    )
    .await?;
    let arti_clients = guard.get_arti_clients();
    let _guard_arc = Arc::new(tokio::sync::Mutex::new(guard));
    println!("[INIT] {} arti clients ready", arti_clients.len());

    let frontier = Arc::new(CrawlerFrontier::new(
        Some(app.handle().clone()),
        seed_url.to_string(),
        arti_clients.len().max(1),
        true,
        Vec::new(),
        arti_clients,
        options,
        None,
    ));

    let adapter = QilinAdapter;
    let app_handle = app.handle().clone();

    let init_elapsed = init_start.elapsed();
    println!("[INIT] Bootstrap complete in {:.1?}", init_elapsed);
    println!("[SOAK] Starting 30-minute crawl with traffic separation...\n");

    let crawl_start = Instant::now();
    let monitor_handle = app.handle().clone();

    // Spawn monitor: prints stats every 60s
    tokio::spawn(async move {
        let mut last_count = 0usize;
        for minute in 1..=30 {
            tokio::time::sleep(Duration::from_secs(60)).await;
            let monitor_state = monitor_handle.state::<crawli_lib::AppState>();
            if let Ok(summary) = monitor_state.vfs.summarize_entries().await {
                let delta = summary.discovered_count.saturating_sub(last_count);
                let rate = summary.discovered_count as f64 / (minute * 60) as f64;
                let delta_rate = delta as f64 / 60.0;
                println!(
                    "--- MINUTE {:02} --- total={} files={} folders={} Δ={}/min rate={:.2}/s Δrate={:.2}/s size={:.1}MB",
                    minute,
                    summary.discovered_count,
                    summary.file_count,
                    summary.folder_count,
                    delta,
                    rate,
                    delta_rate,
                    summary.total_size_bytes as f64 / (1024.0 * 1024.0),
                );
                last_count = summary.discovered_count;
            }
        }
    });

    // Spawn memory monitor: every 5 minutes
    let _mem_handle = app.handle().clone();
    tokio::spawn(async move {
        let mut sys = sysinfo::System::new();
        let pid = sysinfo::Pid::from(std::process::id() as usize);
        for check in 1..=6 {
            tokio::time::sleep(Duration::from_secs(300)).await;
            sys.refresh_all();
            if let Some(proc_) = sys.process(pid) {
                let rss_mb = proc_.memory() as f64 / (1024.0 * 1024.0);
                println!(
                    "[MEMORY @{}min] RSS={:.1}MB CPU={:.1}%",
                    check * 5,
                    rss_mb,
                    proc_.cpu_usage()
                );
            }
        }
    });

    let crawl_future = adapter.crawl(&seed_url, frontier, app_handle);

    tokio::select! {
        res = crawl_future => {
            println!("\n[SOAK] Crawl completed before {} minutes! Result: {:?}",
                SOAK_DURATION_SECS / 60, res.is_ok());
        }
        _ = tokio::time::sleep(Duration::from_secs(SOAK_DURATION_SECS)) => {
            println!("\n[SOAK] {} minutes elapsed. Halting crawl.", SOAK_DURATION_SECS / 60);
        }
    }

    let total_elapsed = crawl_start.elapsed();
    let final_summary = state.vfs.summarize_entries().await?;

    println!("\n=======================================================");
    println!("  FINAL STATISTICS — Phase 76C Partition Soak");
    println!("=======================================================");
    println!("  Duration:          {:.1?}", total_elapsed);
    println!("  Discovered Nodes:  {}", final_summary.discovered_count);
    println!("  Files:             {}", final_summary.file_count);
    println!("  Folders:           {}", final_summary.folder_count);
    println!(
        "  Total Size:        {:.2} MB",
        final_summary.total_size_bytes as f64 / (1024.0 * 1024.0)
    );
    println!(
        "  Throughput:        {:.2} entries/sec",
        final_summary.discovered_count as f64 / total_elapsed.as_secs_f64()
    );
    println!("  Mode:              reserve_for_downloads=true (partitioned)");
    println!(
        "  Verdict:           {}",
        if final_summary.discovered_count > 500 {
            "✅ PASS"
        } else {
            "⚠️ LOW YIELD"
        }
    );
    println!("=======================================================");

    // Cleanup temp VFS
    let _ = std::fs::remove_dir_all(&vfs_path);

    Ok(())
}
