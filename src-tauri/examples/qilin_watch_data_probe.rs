use reqwest::{Client, Proxy};
use std::time::Duration;

#[tokio::main]
async fn main() {
    println!("[*] Booting Tor daemon on 9060...");
    let mut child = std::process::Command::new("tor")
        .arg("--SocksPort")
        .arg("9060")
        .arg("--DataDirectory")
        .arg("/tmp/tor_watch_data")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();

    tokio::time::sleep(Duration::from_secs(15)).await;

    let proxy = Proxy::all("socks5h://127.0.0.1:9060").unwrap();
    let client = Client::builder()
        .proxy(proxy)
        .timeout(Duration::from_secs(60))
        .build()
        .unwrap();

    let url = "http://tdb5vowurjqlvgkdntq6vfv3epahdkqdhf7hqvq43hr4zb6bovpsk2yd.onion/d273fe69-4d65-4ca1-97f0-189aa4bb4741/";
    println!("[*] Fetching {}...", url);
    match client.get(url).send().await {
        Ok(resp) => {
            println!("[*] HTTP Status: {}", resp.status());
            println!("[*] Final URL: {}", resp.url());
            let html = resp.text().await.unwrap();
            println!(
                "[*] Fetched {} bytes. Snippet:\n{}",
                html.len(),
                &html[..std::cmp::min(html.len(), 500)]
            );
        }
        Err(e) => {
            eprintln!("Error fetching target: {}", e);
        }
    }

    let _ = child.kill();
    let _ = child.wait();
}
