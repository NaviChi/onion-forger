//! Phase 124 — 5-minute DragonForce benchmark (single Balanced preset, 8 circuits)
//! Comparable to Phase 120B (1,444 entries) and Phase 121 (3,518 entries) baselines.
use crawli_lib::adapters::dragonforce::parse_dragonforce_fsguest;
use crawli_lib::adapters::EntryType;
use crawli_lib::tor;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

const RUN_SECS: u64 = 300; // 5 minutes

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

        println!("\n╔═══════════════════════════════════════════════════════════╗");
        println!("║  Phase 124 DragonForce 5-Min Benchmark                    ║");
        println!("║  Baseline: Ph120B=1,444 | Ph121=3,518                     ║");
        println!("║  Changes: P0 ZeroAlloc | P1 AdaptiveTTFB | P2 SizeGate   ║");
        println!("║           P3 HTTP/2+UA | CUSUM ChangePoint               ║");
        println!("╚═══════════════════════════════════════════════════════════╝\n");

        // Bootstrap shared Tor swarm (8 clients)
        println!("[INIT] Bootstrapping Tor swarm (8 clients)...");
        let t0 = Instant::now();
        let (guard, _ports) = tor::bootstrap_tor_cluster_for_traffic(
            app_handle.clone(),
            8,
            0,
            tor::SwarmTrafficClass::OnionService,
        )
        .await
        .expect("Bootstrap failed");
        let bootstrap_secs = t0.elapsed().as_secs_f64();
        println!("[INIT] Swarm ready in {:.1}s\n", bootstrap_secs);

        let shared_clients = guard.get_arti_clients();

        // Pre-warm HS descriptor
        println!("[INIT] Pre-warming HS descriptor...");
        let warmup_tc = shared_clients[0].read().unwrap().clone();
        let warmup_client = crawli_lib::arti_client::ArtiClient::new((*warmup_tc).clone(), None);
        let mut hs_warm = false;
        for attempt in 0..5 {
            match tokio::time::timeout(Duration::from_secs(60), warmup_client.get(target).send()).await {
                Ok(Ok(r)) if r.status().is_success() => {
                    println!("[INIT] HS descriptor cached (attempt {})\n", attempt + 1);
                    hs_warm = true;
                    break;
                }
                Ok(Ok(r)) => {
                    println!("[INIT] Warmup attempt {} → status {}", attempt + 1, r.status());
                }
                Ok(Err(e)) => {
                    println!("[INIT] Warmup attempt {} → error: {}", attempt + 1, e);
                }
                Err(_) => {
                    println!("[INIT] Warmup attempt {} → timeout", attempt + 1);
                }
            }
            if attempt < 4 {
                tokio::time::sleep(Duration::from_secs(3)).await;
            }
        }
        if !hs_warm {
            println!("[INIT] HS warmup failed — proceeding anyway (workers will retry)\n");
        }

        let workers = 12usize; // Balanced preset
        let multi_clients = 8usize;

        // Build pool
        let seeded = crawli_lib::multi_client_pool::snapshot_seed_clients(&shared_clients, multi_clients);
        let pool = Arc::new(
            crawli_lib::multi_client_pool::MultiClientPool::new_seeded(multi_clients, seeded, None)
                .await
                .unwrap(),
        );

        let queue: Arc<tokio::sync::Mutex<Vec<String>>> =
            Arc::new(tokio::sync::Mutex::new(vec![target.to_string()]));
        let visited: Arc<tokio::sync::Mutex<HashSet<String>>> =
            Arc::new(tokio::sync::Mutex::new(HashSet::new()));
        visited.lock().await.insert(target.to_string());

        let total_files = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let total_folders = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let total_bytes_indexed = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let total_fetches = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let total_failures = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let total_pages_parsed = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let deadline = Instant::now() + Duration::from_secs(RUN_SECS);
        let crawl_start = Instant::now();

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
            let pages_c = total_pages_parsed.clone();

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

                    let client_idx = w % multi_clients;
                    let tc = pool_c.get_client(client_idx).await;
                    let client = crawli_lib::arti_client::ArtiClient::new((*tc).clone(), None);

                    let mut body_bytes_raw = bytes::Bytes::new();
                    let mut ok = false;
                    for retry in 0..5 {
                        if Instant::now() >= deadline { break; }
                        fetches_c.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let fetch_timeout = if retry == 0 { 45 } else { 30 };
                        match tokio::time::timeout(
                            Duration::from_secs(fetch_timeout),
                            client.get(&url).header("Connection", "keep-alive").send()
                        ).await {
                            Ok(Ok(resp)) if resp.status().is_success() => {
                                // Phase 124: Use bytes() not text()
                                if let Ok(Ok(b)) = tokio::time::timeout(
                                    Duration::from_secs(20),
                                    resp.bytes()
                                ).await {
                                    body_bytes_raw = b;
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

                    if !ok || body_bytes_raw.is_empty() { continue; }

                    let host = url::Url::parse(&url)
                        .ok()
                        .and_then(|u| u.host_str().map(|s| s.to_string()))
                        .unwrap_or_default();
                    let url_clone = url.clone();
                    let body_len = body_bytes_raw.len();

                    // Phase 124 P2: Size-gated decode
                    let entries = if body_len < 4096 {
                        let body = String::from_utf8_lossy(&body_bytes_raw);
                        parse_dragonforce_fsguest(&body, &host, &url_clone)
                    } else {
                        tokio::task::spawn_blocking(move || {
                            let body = String::from_utf8_lossy(&body_bytes_raw);
                            parse_dragonforce_fsguest(&body, &host, &url_clone)
                        }).await.unwrap_or_default()
                    };

                    pages_c.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

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

        // Progress ticker every 30s
        let files_tick = total_files.clone();
        let folders_tick = total_folders.clone();
        let fetches_tick = total_fetches.clone();
        let fails_tick = total_failures.clone();
        let bytes_tick = total_bytes_indexed.clone();
        let pages_tick = total_pages_parsed.clone();
        let ticker = tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            loop {
                interval.tick().await;
                if Instant::now() >= deadline { break; }
                let elapsed = crawl_start.elapsed().as_secs();
                let f = files_tick.load(std::sync::atomic::Ordering::Relaxed);
                let d = folders_tick.load(std::sync::atomic::Ordering::Relaxed);
                let req = fetches_tick.load(std::sync::atomic::Ordering::Relaxed);
                let fail = fails_tick.load(std::sync::atomic::Ordering::Relaxed);
                let b = bytes_tick.load(std::sync::atomic::Ordering::Relaxed);
                let p = pages_tick.load(std::sync::atomic::Ordering::Relaxed);
                let rate = if elapsed > 0 { (f + d) as f64 / elapsed as f64 } else { 0.0 };
                println!("  [{:>3}s] files={} folders={} pages={} fetches={} fails={} indexed={:.1}MB rate={:.1}/s",
                    elapsed, f, d, p, req, fail, b as f64 / 1_048_576.0, rate);
            }
        });

        for h in handles { let _ = h.await; }
        ticker.abort();

        let total_elapsed = crawl_start.elapsed();
        let files = total_files.load(std::sync::atomic::Ordering::Relaxed);
        let folders = total_folders.load(std::sync::atomic::Ordering::Relaxed);
        let fetches = total_fetches.load(std::sync::atomic::Ordering::Relaxed);
        let failures = total_failures.load(std::sync::atomic::Ordering::Relaxed);
        let bytes = total_bytes_indexed.load(std::sync::atomic::Ordering::Relaxed);
        let pages = total_pages_parsed.load(std::sync::atomic::Ordering::Relaxed);
        let entries = files + folders;
        let rate = entries as f64 / total_elapsed.as_secs_f64();
        let success_rate = if fetches > 0 { ((fetches - failures) as f64 / fetches as f64) * 100.0 } else { 0.0 };

        println!("\n╔═══════════════════════════════════════════════════════════╗");
        println!("║           PHASE 124 — 5 MINUTE BENCHMARK RESULTS          ║");
        println!("╠═══════════════════════════════════════════════════════════╣");
        println!("║  Duration:          {:.1}s", total_elapsed.as_secs_f64());
        println!("║  Bootstrap:         {:.1}s", bootstrap_secs);
        println!("║  Workers:           {}", workers);
        println!("║  ──────────────────────────────────────────────────────── ║");
        println!("║  Files discovered:  {}", files);
        println!("║  Folders discovered:{}", folders);
        println!("║  TOTAL ENTRIES:     {} ← compare: Ph120B=1444, Ph121=3518", entries);
        println!("║  Pages parsed:      {}", pages);
        println!("║  Data indexed:      {:.2} GB", bytes as f64 / 1_073_741_824.0);
        println!("║  ──────────────────────────────────────────────────────── ║");
        println!("║  HTTP fetches:      {}", fetches);
        println!("║  Failures:          {}", failures);
        println!("║  Success rate:      {:.1}%", success_rate);
        println!("║  Entries/sec:       {:.1}", rate);
        println!("║  ──────────────────────────────────────────────────────── ║");
        println!("║  COMPARISON:");
        println!("║    Phase 120B:  1,444 entries   (baseline)");
        println!("║    Phase 121:   3,518 entries   (2.4× vs 120B)");
        println!("║    Phase 124:   {} entries   ({:.1}× vs 120B)", entries, entries as f64 / 1444.0);
        println!("╚═══════════════════════════════════════════════════════════╝");
    });
}
