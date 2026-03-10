// examples/qilin_probe_test.rs
// Phase 62: Live CLI probe test — exercises the FULL timeout chain
// Run: cargo run --example qilin_probe_test 2>&1

use anyhow::Result;
use crawli_lib::adapters::qilin_nodes::QilinNodeCache;
use crawli_lib::arti_client::ArtiClient;
use crawli_lib::tor_native::spawn_tor_node;
use std::time::Instant;

#[tokio::main]
async fn main() -> Result<()> {
    let target = "http://ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion/site/view?uuid=c9d2ba19-6aa1-3087-8773-f63d023179ed";
    let uuid = "c9d2ba19-6aa1-3087-8773-f63d023179ed";

    // Standalone binaries need explicit CryptoProvider (Tauri normally handles this)
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        rustls::crypto::CryptoProvider::install_default(rustls::crypto::ring::default_provider())
            .expect("Failed to install ring CryptoProvider");
    }

    println!("=== Qilin Live Probe Test ===");
    println!("Target: {}", target);
    println!();

    // Stage 1: Bootstrap Tor
    println!("[1/4] Bootstrapping Tor client...");
    let tor_start = Instant::now();
    let tor_client = spawn_tor_node(0, false).await?;
    let client = ArtiClient::new(tor_client, Some(arti_client::IsolationToken::new()));
    println!(
        "[1/4] ✅ Tor ready in {:.1}s\n",
        tor_start.elapsed().as_secs_f64()
    );

    // Stage 2: Fingerprint probe (the call that WAS missing timeout)
    println!("[2/4] Fingerprint probe (30s timeout per attempt, up to 4 attempts)...");
    let mut fingerprint_ok = false;
    let mut response_body = String::new();
    for attempt in 1..=4 {
        let fp_start = Instant::now();
        print!("  Attempt {}/4... ", attempt);
        match tokio::time::timeout(
            std::time::Duration::from_secs(30),
            client.get(target).send(),
        )
        .await
        {
            Ok(Ok(resp)) => {
                println!(
                    "✅ status={} ({:.1}s)",
                    resp.status(),
                    fp_start.elapsed().as_secs_f64()
                );

                // Body read with timeout
                print!("[3/4] Reading body (15s timeout)... ");
                let body_start = Instant::now();
                match tokio::time::timeout(std::time::Duration::from_secs(15), resp.text()).await {
                    Ok(Ok(body)) => {
                        println!(
                            "✅ {} bytes ({:.1}s)",
                            body.len(),
                            body_start.elapsed().as_secs_f64()
                        );
                        response_body = body;
                        fingerprint_ok = true;
                    }
                    Ok(Err(e)) => println!("❌ decode error: {}", e),
                    Err(_) => println!("⏰ TIMED OUT after 15s"),
                }
                break;
            }
            Ok(Err(e)) => {
                println!("❌ error: {} ({:.1}s)", e, fp_start.elapsed().as_secs_f64());
            }
            Err(_) => {
                println!("⏰ TIMED OUT after 30s");
            }
        }
        if attempt < 4 {
            let sleep_ms = attempt as u64 * 750;
            println!("  Sleeping {}ms before retry...", sleep_ms);
            tokio::time::sleep(std::time::Duration::from_millis(sleep_ms)).await;
        }
    }

    if fingerprint_ok {
        // Check adapter match
        let is_qilin = response_body.contains("<div class=\"page-header-title\">QData</div>")
            || response_body.contains("Data browser")
            || response_body.contains("_csrf-blog")
            || response_body.contains("item_box_photos");
        println!("\n  Qilin adapter match: {}", is_qilin);
        println!(
            "  Body first 300 chars: {}",
            &response_body[..response_body.len().min(300)]
        );
    } else {
        println!(
            "\n[2/4] ❌ ALL fingerprint probes failed. This is the EXACT behavior the GUI sees."
        );
        println!(
            "       Previously this would hang forever. Now it fails after 4×30s = 120s max.\n"
        );
    }

    // Stage 4: Storage node discovery
    println!("\n[4/4] Storage node discovery (45s timeout)...");
    let disco_start = Instant::now();
    let node_cache = QilinNodeCache::default();
    let _ = node_cache.initialize().await;
    node_cache.seed_known_mirrors(uuid).await;

    match tokio::time::timeout(
        std::time::Duration::from_secs(45),
        node_cache.discover_and_resolve(target, uuid, &client, None),
    )
    .await
    {
        Ok(Some(node)) => {
            println!(
                "[4/4] ✅ Resolved: {} ({}ms) in {:.1}s",
                node.host,
                node.avg_latency_ms,
                disco_start.elapsed().as_secs_f64()
            );
        }
        Ok(None) => {
            println!(
                "[4/4] ⚠ No node resolved in {:.1}s",
                disco_start.elapsed().as_secs_f64()
            );
        }
        Err(_) => {
            println!("[4/4] ⏰ Discovery TIMED OUT after 45s");
        }
    }

    let total = tor_start.elapsed();
    println!(
        "\n=== TOTAL: {:.1}s ({}m {}s) ===",
        total.as_secs_f64(),
        total.as_secs() / 60,
        total.as_secs() % 60
    );
    Ok(())
}
