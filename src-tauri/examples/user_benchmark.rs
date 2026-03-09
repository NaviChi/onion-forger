use crawli_lib::adapters::qilin::QilinAdapter;
use crawli_lib::adapters::{CrawlerAdapter, EntryType};
use crawli_lib::frontier::{CrawlOptions, CrawlerFrontier};
use crawli_lib::AppState;
use std::sync::Arc;
use tauri::Builder;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let app = Builder::default()
        .manage(AppState::default())
        .build(tauri::generate_context!())?;
    let target = "http://25j35d6uf37tvfqt5pmz457yicgu35yhizojqxbfzv33dni2d73q3oad.onion/80349839-d06f-41a8-b954-3602fe60725a/".to_string();

    let options = CrawlOptions {
        listing: true,
        sizes: true,
        download: false,
        circuits: Some(96), // Use 96 circuits (8 daemons × 12)
        daemons: Some(8),   // Use 8 daemons
        agnostic_state: false,
        resume: false,
        resume_index: None,
        mega_password: None,
                stealth_ramp: true,
    };

    println!("Bootstrapping Tor cluster...");
    let tor_daemons = 8;
    let (_swarm, ports) =
        crawli_lib::tor::bootstrap_tor_cluster(app.handle().clone(), tor_daemons).await?;
    println!("Tor cluster bootstrapped on ports: {:?}", ports);

    if let Some(&port) = ports.first() {
        println!("Testing direct connection through port {}...", port);
        let proxy = reqwest::Proxy::all(format!("socks5h://127.0.0.1:{}", port))?;
        let client = reqwest::Client::builder()
            .proxy(proxy)
            .timeout(std::time::Duration::from_secs(30))
            .build()?;

        match client.get(&target).send().await {
            Ok(resp) => {
                println!("Direct connection response: {}", resp.status());
            }
            Err(e) => {
                println!("Direct connection failed: {}", e);
            }
        }
    }

    println!("Starting Qilin crawl on: {}", target);
    let start_time = std::time::Instant::now();

    let num_circuits = 96;
    let mut tor_ports = Vec::with_capacity(num_circuits);
    if ports.is_empty() {
        tor_ports = vec![0; num_circuits];
    } else {
        for i in 0..num_circuits {
            tor_ports.push(ports[i % ports.len()]);
        }
    }

    let frontier = Arc::new(CrawlerFrontier::new(
        Some(app.handle().clone()),
        target.clone(),
        num_circuits, // circuits
        false,        // force_tor
        tor_ports,
        Vec::new(),
        options,
        None,
    ));

    let adapter = QilinAdapter;
    let entries = adapter
        .crawl(&target, frontier, app.handle().clone())
        .await?;

    let duration_secs = start_time.elapsed().as_secs_f64();
    let num_entries = entries.len();
    let throughput = (num_entries as f64) / duration_secs;

    let mut num_files = 0;
    let mut num_folders = 0;
    let mut total_size: u64 = 0;

    for entry in entries {
        match entry.entry_type {
            EntryType::File => {
                num_files += 1;
                total_size += entry.size_bytes.unwrap_or(0);
            }
            EntryType::Folder => num_folders += 1,
        }
    }

    println!("\n=== QILIN CRAWL BENCHMARK RESULTS ===");
    println!("Target:        {}", target);
    println!("Total Entries: {}", num_entries);
    println!("  Files:       {}", num_files);
    println!("  Folders:     {}", num_folders);
    println!("Total Size:    {} bytes", total_size);
    println!("Duration:      {:.2} seconds", duration_secs);
    println!("Throughput:    {:.2} entries / sec", throughput);
    println!("=====================================\n");

    Ok(())
}
