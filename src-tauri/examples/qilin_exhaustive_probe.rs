use crawli_lib::tor::TorProcessGuard;
use reqwest::{Client, Proxy};
use std::time::Duration;
use tokio::time::sleep;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n=======================================================");
    println!(" QILIN EXHAUSTIVE SUB-LINEAR FUZZING SUITE");
    println!(" Testing single-character wildcard extraction limits.");
    println!("=======================================================\n");

    let swarm_guard = TorProcessGuard::new();
    sleep(Duration::from_secs(4)).await;

    let proxy = Proxy::all("socks5h://127.0.0.1:9051")?;
    let client = Client::builder()
        .proxy(proxy)
        .danger_accept_invalid_certs(true)
        .timeout(Duration::from_secs(120))
        .connect_timeout(Duration::from_secs(45))
        .pool_max_idle_per_host(8)
        .tcp_nodelay(true)
        // Tor Browser Strict Header Profile
        .default_headers({
            let mut headers = reqwest::header::HeaderMap::new();
            headers.insert("User-Agent", "Mozilla/5.0 (Windows NT 10.0; rv:109.0) Gecko/20100101 Firefox/115.0".parse().unwrap());
            headers.insert("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8".parse().unwrap());
            headers.insert("Accept-Language", "en-US,en;q=0.5".parse().unwrap());
            headers.insert("Connection", "keep-alive".parse().unwrap());
            headers.insert("Upgrade-Insecure-Requests", "1".parse().unwrap());
            headers.insert("Sec-Fetch-Dest", "document".parse().unwrap());
            headers.insert("Sec-Fetch-Mode", "navigate".parse().unwrap());
            headers.insert("Sec-Fetch-Site", "none".parse().unwrap());
            headers.insert("Sec-Fetch-User", "?1".parse().unwrap());
            headers
        })
        .build()?;

    let host = "http://25mjg55vcbjzwykz2uqsvaw7hcevm4pqxl42o324zr6qf5zgddmghkqd.onion";
    let base_route = "/6a230577-89fc-4731-bec1-6fdfa81656fc";

    // Fuzz Matrix: Test if single character search strings yield exhaustively all files containing that letter
    let fuzzer_array = vec!["a", "e", "1", "x"];
    
    for char in fuzzer_array {
        let target = format!("{}{}/?search={}", host, base_route, char);
        println!("[*] Injecting Exhaustive Vector: {}", target);
        
        match client.get(&target).send().await {
            Ok(res) => {
                if res.status().is_success() {
                    let text = res.text().await.unwrap_or_default();
                    let occurrences = text.matches("<tr>").count(); // Rough count of file rows
                    
                    println!("    -> [SUCCESS] Search space flattened. Extracted ~{} items containing '{}'.", occurrences, char);
                    
                    if text.contains("class=\"pagination\"") || text.contains("Next") {
                        println!("    -> Pagination loops detected. This parameter will recursively extract entirely.");
                    }
                } else {
                    println!("    -> [FAIL] Status Code: {}", res.status());
                }
            }
            Err(e) => println!("    -> [ERROR] {}", e),
        }
        
        println!("\n[!] Awaiting Tor Exit Flush (20s)...");
        sleep(Duration::from_secs(20)).await;
    }

    drop(swarm_guard);
    Ok(())
}
