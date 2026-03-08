use crawli_lib::adapters::qilin::QilinAdapter;
use crawli_lib::adapters::CrawlerAdapter;
use crawli_lib::frontier::{CrawlOptions, CrawlerFrontier};

#[tokio::main]
async fn main() {
    println!("=== CLI Qilin Test (Export Text Validation) ===");

    let app = tauri::Builder::default()
        .build(tauri::generate_context!())
        .expect("build tauri app");

    crawli_lib::tor::cleanup_stale_tor_daemons();

    let (swarm_guard, ports) = crawli_lib::tor::bootstrap_tor_cluster(app.handle().clone(), 12)
        .await
        .unwrap();

    let target = "http://ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion/site/view?uuid=c9d2ba19-6aa1-3087-8773-f63d023179ed";

    println!("Probing Target: {}", target);

    let opts = CrawlOptions {
        listing: true,
        circuits: Some(24),
        daemons: Some(12),
        resume: false,
        ..Default::default()
    };
    let arti_clients = swarm_guard.get_arti_clients();

    let frontier = std::sync::Arc::new(CrawlerFrontier::new(
        Some(app.handle().clone()),
        target.to_string(),
        12,
        true,
        ports.clone(),
        arti_clients,
        opts,
        None,
    ));

    let qilin_adapter = QilinAdapter;
    let start_time = std::time::Instant::now();

    match qilin_adapter
        .crawl(target, frontier, app.handle().clone())
        .await
    {
        Ok(entries) => {
            println!(
                "  [✅ SUCCESS] Target online! Crawl completed successfully in {}ms.",
                start_time.elapsed().as_millis()
            );
            println!("  [✅] Extracted {} total entries.", entries.len());

            // Generate Text File
            let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
            let log_filename = format!("crawl_index_{}.txt", timestamp);
            let log_path = std::path::PathBuf::from(&log_filename);

            let mut content = String::with_capacity(entries.len() * 128);
            content.push_str(&format!(
                "CRAWL INDEX COMPLETED AT: {}\n",
                chrono::Local::now().to_rfc2822()
            ));
            content.push_str(&format!("TARGET URL: {}\n", target));
            content.push_str(&format!("TOTAL ENTRIES: {}\n", entries.len()));
            content.push_str(
                "========================================================================\n\n",
            );

            for file in &entries {
                let type_str = if matches!(file.entry_type, crawli_lib::adapters::EntryType::Folder)
                {
                    "[DIR]"
                } else {
                    "[FILE]"
                };
                let size_str = file
                    .size_bytes
                    .map(|s| format!("{} bytes", s))
                    .unwrap_or_else(|| "Unknown size".to_string());
                content.push_str(&format!("{:<7} {} ({})\n", type_str, file.path, size_str));
            }

            if std::fs::write(&log_path, content).is_ok() {
                println!("  [📝] Text File Generated: {}", log_path.display());
                let output = std::fs::read_to_string(&log_path).unwrap();
                println!(
                    "\n--- File Snippet ---\n{}...\n--------------------",
                    &output.chars().take(500).collect::<String>()
                );
            }
        }
        Err(e) => {
            println!("  [❌ ERROR] Crawl disconnected or failed: {}", e);
        }
    }

    drop(swarm_guard);
}
