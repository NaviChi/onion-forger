use std::time::Duration;

#[tokio::main]
async fn main() {
    let app = tauri::Builder::default()
        .build(tauri::generate_context!())
        .expect("build tauri app");

    let (swarm_guard, ports) = crawli_lib::tor::bootstrap_tor_cluster(app.handle().clone(), 1)
        .await
        .unwrap();

    let proxy = reqwest::Proxy::all(format!("socks5h://127.0.0.1:{}", ports[0])).unwrap();
    let client = reqwest::Client::builder()
        .proxy(proxy)
        .timeout(Duration::from_secs(60))
        .build()
        .unwrap();

    let raw_html = std::fs::read_to_string("/tmp/dragon_dump.html").unwrap_or_default();
    let mut token_parsed = String::new();
    if let Some(start) = raw_html.find("?path=/&token=") {
        let after = &raw_html[start + 14..];
        if let Some(end) = after.find("\"") {
            token_parsed = after[..end].to_string();
        }
    }

    if token_parsed.is_empty() {
        println!("Error: Could not extract JWT token from iframe source");
        drop(swarm_guard);
        return;
    }

    // Based on standard React/SPA ransomware structures, the API backend usually sits at /api
    let api_url = "http://dragonforxxbp3awc7mzs5dkswrua3znqyx5roefmi4smjrsdi22xwqd.onion/api/files";

    let full_url = format!("{}?path=/", api_url);

    // Try sending Authorization header
    match client
        .get(&full_url)
        .header("Authorization", format!("Bearer {}", token_parsed))
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64)")
        .send()
        .await
    {
        Ok(resp) => {
            let status = resp.status();
            println!("Status 1 (Deploy-Uuid): {}", status);
            if let Ok(text) = resp.text().await {
                let output = text.to_string();
                std::fs::write("/tmp/dragon_api.json", &output).unwrap();
                println!("Saved API response to /tmp/dragon_api.json");
                println!("Payload: {}", output.chars().take(800).collect::<String>());
            }
        }
        Err(e) => {
            println!("Error: {}", e);
        }
    }
    drop(swarm_guard);
}
