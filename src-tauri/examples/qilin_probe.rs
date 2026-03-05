use crawli_lib::adapters::qilin::QilinAdapter;
use crawli_lib::adapters::CrawlerAdapter;
use crawli_lib::frontier::{CrawlOptions, CrawlerFrontier};

#[tokio::main]
async fn main() {
    let app = tauri::Builder::default()
        .build(tauri::generate_context!())
        .expect("build tauri app");

    crawli_lib::tor::cleanup_stale_tor_daemons();

    let (swarm_guard, ports) = crawli_lib::tor::bootstrap_tor_cluster(app.handle().clone(), 12)
        .await
        .unwrap();

    let targets = vec![
        "http://ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion/site/view?uuid=c9d2ba19-6aa1-3087-8773-f63d023179ed",
    ];

    println!("\n=======================================================");
    println!("🔍 INITIATING QILIN MULTI-TARGET METHODOLOGY PROBE 🔍");
    println!("Targets to test: {}", targets.len());
    println!("=======================================================\n");

    let mut success_found = false;

    for (i, target) in targets.iter().enumerate() {
        println!("\n[Attempt {}/{}] Probing Target: {}", i + 1, targets.len(), target);

        let opts = CrawlOptions {
            listing: true,
            circuits: Some(24),
            daemons: Some(12),
            ..Default::default()
        };

        // Leverage the fast proxy swarm to extract optimally
        let frontier = std::sync::Arc::new(CrawlerFrontier::new(
            Some(app.handle().clone()),
            target.to_string(),
            12,
            true,
            ports.clone(),
            opts,
        ));
        let qilin_adapter = QilinAdapter::default();

        let start_time = std::time::Instant::now();
        
        match qilin_adapter.crawl(target, frontier, app.handle().clone()).await {
            Ok(entries) => {
                if entries.is_empty() {
                    println!("  [❌] Target {} offline or unreachable (0 entries parsed). Proceeding to fallback target...", target);
                } else {
                    let mut files = 0;
                    let mut dirs = 0;
                    for e in &entries {
                        if matches!(e.entry_type, crawli_lib::adapters::EntryType::Folder) {
                            dirs += 1;
                        } else {
                            files += 1;
                        }
                    }
                    println!("  [✅ SUCCESS] Target online! Crawl completed successfully in {}ms.", start_time.elapsed().as_millis());
                    println!("  [✅] Extracted {} total entries ({} files, {} directories).", entries.len(), files, dirs);
                    success_found = true;
                }
            }
            Err(e) => {
                println!("  [❌ ERROR] Crawl disconnected or failed: {}", e);
            }
        }
    }

    if !success_found {
        println!("\n=======================================================");
        println!("🚨 MULTI-TARGET FATAL FAILURE 🚨");
        println!("All three Qilin URLs are definitively OFFLINE or blocking Tor nodes.");
        println!("=======================================================\n");
    } else {
        println!("\n=======================================================");
        println!("🏁 ONLINE PROBE SUCCESSFUL 🏁");
        println!("We have successfully identified an online Qilin Mirror.");
        println!("=======================================================\n");
    }

    drop(swarm_guard);
}
