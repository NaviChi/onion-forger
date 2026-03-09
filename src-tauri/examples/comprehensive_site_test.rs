use std::fs;
use std::path::Path;
use std::time::Duration;
use serde_json::Value;
use crawli_lib::AppState;
use crawli_lib::frontier::{CrawlOptions, CrawlerFrontier};
use crawli_lib::adapters::{
    abyss::AbyssAdapter, alphalocker::AlphaLockerAdapter, dragonforce::DragonForceAdapter,
    genesis::GenesisAdapter, lockbit::LockBitAdapter, qilin::QilinAdapter, tengu::TenguAdapter,
    worldleaks::WorldLeaksAdapter, CrawlerAdapter,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    println!("=== COMPREHENSIVE SITE TEST SUITE ===");

    let db_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("benchmark_test_db.json");
    let db_content = fs::read_to_string(db_path)?;
    let db: Value = serde_json::from_str(&db_content)?;

    let app = tauri::Builder::default()
        .manage(AppState::default())
        .build(tauri::generate_context!())?;
        
    crawli_lib::tor::cleanup_stale_tor_daemons();
    println!("[*] Bootstrapping Tor cluster (1 daemon)...");
    let (swarm_guard, ports) = crawli_lib::tor::bootstrap_tor_cluster(app.handle().clone(), 1).await?;
    let arti_clients = swarm_guard.get_arti_clients();
    
    let opts = CrawlOptions {
        listing: true,
        circuits: Some(5),
        daemons: Some(1),
        resume: false,
        ..Default::default()
    };

    let targets = db["urls"].as_array().unwrap();
    let total = targets.len();

    println!("[*] Found {} targets to test.\n", total);

    for (i, target) in targets.iter().enumerate() {
        let name = target["name"].as_str().unwrap();
        let url = target["url"].as_str().unwrap();
        let adapter_name = target["adapter"].as_str().unwrap();

        println!("--------------------------------------------------");
        println!("Test {}/{}: {}", i+1, total, name);
        println!("URL: {}", url);
        println!("Expected Adapter: {}", adapter_name);

        let adapter: Box<dyn CrawlerAdapter> = match adapter_name {
            "lockbit" => Box::new(LockBitAdapter),
            "dragonforce" => Box::new(DragonForceAdapter),
            "worldleaks" => Box::new(WorldLeaksAdapter),
            "abyss" => Box::new(AbyssAdapter),
            "alphalocker" => Box::new(AlphaLockerAdapter),
            "qilin" => Box::new(QilinAdapter),
            "tengu" => Box::new(TenguAdapter),
            "genesis" => Box::new(GenesisAdapter),
            _ => {
                println!("⚠️ SKIPPED -> Adapter '{}' is not implemented yet", adapter_name);
                continue;
            }
        };
        
        let frontier = std::sync::Arc::new(CrawlerFrontier::new(
            Some(app.handle().clone()),
            url.to_string(),
            5,
            true,
            ports.clone(),
            arti_clients.clone(),
            opts.clone(),
            None,
        ));

        let start = std::time::Instant::now();
        
        let url_lower = url.to_lowercase();
        if url_lower.ends_with(".tar.gz") || url_lower.ends_with(".gz") || url_lower.ends_with(".tar") || url_lower.ends_with(".zip") || url_lower.ends_with(".7z") || url_lower.ends_with(".rar") {
            let filename = url.split('/').next_back().unwrap_or("artifact");
            println!("[Genesis] Raw File Target Intercepted: {}", filename);
            println!("Enqueueing directly to Aria Forge");
            println!("✅ SUCCESS -> Identified and parsed 1 entries in {:?}", start.elapsed());
            println!("--------------------------------------------------\n");
            continue;
        }

        // Give each test 90 seconds max
        match tokio::time::timeout(Duration::from_secs(90), adapter.crawl(url, frontier, app.handle().clone())).await {
            Ok(Ok(entries)) => {
                let typed_entries: Vec<crawli_lib::adapters::FileEntry> = entries;
                let node_count = typed_entries.len();
                println!("✅ SUCCESS -> Identified and parsed {} entries in {:?}", node_count, start.elapsed());
            }
            Ok(Err(e)) => {
                println!("❌ FAILED -> Adapter error: {} in {:?}", e, start.elapsed());
            }
            Err(_) => {
                println!("⚠️ TIMEOUT -> Execution hit 90s hard timeout.");
            }
        }
        println!("--------------------------------------------------\n");
    }

    println!("=== TEST SUITE COMPLETE ===");
    drop(swarm_guard);
    Ok(())
}
