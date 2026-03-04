use std::time::Duration;

#[tokio::main]
async fn main() {
    let app = tauri::Builder::default()
        .build(tauri::generate_context!())
        .expect("build tauri app");

    crawli_lib::tor::cleanup_stale_tor_daemons();

    let (swarm_guard, ports) = crawli_lib::tor::bootstrap_tor_cluster(app.handle().clone(), 1)
        .await
        .unwrap();

    let proxy = reqwest::Proxy::all(format!("socks5h://127.0.0.1:{}", ports[0])).unwrap();
    let client = reqwest::Client::builder()
        .proxy(proxy)
        .timeout(Duration::from_secs(120))
        .build()
        .unwrap();

    let target_url = "http://ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion/site/view?uuid=30111f69-5882-3535-8484-77a751b3c1c5";

    for attempt in 1..=5 {
        println!("Fetching Qilin (Attempt {}): {}", attempt, target_url);

        match client.get(target_url).send().await {
            Ok(resp) => {
                let status = resp.status();
                println!("Status: {}", status);
                if let Ok(text) = resp.text().await {
                    std::fs::write("/tmp/qilin_dump.html", &text).unwrap();
                    println!("Saved HTML to /tmp/qilin_dump.html");
                    let preview = text.chars().take(800).collect::<String>();
                    println!("{}\n[SUCCESS] Extracted payload.", preview);
                    break;
                }
            }
            Err(e) => {
                println!("Error: {}", e);
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    }

    drop(swarm_guard);
}
