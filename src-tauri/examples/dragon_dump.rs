use std::time::Duration;

#[tokio::main]
async fn main() {
    let app = tauri::Builder::default()
        .build(tauri::generate_context!())
        .expect("build tauri app");

    crawli_lib::tor::cleanup_stale_tor_daemons();

    println!("Bootstrapping tor...");
    let (swarm_guard, ports) = crawli_lib::tor::bootstrap_tor_cluster(app.handle().clone(), 1).await.unwrap();

    let proxy = reqwest::Proxy::all(format!("socks5h://127.0.0.1:{}", ports[0])).unwrap();
    let client = reqwest::Client::builder()
        .proxy(proxy)
        .timeout(Duration::from_secs(60))
        .build()
        .unwrap();

    let url = "http://dragonforxxbp3awc7mzs5dkswrua3znqyx5roefmi4smjrsdi22xwqd.onion/www.rjzavoral.com";
    println!("Fetching {} via port {}...", url, ports[0]);

    match client.get(url).send().await {
        Ok(resp) => {
            println!("Status: {}", resp.status());
            if let Ok(text) = resp.text().await {
                std::fs::write("/tmp/dragon_dump.html", text).unwrap();
                println!("Saved to /tmp/dragon_dump.html");
            }
        }
        Err(e) => {
            println!("Error: {}", e);
        }
    }
    drop(swarm_guard);
}
