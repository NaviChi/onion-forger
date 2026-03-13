/// Phase 34: Headless CLI Crawl Test
/// 
/// This test crawls the Qilin site WITHOUT the Tauri runtime.
/// It bootstraps Tor, crawls the target, and prints benchmark results.
/// 
/// Usage: cargo run --example qilin_cli_test --release 2>&1 | tee crawl_results.log

use crawli_lib::adapters::qilin::QilinAdapter;
use crawli_lib::adapters::{CrawlerAdapter, SiteFingerprint};
use crawli_lib::frontier::CrawlOptions;
use std::time::Instant;

#[tokio::main]
async fn main() {
    println!("╔══════════════════════════════════════════════════════════╗");
    println!("║  QILIN CRAWLER — PHASE 34 HEADLESS BENCHMARK           ║");
    println!("╚══════════════════════════════════════════════════════════╝");
    println!();

    let target = "http://ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion/site/view?uuid=c9d2ba19-6aa1-3087-8773-f63d023179ed";

    // ── Phase 1: Bootstrap Tor ──────────────────────────────────────
    println!("[1/4] Cleaning stale Tor daemons...");
    crawli_lib::tor::cleanup_stale_tor_daemons();

    println!("[1/4] Bootstrapping Tor (headless mode)...");

    let bootstrap_start = Instant::now();

    let _opts = CrawlOptions {
        listing: true,
        sizes: true,
        download: false,
        circuits: Some(24),
        agnostic_state: false,
        resume: false,
        resume_index: None,
        stealth_ramp: true,
        parallel_download: false,
        force_clearnet: false,
        mega_password: None,
        download_mode: crawli_lib::frontier::DownloadMode::Medium,
    };

    // Use manual SOCKS5 proxy setup — check CLI arg, env var, or common ports
    let socks_port: u16 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .or_else(|| std::env::var("TOR_SOCKS_PORT").ok().and_then(|s| s.parse().ok()))
        .unwrap_or(9050);
    println!("[1/4] Attempting connection via SOCKS5 proxy at 127.0.0.1:{}", socks_port);

    let proxy_url = format!("socks5h://127.0.0.1:{}", socks_port);
    let client = match reqwest::Client::builder()
        .proxy(reqwest::Proxy::all(&proxy_url).expect("proxy URL"))
        .timeout(std::time::Duration::from_secs(120))
        .connect_timeout(std::time::Duration::from_secs(15))
        .pool_max_idle_per_host(4)
        .tcp_nodelay(true)
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("FATAL: Failed to create HTTP client: {}", e);
            std::process::exit(1);
        }
    };

    let bootstrap_elapsed = bootstrap_start.elapsed();
    println!("[1/4] ✅ Client ready in {:.1}s", bootstrap_elapsed.as_secs_f64());

    // ── Phase 2: Fingerprint ────────────────────────────────────────
    println!();
    println!("[2/4] Fetching site fingerprint...");
    let fp_start = Instant::now();

    let resp = match client.get(target).send().await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("FATAL: Cannot reach target: {}", e);
            eprintln!("       Is Tor running? Is the .onion site up?");
            std::process::exit(1);
        }
    };

    let status = resp.status().as_u16();
    let headers = resp.headers().clone();
    let body = resp.text().await.unwrap_or_else(|_| "[DECODE_ERROR]".to_string());
    let fp_elapsed = fp_start.elapsed();

    println!("[2/4] ✅ Fingerprint acquired in {:.1}s (status={})", fp_elapsed.as_secs_f64(), status);
    println!("       Body length: {} bytes", body.len());

    let fingerprint = SiteFingerprint {
        url: target.to_string(),
        status,
        headers,
        body,
    };

    // ── Phase 3: Adapter Match ──────────────────────────────────────
    println!();
    println!("[3/4] Testing QilinAdapter.can_handle()...");
    let adapter = QilinAdapter::default();
    let can_handle = adapter.can_handle(&fingerprint).await;
    println!("[3/4] QilinAdapter match: {}", if can_handle { "✅ YES" } else { "❌ NO" });

    if !can_handle {
        eprintln!("FATAL: QilinAdapter did not match this site.");
        eprintln!("       Body preview: {}", &fingerprint.body[..fingerprint.body.len().min(500)]);
        std::process::exit(1);
    }

    // ── Phase 4: Manual Crawl ───────────────────────────────────────
    println!();
    println!("[4/4] Starting manual crawl...");
    println!("       Target: {}", target);
    println!("       Mode: listing=true, sizes=true, download=false");
    println!();

    // Step 1: Node discovery (skipped in headless SOCKS5 mode — requires ArtiClient)
    println!("[DISCOVERY] Skipping multi-node discovery (requires Arti runtime).");
    println!("[DISCOVERY] Using direct target URL for manual crawl.");
    let discovery_start = Instant::now();
    let uuid = "c9d2ba19-6aa1-3087-8773-f63d023179ed";
    let best_node: Option<crawli_lib::adapters::qilin_nodes::StorageNode> = None;
    let discovery_elapsed = discovery_start.elapsed();

    // Step 2: Crawl the resolved storage node
    let crawl_url = best_node
        .as_ref()
        .map(|n| n.url.clone())
        .unwrap_or_else(|| target.to_string());

    println!();
    println!("[CRAWL] Crawling: {}", crawl_url);
    println!("[CRAWL] Fetching root listing...");

    let mut total_entries = 0usize;
    let mut total_files = 0usize;
    let mut total_dirs = 0usize;
    let mut total_size: u64 = 0;
    let mut total_requests = 0usize;
    let mut failed_requests = 0usize;
    let mut queue: Vec<String> = vec![crawl_url.clone()];
    let mut visited: std::collections::HashSet<String> = std::collections::HashSet::new();
    visited.insert(crawl_url.clone());

    let crawl_loop_start = Instant::now();

    while let Some(url) = queue.pop() {
        total_requests += 1;

        // Fetch the page
        let page_start = Instant::now();
        let page_resp = match tokio::time::timeout(
            std::time::Duration::from_secs(30),
            client.get(&url).send()
        ).await {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => {
                failed_requests += 1;
                eprintln!("[CRAWL] ❌ Failed: {} — {}", url, e);
                continue;
            }
            Err(_) => {
                failed_requests += 1;
                eprintln!("[CRAWL] ⏰ Timeout: {}", url);
                continue;
            }
        };

        if !page_resp.status().is_success() {
            failed_requests += 1;
            eprintln!("[CRAWL] ❌ HTTP {}: {}", page_resp.status(), url);
            continue;
        }

        let body_bytes = match page_resp.bytes().await {
            Ok(b) => b,
            Err(e) => {
                failed_requests += 1;
                eprintln!("[CRAWL] ❌ Body read failed: {} — {}", url, e);
                continue;
            }
        };
        let page_elapsed = page_start.elapsed();
        let html = String::from_utf8_lossy(&body_bytes);

        // Parse HTML table rows using byte scanning (no memchr dependency)
        let html_bytes = html.as_bytes();
        let target_href = b"<td class=\"link\"><a href=\"";
        let target_size = b"</a></td><td class=\"size\">";

        let mut pos = 0;
        while pos < html_bytes.len().saturating_sub(target_href.len()) {
            // Find next href marker
            let remaining = &html_bytes[pos..];
            let href_pos = remaining.windows(target_href.len()).position(|w| w == target_href);
            let offset = match href_pos {
                Some(o) => o,
                None => break,
            };

            let start_idx = pos + offset + target_href.len();
            pos = start_idx;

            // Find closing quote
            let href_end_idx = match html_bytes[start_idx..].iter().position(|&b| b == b'"') {
                Some(o) => start_idx + o,
                None => continue,
            };

            let href_str = match std::str::from_utf8(&html_bytes[start_idx..href_end_idx]) {
                Ok(s) => s,
                Err(_) => continue,
            };

            if href_str == "../" || href_str == "/" || href_str.starts_with('?') {
                continue;
            }

            let is_dir = href_str.ends_with('/');

            // Extract size
            let size_search_start = href_end_idx;
            let size_remaining = &html_bytes[size_search_start..];
            if let Some(size_offset) = size_remaining.windows(target_size.len()).position(|w| w == target_size) {
                let size_start_idx = size_search_start + size_offset + target_size.len();
                if let Some(size_end_offset) = html_bytes[size_start_idx..].iter().position(|&b| b == b'<') {
                    let size_end_idx = size_start_idx + size_end_offset;
                    if let Ok(size_str) = std::str::from_utf8(&html_bytes[size_start_idx..size_end_idx]) {
                        total_entries += 1;

                        if is_dir {
                            total_dirs += 1;
                            let child_url = format!("{}/{}", url.trim_end_matches('/'), href_str);
                            if visited.insert(child_url.clone()) {
                                queue.push(child_url);
                            }
                        } else {
                            total_files += 1;
                            let raw_size = size_str.trim();
                            if raw_size != "-" {
                                if let Some(parsed) = parse_size_str(raw_size) {
                                    total_size += parsed;
                                }
                            }
                        }
                    }
                }
            }
        }

        let elapsed_ms = page_elapsed.as_millis();
        let rate = if crawl_loop_start.elapsed().as_secs_f64() > 0.0 {
            total_entries as f64 / crawl_loop_start.elapsed().as_secs_f64()
        } else {
            0.0
        };

        println!(
            "[CRAWL] {:>4} entries | {:>4} files | {:>4} dirs | {:>10} | {:>5}ms | {:.1} e/s | Q:{} | {}",
            total_entries,
            total_files,
            total_dirs,
            format_bytes(total_size),
            elapsed_ms,
            rate,
            queue.len(),
            url.split('/').last().unwrap_or("")
        );
    }

    let crawl_elapsed = crawl_loop_start.elapsed();
    let total_elapsed = Instant::now().duration_since(bootstrap_start);

    // ── Results ─────────────────────────────────────────────────────
    println!();
    println!("╔══════════════════════════════════════════════════════════╗");
    println!("║  BENCHMARK RESULTS                                     ║");
    println!("╠══════════════════════════════════════════════════════════╣");
    println!("║  Target UUID:    {}  ║", uuid);
    println!("╠══════════════════════════════════════════════════════════╣");
    println!("║  Discovery:      {:>8.1}s                              ║", discovery_elapsed.as_secs_f64());
    println!("║  Crawl Duration: {:>8.1}s                              ║", crawl_elapsed.as_secs_f64());
    println!("║  Total Duration: {:>8.1}s                              ║", total_elapsed.as_secs_f64());
    println!("╠══════════════════════════════════════════════════════════╣");
    println!("║  Total Entries:   {:>8}                               ║", total_entries);
    println!("║  Files:           {:>8}                               ║", total_files);
    println!("║  Directories:     {:>8}                               ║", total_dirs);
    println!("║  Total Size:      {:>12}                           ║", format_bytes(total_size));
    println!("╠══════════════════════════════════════════════════════════╣");
    println!("║  HTTP Requests:   {:>8}                               ║", total_requests);
    println!("║  Failed Requests: {:>8}                               ║", failed_requests);
    println!("║  Success Rate:    {:>7.1}%                              ║", 
        if total_requests > 0 { (total_requests - failed_requests) as f64 / total_requests as f64 * 100.0 } else { 0.0 });
    println!("║  Crawl Rate:      {:>7.1} entries/sec                   ║",
        if crawl_elapsed.as_secs_f64() > 0.0 { total_entries as f64 / crawl_elapsed.as_secs_f64() } else { 0.0 });
    println!("╚══════════════════════════════════════════════════════════╝");
}

fn parse_size_str(s: &str) -> Option<u64> {
    let s = s.trim();
    if s == "-" || s.is_empty() {
        return None;
    }
    
    // Try to parse as plain number first (bytes)
    if let Ok(n) = s.parse::<u64>() {
        return Some(n);
    }

    // Parse human-readable sizes like "1.5 GB", "234 MB", "12 KB"
    let s_upper = s.to_uppercase();
    let multiplier = if s_upper.ends_with("GB") || s_upper.ends_with("G") {
        1_073_741_824u64
    } else if s_upper.ends_with("MB") || s_upper.ends_with("M") {
        1_048_576u64
    } else if s_upper.ends_with("KB") || s_upper.ends_with("K") {
        1_024u64
    } else if s_upper.ends_with('B') {
        1u64
    } else {
        return s.parse::<u64>().ok();
    };

    let num_str = s_upper
        .trim_end_matches("GB")
        .trim_end_matches("MB")
        .trim_end_matches("KB")
        .trim_end_matches('G')
        .trim_end_matches('M')
        .trim_end_matches('K')
        .trim_end_matches('B')
        .trim();

    num_str.parse::<f64>().ok().map(|n| (n * multiplier as f64) as u64)
}

fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    const TB: u64 = 1024 * GB;

    if bytes >= TB {
        format!("{:.2} TB", bytes as f64 / TB as f64)
    } else if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}
