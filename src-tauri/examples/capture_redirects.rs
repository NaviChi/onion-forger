//! Phase 77D part 2: Capture CMS 302 redirect Location headers
//! to discover which storage nodes are assigned per victim UUID.
//!
//! We disable redirect following to capture the raw Location header.
//!
//! Usage: cargo run --example capture_redirects

use anyhow::Result;
use std::sync::Arc;
use std::time::Instant;

const CMS: &str = "ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion";

#[tokio::main]
async fn main() -> Result<()> {
    println!("╔══════════════════════════════════════════════════════╗");
    println!("║  Phase 77D-2: CMS 302 Redirect Location Capture    ║");
    println!("╚══════════════════════════════════════════════════════╝\n");

    // Bootstrap
    println!("[BOOT] Bootstrapping...");
    let boot_start = Instant::now();

    use crawli_lib::tor_native::{run_socks_proxy, spawn_tor_node};

    let tor_client = spawn_tor_node(0, false).await?;
    let proxy_port = 19061_u16;
    let client_arc = Arc::new(tor_client.clone());
    let is_running = Arc::new(std::sync::atomic::AtomicBool::new(false));

    tokio::spawn({
        let c = client_arc.clone();
        let r = is_running.clone();
        async move {
            let _ = run_socks_proxy(c, proxy_port, r, 0).await;
        }
    });

    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    // Build HTTP client WITHOUT redirect following
    let http = reqwest::Client::builder()
        .proxy(reqwest::Proxy::all(format!(
            "socks5h://127.0.0.1:{}",
            proxy_port
        ))?)
        .timeout(std::time::Duration::from_secs(45))
        .redirect(reqwest::redirect::Policy::none()) // CRITICAL: capture raw 302
        .build()?;

    println!(
        "[BOOT] ✓ Ready in {:.1}s\n",
        boot_start.elapsed().as_secs_f64()
    );

    // Get victim UUIDs from CMS
    println!("─── Scraping CMS for victim UUIDs ───\n");
    let cms_url = format!("http://{}/", CMS);
    let resp = http.get(&cms_url).send().await?;
    let body = resp.text().await?;

    let uuid_re = regex::Regex::new(
        r#"site/(?:view|data)\?uuid=([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})"#
    ).unwrap();

    let mut uuids: Vec<String> = Vec::new();
    for cap in uuid_re.captures_iter(&body) {
        let uuid = cap[1].to_string();
        if !uuids.contains(&uuid) {
            uuids.push(uuid);
        }
    }
    println!("Found {} victim UUIDs\n", uuids.len());

    // For each UUID, hit /site/data?uuid= and capture the 302 Location header
    println!("─── Capturing 302 redirect targets ───\n");
    println!(
        "{:<40} {:<8} {:<60}",
        "UUID", "STATUS", "REDIRECT TARGET (Location header)"
    );
    println!("{}", "─".repeat(110));

    let mut storage_nodes: Vec<String> = Vec::new();

    for uuid in uuids.iter().take(20) {
        let data_url = format!("http://{}/site/data?uuid={}", CMS, uuid);
        let start = Instant::now();

        match tokio::time::timeout(
            std::time::Duration::from_secs(30),
            http.get(&data_url).send(),
        )
        .await
        {
            Ok(Ok(resp)) => {
                let status = resp.status();
                let location = resp
                    .headers()
                    .get("location")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("(none)")
                    .to_string();
                let elapsed = start.elapsed().as_secs_f64();

                println!(
                    "{} {:>3} {:<60} ({:.1}s)",
                    &uuid[..36],
                    status.as_u16(),
                    location,
                    elapsed
                );

                // Extract storage onion from Location header
                let onion_re = regex::Regex::new(r#"http://([a-z2-7]{56}\.onion)"#).unwrap();
                if let Some(cap) = onion_re.captures(&location) {
                    let node = cap[1].to_string();
                    if !storage_nodes.contains(&node) {
                        storage_nodes.push(node);
                    }
                }
            }
            Ok(Err(e)) => {
                println!("{} ERR {}", &uuid[..36], e);
            }
            Err(_) => {
                println!("{} TMO (30s)", &uuid[..36]);
            }
        }

        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    println!("\n─── Discovered Storage Nodes (from 302 Location) ───\n");
    if storage_nodes.is_empty() {
        println!("  ⚠ No storage node addresses found in redirects.");
        println!("    This means the CMS may be doing server-side proxy");
        println!("    instead of 302, or using JavaScript redirects.");
    } else {
        println!("  Found {} unique storage nodes:", storage_nodes.len());
        for (i, node) in storage_nodes.iter().enumerate() {
            println!("      {}. {}", i + 1, node);
        }
    }

    println!("\n✓ Redirect capture complete.");
    Ok(())
}
