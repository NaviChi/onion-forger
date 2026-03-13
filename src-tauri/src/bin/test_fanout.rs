/// Phase 138 Download Benchmark: Timed .onion download with fan-out circuits
/// Bootstraps Tor with isolation fan-out, crawls the Qilin site page, then
/// downloads files for the specified duration while reporting throughput.
///
/// Usage: cargo run --bin test_fanout --release [DURATION_SECS]
use anyhow::Result;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

static TOTAL_BYTES: AtomicU64 = AtomicU64::new(0);
static TOTAL_REQUESTS: AtomicUsize = AtomicUsize::new(0);
static FAILED_REQUESTS: AtomicUsize = AtomicUsize::new(0);

#[tokio::main(flavor = "multi_thread", worker_threads = 8)]
async fn main() -> Result<()> {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let duration_secs: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(180);

    let target_url = std::env::var("TEST_URL").unwrap_or_else(|_| {
        "http://ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion/site/view?uuid=2282dcf9-043f-3583-aadc-722db21e1cc1".to_string()
    });

    let fan_out: usize = std::env::var("CRAWLI_ISOLATION_FAN_OUT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(4);

    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║  Phase 138: .onion Parallel Download Benchmark              ║");
    println!("╚══════════════════════════════════════════════════════════════╝");
    println!("  URL:      {}...", &target_url[..target_url.len().min(60)]);
    println!("  Duration: {}s", duration_secs);
    println!("  Fan-out:  {}", fan_out);

    let profile = crawli_lib::resource_governor::detect_profile(None);
    println!("  CPU: {} cores | RAM: {:.1} GB | Arti cap: {}",
        profile.cpu_cores,
        profile.total_memory_bytes as f64 / (1024.0 * 1024.0 * 1024.0),
        profile.recommended_arti_cap);

    // ── Phase 1: Bootstrap ──────────────────────────────────────────
    println!("\n[Phase 1] Bootstrapping Tor circuits...");
    let boot_start = Instant::now();

    let num_base = profile.recommended_arti_cap.min(4).max(2);
    let mut base_clients = Vec::new();
    let mut join_set = tokio::task::JoinSet::new();

    for i in 0..num_base {
        join_set.spawn(async move {
            let start = Instant::now();
            let result = crawli_lib::tor_native::spawn_tor_node(i, false).await;
            (i, start.elapsed(), result)
        });
    }

    while let Some(result) = join_set.join_next().await {
        if let Ok((idx, elapsed, Ok(client))) = result {
            println!("  ✅ Base node {} ready in {:.1}s", idx, elapsed.as_secs_f64());
            base_clients.push(Arc::new(client));
        } else if let Ok((idx, elapsed, Err(e))) = result {
            println!("  ❌ Base node {} failed in {:.1}s: {}", idx, elapsed.as_secs_f64(), e);
        }
    }

    if base_clients.is_empty() {
        println!("FATAL: No Tor clients bootstrapped");
        return Ok(());
    }

    // Fan out
    let mut all_clients: Vec<Arc<arti_client::TorClient<tor_rtcompat::PreferredRuntime>>> = Vec::new();
    for base in &base_clients {
        all_clients.push(base.clone());
        for _ in 1..fan_out {
            all_clients.push(Arc::new(base.isolated_client()));
        }
    }

    let boot_elapsed = boot_start.elapsed();
    let total_slots = all_clients.len();
    println!("  {} base × {} fan-out = {} circuit slots in {:.1}s",
        base_clients.len(), fan_out, total_slots, boot_elapsed.as_secs_f64());

    // ── Phase 2: Fetch site page to discover files ──────────────────
    println!("\n[Phase 2] Fetching site to discover files...");
    let client0 = crawli_lib::arti_client::ArtiClient::new(
        (*all_clients[0]).clone(),
        Some(arti_client::IsolationToken::new()),
    );

    let page_start = Instant::now();
    let page_resp = match client0.get(&target_url).send().await {
        Ok(resp) => resp,
        Err(e) => {
            println!("  ❌ Failed to fetch site page: {}", e);
            run_page_throughput_test(&all_clients, &target_url, duration_secs).await;
            return Ok(());
        }
    };

    let status = page_resp.status();
    let body = page_resp.bytes().await.unwrap_or_default();
    println!("  Page fetched: HTTP {} ({} bytes) in {:.1}s",
        status, body.len(), page_start.elapsed().as_secs_f64());

    let body_str = String::from_utf8_lossy(&body);
    let file_urls = extract_file_urls(&body_str, &target_url);
    println!("  Discovered {} downloadable file URLs", file_urls.len());

    if file_urls.is_empty() {
        println!("  No files found, running page-level throughput test...");
        run_page_throughput_test(&all_clients, &target_url, duration_secs).await;
        print_final_report(total_slots, base_clients.len(), fan_out, boot_elapsed, Instant::now());
        return Ok(());
    }

    // ── Phase 3: Parallel download for N minutes ────────────────────
    println!("\n[Phase 3] Starting {}-second parallel download ({} circuit slots)...", duration_secs, total_slots);
    let deadline = Instant::now() + Duration::from_secs(duration_secs);
    let global_start = Instant::now();

    // Progress reporter
    let progress_handle = {
        let start = global_start;
        tokio::spawn(async move {
            let mut last_bytes = 0u64;
            loop {
                tokio::time::sleep(Duration::from_secs(15)).await;
                let elapsed = start.elapsed().as_secs_f64();
                let bytes_now = TOTAL_BYTES.load(Ordering::Relaxed);
                let reqs = TOTAL_REQUESTS.load(Ordering::Relaxed);
                let fails = FAILED_REQUESTS.load(Ordering::Relaxed);
                let delta_bytes = bytes_now.saturating_sub(last_bytes);
                let instant_speed = delta_bytes as f64 / (15.0 * 1024.0 * 1024.0);
                let avg_speed = bytes_now as f64 / (elapsed * 1024.0 * 1024.0);
                println!("  📊 [{:>4.0}s] {:>8.1} MB total | {:>6.3} MB/s avg | {:>6.3} MB/s burst | {} ok / {} fail",
                    elapsed, bytes_now as f64 / (1024.0 * 1024.0), avg_speed, instant_speed, reqs, fails);
                last_bytes = bytes_now;
            }
        })
    };

    // Spawn download workers — one per circuit slot
    let mut download_handles = Vec::new();
    for (slot_id, tor_client) in all_clients.iter().enumerate() {
        let urls = file_urls.clone();
        let client_arc = tor_client.clone();
        let dl_deadline = deadline;

        download_handles.push(tokio::spawn(async move {
            let mut url_idx = slot_id % urls.len();
            while Instant::now() < dl_deadline {
                let url = &urls[url_idx % urls.len()];
                let arti = crawli_lib::arti_client::ArtiClient::new(
                    (*client_arc).clone(),
                    Some(arti_client::IsolationToken::new()),
                );

                match tokio::time::timeout(Duration::from_secs(60), arti.get(url).send()).await {
                    Ok(Ok(resp)) => {
                        match resp.bytes().await {
                            Ok(bytes) => {
                                TOTAL_BYTES.fetch_add(bytes.len() as u64, Ordering::Relaxed);
                                TOTAL_REQUESTS.fetch_add(1, Ordering::Relaxed);
                            }
                            Err(_) => {
                                FAILED_REQUESTS.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    }
                    Ok(Err(_)) | Err(_) => {
                        FAILED_REQUESTS.fetch_add(1, Ordering::Relaxed);
                    }
                }
                url_idx += 1;
            }
        }));
    }

    for handle in download_handles {
        let _ = handle.await;
    }
    progress_handle.abort();

    print_final_report(total_slots, base_clients.len(), fan_out, boot_elapsed, global_start);

    Ok(())
}

fn print_final_report(total_slots: usize, base_count: usize, fan_out: usize, boot_elapsed: Duration, download_start: Instant) {
    let total_elapsed = download_start.elapsed();
    let total_bytes = TOTAL_BYTES.load(Ordering::Relaxed);
    let total_reqs = TOTAL_REQUESTS.load(Ordering::Relaxed);
    let total_fails = FAILED_REQUESTS.load(Ordering::Relaxed);

    let pid = sysinfo::Pid::from(std::process::id() as usize);
    let rss = {
        let mut s = sysinfo::System::new();
        s.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
        s.process(pid).map(|p| p.memory()).unwrap_or(0)
    };

    let mb = total_bytes as f64 / (1024.0 * 1024.0);
    let avg_speed = if total_elapsed.as_secs_f64() > 0.0 { mb / total_elapsed.as_secs_f64() } else { 0.0 };
    let success_rate = if total_reqs + total_fails > 0 {
        total_reqs as f64 / (total_reqs + total_fails) as f64 * 100.0
    } else { 0.0 };

    println!("\n╔══════════════════════════════════════════════════════════════╗");
    println!("║                    📊 BENCHMARK RESULTS                     ║");
    println!("╠══════════════════════════════════════════════════════════════╣");
    println!("║  Duration:          {:>8.1}s                                ║", total_elapsed.as_secs_f64());
    println!("║  Circuit slots:     {:>8}                                  ║", total_slots);
    println!("║  Base clients:      {:>8}                                  ║", base_count);
    println!("║  Fan-out ratio:     {:>8}                                  ║", fan_out);
    println!("║  Bootstrap time:    {:>8.1}s                                ║", boot_elapsed.as_secs_f64());
    println!("╠══════════════════════════════════════════════════════════════╣");
    println!("║  Total downloaded:  {:>8.2} MB                              ║", mb);
    println!("║  Avg speed:         {:>8.3} MB/s                            ║", avg_speed);
    println!("║  Requests OK:       {:>8}                                  ║", total_reqs);
    println!("║  Requests failed:   {:>8}                                  ║", total_fails);
    println!("║  Success rate:      {:>7.1}%                                ║", success_rate);
    println!("╠══════════════════════════════════════════════════════════════╣");
    println!("║  Process RSS:       {:>8.0} MB                              ║", rss as f64 / (1024.0 * 1024.0));
    println!("║  RSS per slot:      {:>8.1} MB                              ║", rss as f64 / (1024.0 * 1024.0) / total_slots as f64);
    println!("╚══════════════════════════════════════════════════════════════╝");
}

fn extract_file_urls(html: &str, base_url: &str) -> Vec<String> {
    let mut urls = Vec::new();
    let origin = if let Some(idx) = base_url.find(".onion") {
        &base_url[..idx + 6]
    } else {
        base_url
    };

    for segment in html.split("href=\"") {
        if let Some(end) = segment.find('"') {
            let href = &segment[..end];
            if href.contains("/download") || href.contains("/file")
                || href.ends_with(".pdf") || href.ends_with(".zip")
                || href.ends_with(".doc") || href.ends_with(".docx")
                || href.ends_with(".xls") || href.ends_with(".xlsx")
                || href.ends_with(".ppt") || href.ends_with(".pptx")
                || href.ends_with(".txt") || href.ends_with(".csv")
                || href.ends_with(".jpg") || href.ends_with(".png")
                || href.ends_with(".rar") || href.ends_with(".7z")
                || href.ends_with(".tar") || href.ends_with(".gz")
            {
                let full_url = if href.starts_with("http") {
                    href.to_string()
                } else if href.starts_with('/') {
                    format!("{}{}", origin, href)
                } else {
                    format!("{}/{}", base_url, href)
                };
                urls.push(full_url);
            }
        }
    }

    for segment in html.split("data-href=\"").chain(html.split("src=\"")) {
        if let Some(end) = segment.find('"') {
            let href = &segment[..end];
            if href.contains("/download") || href.contains("/file/get") {
                let full_url = if href.starts_with("http") {
                    href.to_string()
                } else if href.starts_with('/') {
                    format!("{}{}", origin, href)
                } else {
                    format!("{}/{}", base_url, href)
                };
                if !urls.contains(&full_url) {
                    urls.push(full_url);
                }
            }
        }
    }

    urls.sort();
    urls.dedup();
    urls
}

async fn run_page_throughput_test(
    clients: &[Arc<arti_client::TorClient<tor_rtcompat::PreferredRuntime>>],
    url: &str,
    duration_secs: u64,
) {
    println!("\n[Fallback] Repeated page-fetch throughput test for {}s...", duration_secs);
    let deadline = Instant::now() + Duration::from_secs(duration_secs);
    let start = Instant::now();
    let url = url.to_string();

    let progress = tokio::spawn({
        let s = start;
        async move {
            loop {
                tokio::time::sleep(Duration::from_secs(15)).await;
                let elapsed = s.elapsed().as_secs_f64();
                let bytes = TOTAL_BYTES.load(Ordering::Relaxed);
                let reqs = TOTAL_REQUESTS.load(Ordering::Relaxed);
                let fails = FAILED_REQUESTS.load(Ordering::Relaxed);
                println!("  📊 [{:>4.0}s] {:>8.1} MB | {:>6.3} MB/s | {} ok / {} fail",
                    elapsed, bytes as f64 / (1024.0 * 1024.0),
                    bytes as f64 / (elapsed * 1024.0 * 1024.0), reqs, fails);
            }
        }
    });

    let mut handles = Vec::new();
    for client in clients {
        let c = client.clone();
        let u = url.clone();
        let dl = deadline;
        handles.push(tokio::spawn(async move {
            while Instant::now() < dl {
                let arti = crawli_lib::arti_client::ArtiClient::new(
                    (*c).clone(),
                    Some(arti_client::IsolationToken::new()),
                );
                match tokio::time::timeout(Duration::from_secs(30), arti.get(&u).send()).await {
                    Ok(Ok(resp)) => {
                        match resp.bytes().await {
                            Ok(b) => {
                                TOTAL_BYTES.fetch_add(b.len() as u64, Ordering::Relaxed);
                                TOTAL_REQUESTS.fetch_add(1, Ordering::Relaxed);
                            }
                            Err(_) => { FAILED_REQUESTS.fetch_add(1, Ordering::Relaxed); }
                        }
                    }
                    _ => { FAILED_REQUESTS.fetch_add(1, Ordering::Relaxed); }
                }
            }
        }));
    }

    for h in handles { let _ = h.await; }
    progress.abort();
}
