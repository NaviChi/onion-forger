//! DragonForce benchmark: Low (4) vs Balanced (8) vs Performance (64) — 4 min each
use crawli_lib::adapters::dragonforce::parse_dragonforce_fsguest;
use crawli_lib::adapters::EntryType;
use crawli_lib::tor;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

const RUN_SECS: u64 = 240; // 4 minutes per preset
const PRESETS: &[(u8, &str)] = &[(4, "Low"), (8, "Balanced"), (64, "Performance")];

fn main() {
    let _ =
        rustls::crypto::CryptoProvider::install_default(rustls::crypto::ring::default_provider());

    let app = tauri::Builder::default()
        .manage(crawli_lib::AppState::default())
        .build(tauri::generate_context!())
        .expect("build tauri app");
    let app_handle = app.handle().clone();

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(8)
        .build()
        .expect("build tokio runtime");

    rt.block_on(async {
        let target = "http://dragonforxxbp3awc7mzs5dkswrua3znqyx5roefmi4smjrsdi22xwqd.onion/mobilelinkusa.com/";

        println!("\n╔══════════════════════════════════════════════════════╗");
        println!("║  DragonForce Preset Benchmark (4 min × 3 presets)   ║");
        println!("║  Target: mobilelinkusa.com                          ║");
        println!("╚══════════════════════════════════════════════════════╝\n");

        // Bootstrap shared Tor swarm once
        println!("[INIT] Bootstrapping shared Tor swarm (8 clients)...");
        let t0 = Instant::now();
        let (guard, _ports) = tor::bootstrap_tor_cluster_for_traffic(
            app_handle.clone(),
            8,
            0,
            tor::SwarmTrafficClass::OnionService,
        )
        .await
        .expect("Bootstrap failed");
        println!("[INIT] Swarm ready in {:.1}s\n", t0.elapsed().as_secs_f64());

        let shared_clients = guard.get_arti_clients();

        // Pre-warm HS descriptor with a single fetch
        println!("[INIT] Pre-warming HS descriptor for target domain...");
        let warmup_tc = shared_clients[0].read().unwrap().clone();
        let warmup_client = crawli_lib::arti_client::ArtiClient::new((*warmup_tc).clone(), None);
        for attempt in 0..3 {
            match tokio::time::timeout(Duration::from_secs(60), warmup_client.get(target).send()).await {
                Ok(Ok(r)) if r.status().is_success() => {
                    println!("[INIT] HS descriptor cached (attempt {})\n", attempt + 1);
                    break;
                }
                _ => {
                    if attempt < 2 {
                        tokio::time::sleep(Duration::from_secs(3)).await;
                    }
                }
            }
        }

        let mut all_results = Vec::new();

        for &(circuits, preset_name) in PRESETS {
            println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            println!("  PRESET: {} ({} circuits) — {} sec timeout", preset_name, circuits, RUN_SECS);
            println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

            let multi_clients = (circuits as usize).min(8);
            let workers = circuits as usize;

            // Build pool
            let seeded = crawli_lib::multi_client_pool::snapshot_seed_clients(&shared_clients, multi_clients);
            let pool = Arc::new(
                crawli_lib::multi_client_pool::MultiClientPool::new_seeded(multi_clients, seeded, None)
                    .await
                    .unwrap(),
            );

            // Pre-heat 1 client
            {
                let tc = pool.get_client(0).await;
                let c = crawli_lib::arti_client::ArtiClient::new((*tc).clone(), None);
                let _ = tokio::time::timeout(Duration::from_secs(30), c.head(target).send()).await;
            }

            let queue: Arc<tokio::sync::Mutex<Vec<String>>> = Arc::new(tokio::sync::Mutex::new(vec![target.to_string()]));
            let visited: Arc<tokio::sync::Mutex<HashSet<String>>> = Arc::new(tokio::sync::Mutex::new(HashSet::new()));
            visited.lock().await.insert(target.to_string());

            let total_files = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let total_folders = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let total_bytes_indexed = Arc::new(std::sync::atomic::AtomicU64::new(0));
            let total_fetches = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let total_failures = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let deadline = Instant::now() + Duration::from_secs(RUN_SECS);

            let mut handles = Vec::new();
            for w in 0..workers {
                let pool_c = pool.clone();
                let q = queue.clone();
                let vis = visited.clone();
                let files_c = total_files.clone();
                let folders_c = total_folders.clone();
                let bytes_c = total_bytes_indexed.clone();
                let fetches_c = total_fetches.clone();
                let fails_c = total_failures.clone();

                handles.push(tokio::spawn(async move {
                    loop {
                        if Instant::now() >= deadline { break; }

                        let next = {
                            let mut q_lock = q.lock().await;
                            q_lock.pop()
                        };

                        let url = match next {
                            Some(u) => u,
                            None => {
                                tokio::time::sleep(Duration::from_millis(100)).await;
                                if Instant::now() >= deadline { break; }
                                continue;
                            }
                        };

                        let client_idx = w % 8;
                        let tc = pool_c.get_client(client_idx).await;
                        let client = crawli_lib::arti_client::ArtiClient::new((*tc).clone(), None);

                        let mut html = String::new();
                        let mut ok = false;
                        for _retry in 0..3 {
                            if Instant::now() >= deadline { break; }
                            fetches_c.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            match tokio::time::timeout(Duration::from_secs(45), client.get(&url).send()).await {
                                Ok(Ok(resp)) if resp.status().is_success() => {
                                    if let Ok(Ok(body)) = tokio::time::timeout(Duration::from_secs(30), resp.text()).await {
                                        html = body;
                                        ok = true;
                                        break;
                                    }
                                }
                                _ => {
                                    fails_c.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                    tokio::time::sleep(Duration::from_secs(2)).await;
                                }
                            }
                        }

                        if !ok || html.is_empty() { continue; }

                        let host = url::Url::parse(&url)
                            .ok()
                            .and_then(|u| u.host_str().map(|s| s.to_string()))
                            .unwrap_or_default();
                        let url_clone = url.clone();
                        let entries = tokio::task::spawn_blocking(move || {
                            parse_dragonforce_fsguest(&html, &host, &url_clone)
                        }).await.unwrap_or_default();

                        for e in &entries {
                            match e.entry_type {
                                EntryType::Folder => {
                                    folders_c.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                    let mut vis_lock = vis.lock().await;
                                    if vis_lock.insert(e.raw_url.clone()) {
                                        drop(vis_lock);
                                        q.lock().await.push(e.raw_url.clone());
                                    }
                                }
                                EntryType::File => {
                                    files_c.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                    if let Some(sz) = e.size_bytes {
                                        bytes_c.fetch_add(sz, std::sync::atomic::Ordering::Relaxed);
                                    }
                                }
                            }
                        }
                    }
                }));
            }

            // Progress ticker
            let files_tick = total_files.clone();
            let folders_tick = total_folders.clone();
            let fetches_tick = total_fetches.clone();
            let fails_tick = total_failures.clone();
            let bytes_tick = total_bytes_indexed.clone();
            let ticker = tokio::spawn(async move {
                let mut interval = tokio::time::interval(Duration::from_secs(30));
                let start = Instant::now();
                loop {
                    interval.tick().await;
                    if Instant::now() >= deadline { break; }
                    let elapsed = start.elapsed().as_secs();
                    let f = files_tick.load(std::sync::atomic::Ordering::Relaxed);
                    let d = folders_tick.load(std::sync::atomic::Ordering::Relaxed);
                    let req = fetches_tick.load(std::sync::atomic::Ordering::Relaxed);
                    let fail = fails_tick.load(std::sync::atomic::Ordering::Relaxed);
                    let b = bytes_tick.load(std::sync::atomic::Ordering::Relaxed);
                    println!("  [{:>3}s] files={} folders={} fetches={} fails={} indexed={:.1}MB",
                        elapsed, f, d, req, fail, b as f64 / 1_048_576.0);
                }
            });

            for h in handles { let _ = h.await; }
            ticker.abort();

            let files = total_files.load(std::sync::atomic::Ordering::Relaxed);
            let folders = total_folders.load(std::sync::atomic::Ordering::Relaxed);
            let fetches = total_fetches.load(std::sync::atomic::Ordering::Relaxed);
            let failures = total_failures.load(std::sync::atomic::Ordering::Relaxed);
            let bytes = total_bytes_indexed.load(std::sync::atomic::Ordering::Relaxed);

            println!("\n  ┌─────────────────────────────────────┐");
            println!("  │ {} RESULTS ({} circuits):", preset_name, circuits);
            println!("  │  Files discovered:    {}", files);
            println!("  │  Folders discovered:  {}", folders);
            println!("  │  Total entries:       {}", files + folders);
            println!("  │  Data indexed:        {:.2} GB", bytes as f64 / 1_073_741_824.0);
            println!("  │  HTTP fetches:        {}", fetches);
            println!("  │  Failures:            {}", failures);
            println!("  │  Success rate:        {:.1}%", if fetches > 0 { ((fetches - failures) as f64 / fetches as f64) * 100.0 } else { 0.0 });
            println!("  └─────────────────────────────────────┘\n");

            all_results.push((preset_name, circuits, files, folders, bytes, fetches, failures));

            // Brief cooldown between presets
            if circuits < 64 {
                println!("  [Cooldown 10s before next preset...]\n");
                tokio::time::sleep(Duration::from_secs(10)).await;
            }
        }

        println!("\n╔══════════════════════════════════════════════════════════════════╗");
        println!("║                    COMPARISON SUMMARY                           ║");
        println!("╠══════════╤═════════╤═══════╤════════╤══════════╤════════╤════════╣");
        println!("║ Preset   │Circuits │ Files │Folders │ GB Index │Fetches │ Fails  ║");
        println!("╠══════════╪═════════╪═══════╪════════╪══════════╪════════╪════════╣");
        for (name, circ, files, folders, bytes, fetches, fails) in &all_results {
            println!("║ {:<8} │ {:>7} │ {:>5} │ {:>6} │ {:>8.2} │ {:>6} │ {:>6} ║",
                name, circ, files, folders, *bytes as f64 / 1_073_741_824.0, fetches, fails);
        }
        println!("╚══════════╧═════════╧═══════╧════════╧══════════╧════════╧════════╝");

        println!("\n=== Benchmark Complete ===");
    });
}
