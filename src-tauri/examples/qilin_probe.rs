use crawli_lib::adapters::qilin::QilinAdapter;
use crawli_lib::adapters::CrawlerAdapter;
use crawli_lib::frontier::{CrawlOptions, CrawlerFrontier};

#[tokio::main]
async fn main() {
    let app = tauri::Builder::default()
        .build(tauri::generate_context!())
        .expect("build tauri app");

    crawli_lib::tor::cleanup_stale_tor_daemons();

    let (swarm_guard, ports) = crawli_lib::tor::bootstrap_tor_cluster(app.handle().clone(), 2)
        .await
        .unwrap();

    let targets = vec![
        "http://ef4p3qn56susyjy56vym4gawjzaoc52e52w545e7mu6qhbmfut5iwxqd.onion/0fd57037-2a80-46ab-b662-bc3f21dd1a1c/",
        "http://6esfx73oxphqeh2lpgporkw72uj2xqm5bbb6pfl24mt27hlll7jdswyd.onion/b06ff1c5-0f44-4d7f-b184-9e587d1977aa/",
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
            ..Default::default()
        };

        // Leverage the fast proxy swarm to extract optimally
        let frontier = std::sync::Arc::new(CrawlerFrontier::new(
            Some(app.handle().clone()),
            target.to_string(),
            2,
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
                    println!("  [✅ SUCCESS] Target online! Crawl completed successfully in {}ms.", start_time.elapsed().as_millis());
                    println!("  [✅] Extracted {} entries.", entries.len());
                    success_found = true;
                    break;
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
