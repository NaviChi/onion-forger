//! Legacy compatibility-only SOCKS probe.
//!
//! Not part of the default Crawli/TorForge architecture anymore.

use anyhow::Result;
use reqwest::Proxy;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().init();

    // We start one Tor node manually via the native engine
    // to verify the logic inside `src/tor_native.rs` works.

    // Create random temp directory to act as app state dir
    let temp_dir = std::env::temp_dir().join("test_socks_arti");
    std::fs::create_dir_all(&temp_dir)?;

    println!("Starting native tor proxy on port 9060...");
    // Let's call the `run_socks_proxy` internally or `start_tor_node`
    let proxy_port = 9060;

    // We can just spawn an arti client, and run run_socks_proxy
    // Since tor_native is largely pub(crate), we might need to access it differently,
    // or just use `tor_native::start_tor_engine`? Wait, `start_tor_engine` expects `tauri::AppHandle`.
    // Let's just import the internal stuff directly.
    use crawli_lib::tor_native::{run_socks_proxy, spawn_tor_node};

    println!("Bootstrapping node...");
    let is_running = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let tor_client: arti_client::TorClient<tor_rtcompat::PreferredRuntime> =
        spawn_tor_node(0, false).await?;
    println!("Node bootstrapped!");

    // Run proxy
    let client_clone = std::sync::Arc::new(tor_client.clone());
    tokio::spawn(async move {
        let _ = run_socks_proxy(client_clone, proxy_port, is_running, 0).await;
    });

    // Wait for the server to be ready
    tokio::time::sleep(Duration::from_secs(2)).await;

    println!("Connecting to proxy via reqwest...");
    let proxy = Proxy::all(format!("socks5h://127.0.0.1:{}", proxy_port))?;
    let client = reqwest::Client::builder()
        .proxy(proxy)
        .timeout(Duration::from_secs(30))
        .build()?;

    let res = client.get("https://httpbin.org/ip").send().await?;
    let text = res.text().await?;

    println!("SUCCESS! Payload:");
    println!("{}", text);

    Ok(())
}
