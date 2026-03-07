//! Phase 45: Direct Arti Connect Test (no SOCKS proxy layer)
//!
//! Tests TorClient::connect() directly to isolate whether the issue is
//! in our SOCKS proxy or in arti's circuit builder.
//!
//! Usage: cargo run --example arti_direct_test

use anyhow::Result;
use std::time::Instant;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::main]
async fn main() -> Result<()> {
    // Enable tracing to see arti's internal logs
    // tracing_subscriber::fmt()
    //     .with_env_filter("debug,arti_client=trace,tor_circmgr=trace,tor_chanmgr=trace,tor_guardmgr=trace,tor_proto=trace")
    //     .init();

    println!("╔══════════════════════════════════════════════════════╗");
    println!("║  Crawli: Direct Arti Connect Test (no SOCKS)        ║");
    println!("╚══════════════════════════════════════════════════════╝");
    println!("Runtime profile: torforge");
    println!();

    // Bootstrap one client with default config
    println!("[1/3] Bootstrapping arti client...");
    let bootstrap_start = Instant::now();

    let state_dir = std::env::temp_dir().join("arti_test_isolation");
    let _ = std::fs::remove_dir_all(&state_dir); // Clear previous

    let mut builder = arti_client::TorClientConfig::builder();
    builder
        .storage()
        .cache_dir(arti_client::config::CfgPath::new(
            state_dir.join("cache").to_string_lossy().into_owned(),
        ))
        .state_dir(arti_client::config::CfgPath::new(
            state_dir.join("state").to_string_lossy().into_owned(),
        ));
    builder.address_filter().allow_onion_addrs(true);
    let config = builder.build().unwrap();

    let client = arti_client::TorClient::create_bootstrapped(config).await?;
    println!(
        "       ✓ Bootstrapped in {:.1}s",
        bootstrap_start.elapsed().as_secs_f64()
    );

    // Test 1: Direct connect to httpbin.org (clearnet)
    println!();
    println!("[2/3] Direct connect test → httpbin.org:80 (clearnet over Tor)...");
    let connect_start = Instant::now();

    match client.connect(("httpbin.org", 80)).await {
        Ok(mut stream) => {
            let connect_ms = connect_start.elapsed().as_millis();
            println!("       ✓ Connected in {} ms", connect_ms);

            // Send HTTP GET request
            stream
                .write_all(b"GET /ip HTTP/1.1\r\nHost: httpbin.org\r\nConnection: close\r\n\r\n")
                .await?;

            let mut response = Vec::new();
            stream.read_to_end(&mut response).await?;
            let total_ms = connect_start.elapsed().as_millis();

            let body = String::from_utf8_lossy(&response);
            println!(
                "       Response ({} bytes in {} ms):",
                response.len(),
                total_ms
            );
            // Print first 500 chars
            for line in body.lines().take(10) {
                println!("         {}", line);
            }
        }
        Err(e) => {
            println!(
                "       ✗ FAILED ({} ms): {}",
                connect_start.elapsed().as_millis(),
                e
            );
        }
    }

    // Test 2: Direct connect to check.torproject.org:443 (HTTPS clearnet over Tor)
    println!();
    println!("[3/3] Direct connect test → check.torproject.org:443 (HTTPS clearnet)...");
    let connect_start2 = Instant::now();

    match client.connect(("check.torproject.org", 443)).await {
        Ok(_stream) => {
            let connect_ms = connect_start2.elapsed().as_millis();
            println!(
                "       ✓ TCP connected in {} ms (TLS handshake would follow)",
                connect_ms
            );
        }
        Err(e) => {
            println!(
                "       ✗ FAILED ({} ms): {}",
                connect_start2.elapsed().as_millis(),
                e
            );
        }
    }

    println!();
    println!("✓ Direct arti connect test complete.");
    Ok(())
}
