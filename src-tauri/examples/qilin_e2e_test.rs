// examples/qilin_e2e_test.rs
// Phase 62: Full end-to-end test mirroring the EXACT GUI code path
// fingerprint → adapter match → discovery → directory listing → file extraction
//
// Run: cargo run --example qilin_e2e_test 2>&1

use anyhow::Result;
use crawli_lib::adapters::{self, qilin_nodes::QilinNodeCache, CrawlerAdapter};
use crawli_lib::arti_client::ArtiClient;
use crawli_lib::tor_native::spawn_tor_node;
use std::time::Instant;

#[tokio::main]
async fn main() -> Result<()> {
    // Standalone binaries need explicit CryptoProvider
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        rustls::crypto::CryptoProvider::install_default(rustls::crypto::ring::default_provider())
            .expect("Failed to install ring CryptoProvider");
    }

    let url = "http://ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion/site/view?uuid=c9d2ba19-6aa1-3087-8773-f63d023179ed";
    let uuid = "c9d2ba19-6aa1-3087-8773-f63d023179ed";

    println!("╔══════════════════════════════════════════════╗");
    println!("║   Qilin Full End-to-End Test (GUI mirror)   ║");
    println!("╚══════════════════════════════════════════════╝");
    println!("Target: {}\n", url);

    let total_start = Instant::now();

    // ═══ STAGE 1: Tor Bootstrap ═══
    println!("─── STAGE 1: Tor Bootstrap ───");
    let s1 = Instant::now();
    let tor_client = spawn_tor_node(0, false).await?;
    let client = ArtiClient::new(tor_client, Some(arti_client::IsolationToken::new()));
    println!("  ✅ Tor ready in {:.1}s\n", s1.elapsed().as_secs_f64());

    // ═══ STAGE 2: Fingerprint Probe (matches lib.rs:attempt_crawl) ═══
    println!("─── STAGE 2: Fingerprint Probe ───");
    println!("  (This is the EXACT same flow as lib.rs:attempt_crawl)");
    let s2 = Instant::now();
    let mut body = String::new();
    let mut status = 0u16;
    let max_attempts = 4;
    for attempt in 1..=max_attempts {
        print!("  Attempt {}/{}... ", attempt, max_attempts);
        match tokio::time::timeout(std::time::Duration::from_secs(30), client.get(url).send()).await
        {
            Ok(Ok(resp)) => {
                status = resp.status().as_u16();
                let final_url = resp.url().as_str().to_string();
                println!("✅ status={} final_url={}", status, final_url);
                match tokio::time::timeout(std::time::Duration::from_secs(15), resp.text()).await {
                    Ok(Ok(text)) => {
                        println!("  Body: {} bytes", text.len());
                        body = text;
                    }
                    Ok(Err(e)) => println!("  ❌ Body decode error: {}", e),
                    Err(_) => println!("  ⏰ Body read timeout (15s)"),
                }
                break;
            }
            Ok(Err(e)) => {
                println!("❌ {}", e);
                if attempt < max_attempts {
                    tokio::time::sleep(std::time::Duration::from_millis(attempt as u64 * 750))
                        .await;
                }
            }
            Err(_) => {
                println!("⏰ TIMEOUT (30s)");
                if attempt < max_attempts {
                    tokio::time::sleep(std::time::Duration::from_millis(attempt as u64 * 750))
                        .await;
                }
            }
        }
    }
    println!("  Fingerprint done in {:.1}s\n", s2.elapsed().as_secs_f64());

    if body.is_empty() {
        println!("❌ FATAL: All fingerprint probes failed. Cannot continue.");
        return Ok(());
    }

    // ═══ STAGE 3: Adapter Matching (matches lib.rs:determine_adapter) ═══
    println!("─── STAGE 3: Adapter Matching ───");
    let fingerprint = adapters::SiteFingerprint {
        url: url.to_string(), // Original URL, NOT final redirect
        status,
        headers: reqwest::header::HeaderMap::new(),
        body: body.clone(),
    };

    let registry = adapters::AdapterRegistry::new();
    let adapter = registry.determine_adapter(&fingerprint).await;
    match &adapter {
        Some(a) => println!("  ✅ Adapter matched: {}", a.name()),
        None => println!("  ❌ NO ADAPTER MATCHED — this is the bug!"),
    }

    // Debug: show what was checked
    println!("  Debug checks:");
    println!(
        "    URL contains /site/view: {}",
        url.contains("/site/view")
    );
    println!(
        "    URL contains /site/data: {}",
        url.contains("/site/data")
    );
    println!(
        "    Body contains QData: {}",
        body.contains("<div class=\"page-header-title\">QData</div>")
    );
    println!(
        "    Body contains Data browser: {}",
        body.contains("Data browser")
    );
    println!(
        "    Body contains table#list: {}",
        body.contains("<table id=\"list\">")
    );
    println!(
        "    Body contains td.link: {}",
        body.contains("<td class=\"link\">")
    );
    println!("    Body first 200 chars: {}", &body[..body.len().min(200)]);
    println!();

    if adapter.is_none() {
        println!("❌ Cannot continue without adapter match.");
        return Ok(());
    }

    // ═══ STAGE 4: Storage Node Discovery ═══
    println!("─── STAGE 4: Storage Node Discovery (45s timeout) ───");
    let s4 = Instant::now();
    let node_cache = QilinNodeCache::default();
    let _ = node_cache.initialize().await;
    node_cache.seed_known_mirrors(uuid).await;

    let storage_url = match tokio::time::timeout(
        std::time::Duration::from_secs(45),
        node_cache.discover_and_resolve(url, uuid, &client, None),
    )
    .await
    {
        Ok(Some(node)) => {
            println!(
                "  ✅ Resolved: {} ({}ms) in {:.1}s",
                node.host,
                node.avg_latency_ms,
                s4.elapsed().as_secs_f64()
            );
            node.url
        }
        Ok(None) => {
            println!("  ⚠ No node resolved. Trying fallback mirrors...");
            // Try direct mirror
            let fallback = format!(
                "http://7mnkv5nvnjyifezlfyba6gek7aeimg5eghej5vp65qxnb2hjbtlttlyd.onion/{}/",
                uuid
            );
            println!("  Using fallback: {}", fallback);
            fallback
        }
        Err(_) => {
            println!("  ⏰ Discovery TIMED OUT after 45s. Using fallback.");
            format!(
                "http://7mnkv5nvnjyifezlfyba6gek7aeimg5eghej5vp65qxnb2hjbtlttlyd.onion/{}/",
                uuid
            )
        }
    };
    println!();

    // ═══ STAGE 5: Crawl Root Directory ═══
    println!("─── STAGE 5: Crawl Root Directory ───");
    let s5 = Instant::now();
    print!("  Fetching {}... ", storage_url);
    match tokio::time::timeout(
        std::time::Duration::from_secs(20),
        client.get(&storage_url).send(),
    )
    .await
    {
        Ok(Ok(resp)) => {
            println!("status={}", resp.status());
            match tokio::time::timeout(std::time::Duration::from_secs(15), resp.text()).await {
                Ok(Ok(html)) => {
                    println!("  Body: {} bytes", html.len());

                    // Parse file listing (same regex as Qilin adapter)
                    let v3_row_re = regex::Regex::new(
                        r#"<td class="link"><a href="([^"]+)"[^>]*>.*?</a></td><td class="size">([^<]*)</td>"#
                    ).unwrap();

                    let mut files = Vec::new();
                    let mut dirs = Vec::new();
                    for cap in v3_row_re.captures_iter(&html) {
                        if let (Some(href), Some(size)) = (cap.get(1), cap.get(2)) {
                            let h = href.as_str();
                            if h == "../" || h == "/" || h.starts_with('?') {
                                continue;
                            }
                            if h.ends_with('/') {
                                dirs.push(h.to_string());
                            } else {
                                files.push((h.to_string(), size.as_str().to_string()));
                            }
                        }
                    }

                    println!("\n  ╔══ CRAWL RESULTS ══╗");
                    println!("  ║ Directories: {:>4} ║", dirs.len());
                    println!("  ║ Files:       {:>4} ║", files.len());
                    println!("  ╚══════════════════╝");

                    if !dirs.is_empty() {
                        println!("\n  📁 Directories (first 10):");
                        for d in dirs.iter().take(10) {
                            println!("    └── {}", d);
                        }
                    }
                    if !files.is_empty() {
                        println!("\n  📄 Files (first 10):");
                        for (f, s) in files.iter().take(10) {
                            println!("    └── {} ({})", f, s);
                        }
                    }
                    if files.is_empty() && dirs.is_empty() {
                        println!("\n  ⚠ No entries parsed. Body preview:");
                        println!("    {}", &html[..html.len().min(500)]);
                    }
                }
                Ok(Err(e)) => println!("  ❌ Body error: {}", e),
                Err(_) => println!("  ⏰ Body timeout (15s)"),
            }
        }
        Ok(Err(e)) => println!("❌ Request failed: {}", e),
        Err(_) => println!("⏰ TIMEOUT (20s)"),
    }

    let total = total_start.elapsed();
    println!("\n╔═══════════════════════════════════════════╗");
    println!(
        "║  TOTAL TIME: {:.1}s ({}m {}s)              ║",
        total.as_secs_f64(),
        total.as_secs() / 60,
        total.as_secs() % 60
    );
    println!("╚═══════════════════════════════════════════╝");
    Ok(())
}
