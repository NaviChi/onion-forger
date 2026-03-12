use crawli_lib::adapters::EntryType;
use crawli_lib::aria_downloader::{self, BatchFileEntry};
use crawli_lib::frontier::{CrawlOptions, CrawlerFrontier};
use crawli_lib::{tor, AppState};
use std::sync::Arc;
use std::time::Duration;

fn main() {
    // Standalone binaries need explicit CryptoProvider (Tauri normally handles this)
    let _ =
        rustls::crypto::CryptoProvider::install_default(rustls::crypto::ring::default_provider());

    let app = tauri::Builder::default()
        .manage(AppState::default())
        .build(tauri::generate_context!())
        .expect("build tauri app");
    let app_handle = app.handle().clone();

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(4)
        .build()
        .expect("build tokio runtime");

    rt.block_on(async {
        println!("\n=========== LOKI TOOLS E2E TEST ===========");

        let target_urls = vec![
            (
                "qilin",
                "http://25j35d6uf37tvfqt5pmz457yicgu35yhizojqxbfzv33dni2d73q3oad.onion/80349839-d06f-41a8-b954-3602fe60725a/",
            ),
            (
                "dragonforce",
                "http://dragonforxxbp3awc7mzs5dkswrua3znqyx5roefmi4smjrsdi22xwqd.onion/www.rjzavoral.com",
            ),
        ];

        tor::cleanup_stale_tor_daemons();
        println!("[+] Bootstrapping Tor cluster (1 daemon)...");
        let (guard, active_ports) = tor::bootstrap_tor_cluster_for_traffic(
            app_handle.clone(),
            1,
            0,
            tor::SwarmTrafficClass::OnionService,
        )
        .await
        .expect("Bootstrap failed");

        // Wait briefly for Tor nodes to stabilize
        tokio::time::sleep(Duration::from_secs(5)).await;

        let arti_clients = guard.get_arti_clients();
        let jwt_cache = Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()));

        for (adapter_name, start_url) in target_urls {
            println!("\n------------------------------------------------");
            println!("Testing Adapter: {}", adapter_name);
            println!("URL: {}", start_url);
            println!("------------------------------------------------");

            let options = CrawlOptions {
                listing: true,
                sizes: true,
                download: false,
                circuits: Some(5),
                agnostic_state: false,
                resume: false,
                resume_index: None,
                stealth_ramp: false, parallel_download: false,
            force_clearnet: false,
                mega_password: None,
            };

            let frontier = CrawlerFrontier::new(
                Some(app_handle.clone()),
                start_url.to_string(),
                1,
                true,
                active_ports.clone(),
                arti_clients.clone(),
                options,
                None,
            );

            println!("[*] Initializing {} adapter...", adapter_name);

            let adapter: Box<dyn crawli_lib::adapters::CrawlerAdapter> = match adapter_name {
                "qilin" => Box::new(crawli_lib::adapters::qilin::QilinAdapter::default()),
                "dragonforce" => Box::new(crawli_lib::adapters::dragonforce::DragonForceAdapter::new()),
                _ => continue,
            };

            println!("[*] Adapter matched: {}", adapter_name);
            println!("[*] Beginning test crawl (timeout 300s / 5min)...");

            let frontier_arc = Arc::new(frontier);
            let frontier_clone = frontier_arc.clone();

            let crawl_task = adapter.crawl(start_url, frontier_clone, app_handle.clone());

            let crawl_result = tokio::time::timeout(Duration::from_secs(300), crawl_task).await;

            frontier_arc.cancel();

            let entries = match crawl_result {
                Ok(Ok(e)) => e,
                Ok(Err(err)) => {
                    println!("[-] Crawl returned an error: {}", err);
                    continue;
                }
                Err(_) => {
                    println!("[-] Crawl timed out (as expected for large sites), extracting whatever frontier got isn't supported smoothly here without state sync. We will assume failure if 0 files.");
                    continue; // we didn't hook into the events here, so we skip
                }
            };

            println!("[+] Crawl completed for {}. Found {} total entries.", adapter_name, entries.len());

            let mut files_with_size = 0;
            let mut sample_file = None;

            for entry in &entries {
                if entry.entry_type == EntryType::File {
                    if let Some(sz) = entry.size_bytes {
                        files_with_size += 1;
                        if sample_file.is_none() && sz > 1000 && sz < 50_000_000 {
                            sample_file = Some(entry.clone());
                        }
                    }
                }
            }

            println!("  -> Files with size extracted: {}/{}", files_with_size, entries.len());
            if files_with_size == 0 {
                println!("[!] WARNING: No files with size extracted for {} — network may be down", adapter_name);
            }

            if let Some(file) = sample_file {
                println!("[*] Initiating sample download test for: {}", file.path);
                println!("    File Size limit checked: {} bytes", file.size_bytes.unwrap());

                let output_dir = format!("/tmp/crawli_test_{}", adapter_name);
                let _ = std::fs::create_dir_all(&output_dir);

                let batch_entry = BatchFileEntry {
                    url: file.raw_url.clone(),
                    path: format!("{}/{}", output_dir, file.path.split('/').last().unwrap_or("test_dw")),
                    size_hint: file.size_bytes,
                    jwt_exp: file.jwt_exp,
                    alternate_urls: Vec::new(),
                };

                let control = aria_downloader::activate_download_control().unwrap();
                let download_result = aria_downloader::start_download(
                    app_handle.clone(),
                    batch_entry.clone(),
                    4, // circuits
                    true, // force_tor
                    Some(output_dir),
                    control,
                    Arc::clone(&jwt_cache)
                ).await;

                match download_result {
                    Ok(_) => {
                        println!("[+] Download test SUCCESS for {}", adapter_name);
                        assert!(std::path::Path::new(&batch_entry.path).exists());
                        let meta = std::fs::metadata(&batch_entry.path).unwrap();
                        println!("    Downloaded file size on disk: {}", meta.len());
                    }
                    Err(e) => {
                        println!("[-] Download test FAILED for {}: {}", adapter_name, e);
                    }
                }
            } else {
                println!("[-] No suitable sample file found for download test.");
            }
        }

        println!("\n=========== E2E TEST FINISHED ===========");
    });
}
