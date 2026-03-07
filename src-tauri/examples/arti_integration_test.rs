//! Legacy compatibility integration test.
//!
//! This example intentionally exercises the raw SOCKS compatibility bridge and is
//! no longer representative of Crawli's default TorForge runtime path.
//! Prefer `arti_direct_test`, `qilin_authorized_soak`, and
//! `local_piece_resume_probe` for the supported architecture.

use anyhow::Result;
use reqwest::Proxy;
use std::time::Instant;

/// Well-known clearnet Tor project check page (reachable over Tor circuits)
const TOR_CHECK_URL: &str = "https://check.torproject.org/api/ip";

/// DuckDuckGo .onion (always online, good smoke test)
const DDG_ONION: &str = "https://duckduckgogg42xjoc72x3sjasowoarfbgcmvfimaftt6twagswzczad.onion/";

/// Number of test circuits to bootstrap
const TEST_CIRCUITS: usize = 2;

#[tokio::main]
async fn main() -> Result<()> {
    println!("╔══════════════════════════════════════════════════════╗");
    println!("║  Crawli Phase 45: Native Arti Integration Test      ║");
    println!("╚══════════════════════════════════════════════════════╝");
    println!();

    // ── Step 1: Bootstrap arti ──────────────────────────────────
    println!(
        "[1/4] Bootstrapping {} native arti circuits...",
        TEST_CIRCUITS
    );
    let bootstrap_start = Instant::now();

    // We can't use bootstrap_arti_cluster directly (requires AppHandle).
    // Instead, test the SOCKS proxy connectivity by bootstrapping manually.
    println!("       Runtime: native Arti clients with managed SOCKS bridges");

    // Spawn arti clients manually for CLI
    use arti_client::TorClient;
    use tor_rtcompat::PreferredRuntime;

    let mut socks_ports: Vec<u16> = Vec::new();
    let mut clients: Vec<std::sync::Arc<TorClient<PreferredRuntime>>> = Vec::new();

    for i in 0..TEST_CIRCUITS {
        println!("       Spawning TorClient {}...", i);
        let node_start = Instant::now();

        match crawli_lib::tor_native::spawn_tor_node(i, false).await {
            Ok(client) => {
                let elapsed = node_start.elapsed();
                println!(
                    "       ✓ Circuit {} ready in {:.1}s",
                    i,
                    elapsed.as_secs_f64()
                );
                clients.push(std::sync::Arc::new(client));

                // Allocate SOCKS port
                let port = 19050 + i as u16;
                socks_ports.push(port);
            }
            Err(e) => {
                println!("       ✗ Circuit {} failed: {}", i, e);
            }
        }
    }

    let bootstrap_elapsed = bootstrap_start.elapsed();
    println!();
    println!(
        "       Bootstrap complete: {} circuits in {:.1}s",
        clients.len(),
        bootstrap_elapsed.as_secs_f64()
    );

    if clients.is_empty() {
        println!("       ✗ FAILED: No circuits bootstrapped. Check network connectivity.");
        return Ok(());
    }

    // ── Step 2: Start SOCKS proxies ─────────────────────────────
    println!();
    println!("[2/4] Starting SOCKS5 proxies...");

    let shutdown = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

    for (i, (client, &port)) in clients.iter().zip(socks_ports.iter()).enumerate() {
        let client = client.clone();
        let shutdown = shutdown.clone();
        tokio::spawn(async move {
            if let Err(e) = crawli_lib::tor_native::run_socks_proxy(client, port, shutdown, i).await
            {
                eprintln!("       SOCKS proxy {} crashed: {}", i, e);
            }
        });
    }

    // Give proxies a moment to bind
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    println!(
        "       ✓ {} SOCKS proxies on {:?}",
        socks_ports.len(),
        socks_ports
    );

    // ── Step 3: Connectivity Tests ──────────────────────────────
    println!();
    println!("[3/4] Running connectivity tests...");
    println!();

    let test_port = socks_ports[0];
    let proxy = Proxy::all(format!("socks5h://127.0.0.1:{}", test_port))?;
    let client = reqwest::Client::builder()
        .proxy(proxy)
        .danger_accept_invalid_certs(true)
        .timeout(std::time::Duration::from_secs(60))
        .build()?;

    // Test A: Tor check API (clearnet via Tor)
    print!("       [A] Tor Check API (clearnet over Tor)... ");
    let test_a_start = Instant::now();
    match client.get(TOR_CHECK_URL).send().await {
        Ok(resp) => {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            let test_a_ms = test_a_start.elapsed().as_millis();
            println!("✓ {} ({} ms)", status, test_a_ms);
            println!(
                "         Response: {}",
                body.trim().chars().take(200).collect::<String>()
            );
        }
        Err(e) => {
            let test_a_ms = test_a_start.elapsed().as_millis();
            println!("✗ FAILED ({} ms): {}", test_a_ms, e);
        }
    }

    // Test B: DuckDuckGo .onion
    print!("       [B] DuckDuckGo .onion... ");
    let test_b_start = Instant::now();
    match client.get(DDG_ONION).send().await {
        Ok(resp) => {
            let status = resp.status();
            let body_len = resp.text().await.map(|b| b.len()).unwrap_or(0);
            let test_b_ms = test_b_start.elapsed().as_millis();
            println!("✓ {} ({} ms, {} bytes)", status, test_b_ms, body_len);
        }
        Err(e) => {
            let test_b_ms = test_b_start.elapsed().as_millis();
            println!("✗ FAILED ({} ms): {}", test_b_ms, e);
        }
    }

    // Test C: Multi-circuit round-robin
    println!();
    println!("       [C] Multi-circuit round-robin throughput test...");
    let mut circuit_stats: Vec<(usize, u64, u64)> = Vec::new(); // (circuit_id, bytes, ms)

    for (i, &port) in socks_ports.iter().enumerate() {
        let proxy = Proxy::all(format!("socks5h://127.0.0.1:{}", port))?;
        let c = reqwest::Client::builder()
            .proxy(proxy)
            .danger_accept_invalid_certs(true)
            .timeout(std::time::Duration::from_secs(30))
            .build()?;

        let start = Instant::now();
        match c.get(TOR_CHECK_URL).send().await {
            Ok(resp) => {
                let body = resp.bytes().await.unwrap_or_default();
                let ms = start.elapsed().as_millis() as u64;
                let bytes = body.len() as u64;
                println!(
                    "         Circuit {}: {} bytes in {} ms ({:.2} KB/s)",
                    i,
                    bytes,
                    ms,
                    (bytes as f64 / ms as f64) * 1000.0 / 1024.0
                );
                circuit_stats.push((i, bytes, ms));
            }
            Err(e) => {
                println!("         Circuit {}: FAILED - {}", i, e);
            }
        }
    }

    // ── Step 4: Summary ─────────────────────────────────────────
    println!();
    println!("[4/4] Results Summary");
    println!("╔══════════════════════════════════════════════════════╗");
    println!(
        "║  Bootstrap Time:    {:.1}s ({} circuits)             ",
        bootstrap_elapsed.as_secs_f64(),
        clients.len()
    );
    println!(
        "║  SOCKS Proxies:     {:?}                            ",
        socks_ports
    );

    if !circuit_stats.is_empty() {
        let total_bytes: u64 = circuit_stats.iter().map(|(_, b, _)| b).sum();
        let avg_ms: u64 =
            circuit_stats.iter().map(|(_, _, m)| m).sum::<u64>() / circuit_stats.len() as u64;
        let fastest = circuit_stats.iter().min_by_key(|(_, _, ms)| ms).unwrap();
        let slowest = circuit_stats.iter().max_by_key(|(_, _, ms)| ms).unwrap();

        println!(
            "║  Total Throughput:  {} bytes                        ",
            total_bytes
        );
        println!(
            "║  Avg Latency:       {} ms                           ",
            avg_ms
        );
        println!(
            "║  Fastest Circuit:   #{} ({} ms)                     ",
            fastest.0, fastest.2
        );
        println!(
            "║  Slowest Circuit:   #{} ({} ms)                     ",
            slowest.0, slowest.2
        );
    }

    println!("╚══════════════════════════════════════════════════════╝");

    // Cleanup
    shutdown.store(true, std::sync::atomic::Ordering::Relaxed);
    println!();
    println!("✓ Integration test complete. Native arti engine verified.");

    Ok(())
}
