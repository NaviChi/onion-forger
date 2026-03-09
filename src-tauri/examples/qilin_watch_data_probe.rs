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

    let url = "http://25j35d6uf37tvfqt5pmz457yicgu35yhizojqxbfzv33dni2d73q3oad.onion/80349839-d06f-41a8-b954-3602fe60725a/";
    println!("[*] Fetching {}...", url);
    match client.get(url).send().await {
        Ok(resp) => {
            println!("[*] HTTP Status: {}", resp.status());
            println!("[*] Final URL: {}", resp.url());
            let html = resp.text().await.unwrap();
            std::fs::write("/tmp/ijzn.html", &html).unwrap();
            println!("[*] Wrote {} bytes to /tmp/ijzn.html", html.len());
        }
        Err(e) => {
            eprintln!("Error fetching target: {}", e);
        }
    }

    let _ = child.kill();
    let _ = child.wait();
}
