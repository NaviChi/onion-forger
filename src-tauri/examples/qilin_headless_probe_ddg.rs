use reqwest::{Client, Proxy};
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=======================================================");
    println!("🔍 INITIATING DUCKDUCKGO TOR CONTROL PROBE 🔍");
    println!("Target: http://duckduckgogg42xjoc72x3sjiapvqvkxg225qyr5y2p2t4fud5hfd.onion");
    println!("=======================================================\n");

    let mut child = std::process::Command::new("tor")
        .arg("--SocksPort")
        .arg("9058")
        .arg("--DataDirectory")
        .arg("/tmp/tor_headless_probe_ddg")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;

    println!("    ✅ Tor daemon spawned on SOCKS 9058. Waiting 15s for bootstrap...");
    tokio::time::sleep(Duration::from_secs(15)).await;

    let proxy_url = "socks5h://127.0.0.1:9058";
    let proxy = Proxy::all(proxy_url)?;
    let client = Client::builder()
        .proxy(proxy)
        .timeout(Duration::from_secs(45))
        .danger_accept_invalid_certs(true)
        .build()?;

    let target_url = "http://duckduckgogg42xjoc72x3sjiapvqvkxg225qyr5y2p2t4fud5hfd.onion";

    println!("\n[1] Fetching root UI application...");
    let start = std::time::Instant::now();
    match client.get(target_url).send().await {
        Ok(resp) => {
            if let Ok(text) = resp.text().await {
                println!(
                    "    ✅ Fetched {} bytes of HTML in {:?}",
                    text.len(),
                    start.elapsed()
                );
            } else {
                println!("    ⚠️ Failed to read text body.");
            }
        }
        Err(e) => {
            println!("    ❌ Failed to reach DuckDuckGo ONION: {}", e);
        }
    }

    let _ = child.kill();
    let _ = child.wait();
    Ok(())
}
