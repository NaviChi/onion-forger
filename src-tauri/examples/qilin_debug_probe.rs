use std::time::Duration;

#[tokio::main]
async fn main() {
    let app = tauri::Builder::default()
        .build(tauri::generate_context!())
        .expect("build tauri app");

    crawli_lib::tor::cleanup_stale_tor_daemons();

    let (swarm_guard, ports) = crawli_lib::tor::bootstrap_tor_cluster(app.handle().clone(), 2)
        .await
        .unwrap();

    let proxy = reqwest::Proxy::all(format!("socks5h://127.0.0.1:{}", ports[0])).unwrap();
    let client = reqwest::Client::builder()
        .proxy(proxy)
        .timeout(Duration::from_secs(120))
        .build()
        .unwrap();

    let target_url = "http://a7r2n577n6jqzqexu5an3j2aej3ezb4klm7pkbp44243cqbwi43brjid.onion/72a4c05f-f711-498a-a038-758efa78aa09/";

    let mut success = false;
    for attempt in 1..=10 {
        println!("Fetching root URL (Attempt {}/10)...", attempt);
        match client.get(target_url).send().await {
            Ok(resp) => {
                println!("Response Status: {}", resp.status());
                if let Ok(text) = resp.text().await {
                    println!("Response length: {}", text.len());
                    println!("Contains `<table id=\"list\">`: {}", text.contains("<table id=\"list\">"));
                    println!("Contains `Data browser`: {}", text.contains("Data browser"));
                    let preview = text.chars().take(500).collect::<String>();
                    println!("Preview:\n{}", preview);
                    success = true;
                    break;
                }
            }
            Err(e) => {
                println!("Error on attempt {}: {}", attempt, e);
                tokio::time::sleep(Duration::from_secs(10)).await;
            }
        }
    }

    if !success {
        println!("FATAL: Failed to connect to the Qilin root URL after 10 retries. The server is heavily throttling or offline.");
    }

    drop(swarm_guard);
}
