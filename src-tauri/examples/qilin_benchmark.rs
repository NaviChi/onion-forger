use crawli_lib::adapters::qilin::QilinAdapter;
use crawli_lib::adapters::{CrawlerAdapter, EntryType};
use crawli_lib::frontier::{CrawlOptions, CrawlerFrontier};
use std::sync::Arc;
use std::time::{Duration, Instant};

const CRAWL_DURATION_SECS: u64 = 300; // 5 minutes

#[tokio::main]
async fn main() {
    let app = tauri::Builder::default()
        .build(tauri::generate_context!())
        .expect("build tauri app");

    crawli_lib::tor::cleanup_stale_tor_daemons();

    println!("⏳ Bootstrapping 12 Tor daemons...");
    let (_swarm_guard, ports) = crawli_lib::tor::bootstrap_tor_cluster(app.handle().clone(), 12)
        .await
        .unwrap();
    println!("✅ Tor swarm online: {:?}", ports);

    let target = "http://ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion/site/view?uuid=c9d2ba19-6aa1-3087-8773-f63d023179ed";

    let opts = CrawlOptions {
        listing: true,
        sizes: true,
        circuits: Some(24),
        daemons: Some(12),
        ..Default::default()
    };

    let frontier = Arc::new(CrawlerFrontier::new(
        Some(app.handle().clone()),
        target.to_string(),
        12,
        true,
        ports.clone(),
        opts,
    ));

    let qilin_adapter = QilinAdapter::default();

    println!("\n=======================================================");
    println!("🏎️  QILIN 5-MINUTE CRAWL BENCHMARK");
    println!("Target: {}", target);
    println!("Duration: {} seconds", CRAWL_DURATION_SECS);
    println!("=======================================================\n");

    let start = Instant::now();

    // Spawn the crawl in a background task
    let frontier_clone = frontier.clone();
    let app_handle = app.handle().clone();
    let crawl_handle = tokio::spawn(async move {
        qilin_adapter.crawl(target, frontier_clone, app_handle).await
    });

    // Timer: cancel after 5 minutes
    let frontier_timer = frontier.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(CRAWL_DURATION_SECS)).await;
        println!("\n⏰ 5-MINUTE TIMER EXPIRED — Cancelling crawl...");
        frontier_timer.cancel();
    });

    // Wait for the crawl to finish (either naturally or via cancellation)
    let result = crawl_handle.await;
    let elapsed = start.elapsed();

    println!("\n=======================================================");
    println!("📊 CRAWL BENCHMARK RESULTS");
    println!("=======================================================");
    println!("Total Duration: {:.1}s", elapsed.as_secs_f64());

    match result {
        Ok(Ok(entries)) => {
            let files: Vec<_> = entries.iter().filter(|e| matches!(e.entry_type, EntryType::File)).collect();
            let dirs: Vec<_> = entries.iter().filter(|e| matches!(e.entry_type, EntryType::Folder)).collect();
            let total_size: u64 = files.iter().filter_map(|f| f.size_bytes).sum();

            println!("Total Entries:  {}", entries.len());
            println!("  Files:        {}", files.len());
            println!("  Directories:  {}", dirs.len());
            println!("Total Size:     {}", format_bytes(total_size));
            println!("Crawl Rate:     {:.1} entries/sec", entries.len() as f64 / elapsed.as_secs_f64());
            println!("Crawl Rate:     {:.1} entries/min", entries.len() as f64 / elapsed.as_secs_f64() * 60.0);

            // Print first 20 files as sample
            println!("\n--- Sample Files (first 20) ---");
            for (i, f) in files.iter().take(20).enumerate() {
                let size_str = f.size_bytes.map(|s| format_bytes(s)).unwrap_or_else(|| "?".to_string());
                println!("  {:>3}. {} [{}]", i + 1, f.path, size_str);
            }
            if files.len() > 20 {
                println!("  ... and {} more files", files.len() - 20);
            }
        }
        Ok(Err(e)) => {
            println!("Crawl Error: {}", e);
        }
        Err(e) => {
            println!("Task Error: {}", e);
        }
    }

    println!("=======================================================\n");

    drop(_swarm_guard);
}

fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;

    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}
