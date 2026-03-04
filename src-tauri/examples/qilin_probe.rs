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

    let target_url = "http://a7r2n577n6jqzqexu5an3j2aej3ezb4klm7pkbp44243cqbwi43brjid.onion/72a4c05f-f711-498a-a038-758efa78aa09/usa%20medica/MEGA%20uploads/";

    for attempt in 1..=5 {
        println!("Fetching {}...", target_url);

        match client.get(target_url).send().await {
            Ok(resp) => {
                if let Ok(text) = resp.text().await {
                    let parsed = crawli_lib::adapters::autoindex::parse_autoindex_html(&text);
                    println!("Found {} entries in MEGA uploads:", parsed.len());
                    for (name, size, is_dir) in parsed.iter().take(20) {
                        println!(" - [Dir={}] Name: {}", is_dir, name);
                    }
                    if parsed.len() > 100 {
                        println!("... and {} more entries.", parsed.len() - 20);
                    }
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
