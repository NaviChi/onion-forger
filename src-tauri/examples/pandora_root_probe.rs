//! Phase 77D: Pandora Root Discovery + Multi-Victim UUID Probe (SOCKS version)
//!
//! Uses arti SOCKS proxy + reqwest for proper HTTP handling.
//!
//! Usage: cargo run --example pandora_root_probe

use anyhow::Result;
use std::sync::Arc;
use std::time::Instant;

const PANDORA: &str = "pandora42btuwlldza4uthk4bssbtsv47y4t5at5mo4ke3h4nqveobyd.onion";
const CMS: &str = "ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion";

#[tokio::main]
async fn main() -> Result<()> {
    println!("╔══════════════════════════════════════════════════════╗");
    println!("║  Phase 77D: Pandora Root + UUID Probe (SOCKS)       ║");
    println!("╚══════════════════════════════════════════════════════╝");
    println!();

    // Bootstrap arti + SOCKS proxy
    println!("[BOOT] Bootstrapping arti client + SOCKS proxy...");
    let boot_start = Instant::now();

    use crawli_lib::tor_native::{run_socks_proxy, spawn_tor_node};

    let tor_client = spawn_tor_node(0, false).await?;
    let proxy_port = 19060_u16;
    let client_arc = Arc::new(tor_client.clone());
    let is_running = Arc::new(std::sync::atomic::AtomicBool::new(false));

    tokio::spawn({
        let c = client_arc.clone();
        let r = is_running.clone();
        async move {
            let _ = run_socks_proxy(c, proxy_port, r, 0).await;
        }
    });

    // Wait for SOCKS proxy
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    let http = reqwest::Client::builder()
        .proxy(reqwest::Proxy::all(format!(
            "socks5h://127.0.0.1:{}",
            proxy_port
        ))?)
        .timeout(std::time::Duration::from_secs(45))
        .redirect(reqwest::redirect::Policy::none()) // Don't follow redirects
        .build()?;

    println!(
        "[BOOT] ✓ Ready in {:.1}s\n",
        boot_start.elapsed().as_secs_f64()
    );

    // ═══════════════════════════════════════════════════
    // PHASE 1: Probe pandora root /
    // ═══════════════════════════════════════════════════
    println!("═══════════════════════════════════════════════════");
    println!("  PHASE 1: Probe pandora42btu root /");
    println!("═══════════════════════════════════════════════════");

    let root_url = format!("http://{}/", PANDORA);
    let start = Instant::now();
    match http.get(&root_url).send().await {
        Ok(resp) => {
            let status = resp.status();
            let headers = resp.headers().clone();
            let body = resp.text().await.unwrap_or_default();
            println!(
                "  ✅ Root: {} ({} bytes, {:.1}s)",
                status,
                body.len(),
                start.elapsed().as_secs_f64()
            );

            // Log headers
            println!("  Headers:");
            for (k, v) in headers.iter().take(10) {
                println!("    {}: {}", k, v.to_str().unwrap_or("?"));
            }

            std::fs::write("/tmp/pandora_root.html", &body)?;
            println!("  💾 Saved to /tmp/pandora_root.html");

            // Preview
            let preview = if body.len() > 2000 {
                &body[..2000]
            } else {
                &body
            };
            println!("\n  ── HTML Preview ──");
            for line in preview.lines().take(60) {
                println!("  │ {}", line);
            }
            println!("  ── End Preview ──\n");

            // Search for UUID dirs
            let uuid_re = regex::Regex::new(
                r#"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}"#,
            )
            .unwrap();
            let found: Vec<&str> = uuid_re.find_iter(&body).map(|m| m.as_str()).collect();
            if !found.is_empty() {
                println!("  🎯 Found {} UUID references:", found.len());
                for u in &found {
                    println!("      {}", u);
                }
            }

            // Search for any href links
            let href_re = regex::Regex::new(r#"href="([^"]+)""#).unwrap();
            let links: Vec<&str> = href_re
                .captures_iter(&body)
                .map(|c| c.get(1).unwrap().as_str())
                .collect();
            if !links.is_empty() {
                println!("  🔗 Found {} links:", links.len());
                for l in links.iter().take(30) {
                    println!("      {}", l);
                }
            }
        }
        Err(e) => {
            println!(
                "  ❌ Root probe failed ({:.1}s): {}",
                start.elapsed().as_secs_f64(),
                e
            );
        }
    }

    // ═══════════════════════════════════════════════════
    // PHASE 2: Scrape CMS for victim UUIDs
    // ═══════════════════════════════════════════════════
    println!("\n═══════════════════════════════════════════════════");
    println!("  PHASE 2: Scrape CMS main page for victim UUIDs");
    println!("═══════════════════════════════════════════════════");

    let cms_url = format!("http://{}/", CMS);
    let mut victim_uuids: Vec<String> = Vec::new();

    let start = Instant::now();
    match http.get(&cms_url).send().await {
        Ok(resp) => {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            println!(
                "  ✅ CMS: {} ({} bytes, {:.1}s)",
                status,
                body.len(),
                start.elapsed().as_secs_f64()
            );

            std::fs::write("/tmp/qilin_cms_main.html", &body)?;
            println!("  💾 Saved to /tmp/qilin_cms_main.html");

            // Extract UUIDs
            let uuid_re = regex::Regex::new(
                r#"(?:site/view|site/data)\?uuid=([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})"#
            ).unwrap();

            for cap in uuid_re.captures_iter(&body) {
                let uuid = cap[1].to_string();
                if !victim_uuids.contains(&uuid) {
                    victim_uuids.push(uuid);
                }
            }

            println!("  🎯 Found {} unique victim UUIDs", victim_uuids.len());
            for (i, uuid) in victim_uuids.iter().enumerate() {
                println!("      {}. {}", i + 1, uuid);
            }
        }
        Err(e) => {
            println!(
                "  ❌ CMS scrape failed ({:.1}s): {}",
                start.elapsed().as_secs_f64(),
                e
            );
        }
    }

    // If CMS main page didn't have UUIDs, try the blog listing
    if victim_uuids.is_empty() {
        println!("  Trying alternate paths...");
        for path in &["/site/index", "/blog", "/blog/index", "/index"] {
            let url = format!("http://{}{}", CMS, path);
            let start = Instant::now();
            if let Ok(resp) = http.get(&url).send().await {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                println!(
                    "  {} → {} ({} bytes, {:.1}s)",
                    path,
                    status,
                    body.len(),
                    start.elapsed().as_secs_f64()
                );

                let uuid_re = regex::Regex::new(
                    r#"(?:site/view|site/data)\?uuid=([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})"#
                ).unwrap();
                for cap in uuid_re.captures_iter(&body) {
                    let uuid = cap[1].to_string();
                    if !victim_uuids.contains(&uuid) {
                        victim_uuids.push(uuid);
                    }
                }

                if !victim_uuids.is_empty() {
                    std::fs::write("/tmp/qilin_cms_alternate.html", &body)?;
                    println!("  🎯 Found {} UUIDs from {}", victim_uuids.len(), path);
                    break;
                }
            }
        }
    }

    // ═══════════════════════════════════════════════════
    // PHASE 3: Test UUIDs against pandora
    // ═══════════════════════════════════════════════════
    println!("\n═══════════════════════════════════════════════════");
    println!("  PHASE 3: Test victim UUIDs against pandora42btu");
    println!("═══════════════════════════════════════════════════");

    let test_uuids: Vec<&String> = victim_uuids
        .iter()
        .filter(|u| *u != "f0668431-ee3f-3570-99cb-ea7d9c0691c6")
        .take(10)
        .collect();

    if test_uuids.is_empty() && victim_uuids.is_empty() {
        println!("  ⚠ No UUIDs available. Testing with known UUID...");
        // Test the known UUID just to verify pandora connectivity
        let url = format!("http://{}/f0668431-ee3f-3570-99cb-ea7d9c0691c6/", PANDORA);
        let start = Instant::now();
        match http.get(&url).send().await {
            Ok(resp) => println!(
                "  Known UUID → {} ({:.1}s)",
                resp.status(),
                start.elapsed().as_secs_f64()
            ),
            Err(e) => println!(
                "  Known UUID → ❌ ({:.1}s): {}",
                start.elapsed().as_secs_f64(),
                e
            ),
        }
    } else {
        println!("  Testing {} UUIDs...\n", test_uuids.len());

        for (i, uuid) in test_uuids.iter().enumerate() {
            let url = format!("http://{}/{}/", PANDORA, uuid);
            let start = Instant::now();

            match tokio::time::timeout(std::time::Duration::from_secs(30), http.get(&url).send())
                .await
            {
                Ok(Ok(resp)) => {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    let elapsed = start.elapsed().as_secs_f64();

                    let tag = if status.is_success()
                        && (body.contains("<table")
                            || body.contains("Index of")
                            || body.contains("File Name"))
                    {
                        "🎯 AUTOINDEX!"
                    } else if status.as_u16() == 404 {
                        "❌ 404"
                    } else if status.as_u16() == 403 {
                        "🔒 403"
                    } else {
                        "⚠ Other"
                    };

                    println!(
                        "  [{:2}/{}] {} → {} {} ({} bytes, {:.1}s)",
                        i + 1,
                        test_uuids.len(),
                        &uuid[..8],
                        status,
                        tag,
                        body.len(),
                        elapsed
                    );

                    if body.contains("<table") || body.contains("Index of") {
                        let save = format!("/tmp/pandora_listing_{}.html", &uuid[..8]);
                        std::fs::write(&save, &body)?;
                        println!("         💾 LISTING SAVED: {}", save);
                        // Print first 500 chars
                        let p = if body.len() > 500 {
                            &body[..500]
                        } else {
                            &body
                        };
                        for line in p.lines().take(10) {
                            println!("         │ {}", line);
                        }
                    }
                }
                Ok(Err(e)) => {
                    println!(
                        "  [{:2}/{}] {} → ❌ Error ({:.1}s): {}",
                        i + 1,
                        test_uuids.len(),
                        &uuid[..8],
                        start.elapsed().as_secs_f64(),
                        e
                    );
                }
                Err(_) => {
                    println!(
                        "  [{:2}/{}] {} → ⏰ Timeout",
                        i + 1,
                        test_uuids.len(),
                        &uuid[..8]
                    );
                }
            }

            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    }

    println!("\n✓ Phase 77D probe complete.");
    Ok(())
}
