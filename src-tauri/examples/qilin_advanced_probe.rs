use reqwest::header::{HeaderMap, HeaderValue};
use std::time::Duration;

#[tokio::main]
async fn main() {
    let app = tauri::Builder::default()
        .build(tauri::generate_context!())
        .expect("build tauri app");

    println!("Bootstrapping Tor circuit...");
    crawli_lib::tor::cleanup_stale_tor_daemons();
    let (swarm_guard, ports) = crawli_lib::tor::bootstrap_tor_cluster(app.handle().clone(), 1)
        .await
        .expect("Failed to boot Tor cluster");

    let proxy = reqwest::Proxy::all(format!("socks5h://127.0.0.1:{}", ports[0])).unwrap();
    let client = reqwest::Client::builder()
        .proxy(proxy)
        .timeout(Duration::from_secs(60))
        .build()
        .unwrap();

    let base_url = "http://a7r2n577n6jqzqexu5an3j2aej3ezb4klm7pkbp44243cqbwi43brjid.onion/72a4c05f-f711-498a-a038-758efa78aa09/";

    println!("\n=================================");
    println!("🧪 TESTING QILIN ADVANCED THEORIES");
    println!("Target: {}", base_url);
    println!("=================================\n");

    // ==========================================
    // THEORY A: WebDAV PROPFIND
    // ==========================================
    println!("-> [Theory A] Executing WebDAV PROPFIND request...");
    let mut headers = HeaderMap::new();
    headers.insert("Depth", HeaderValue::from_static("infinity"));

    match client
        .request(reqwest::Method::from_bytes(b"PROPFIND").unwrap(), base_url)
        .headers(headers)
        .send()
        .await
    {
        Ok(resp) => {
            println!("   Status: {}", resp.status());
            if resp.status().is_success() || resp.status() == reqwest::StatusCode::MULTI_STATUS {
                if let Ok(text) = resp.text().await {
                    let preview = text.chars().take(300).collect::<String>();
                    println!(
                        "   [✅ SUCCESS] WebDAV Enabled! Payload Preview:\n{}\n",
                        preview
                    );
                }
            } else {
                println!(
                    "   [❌ FAILED] WebDAV appears blocked or unsupported (Status {}).\n",
                    resp.status()
                );
            }
        }
        Err(e) => println!("   [❌ ERROR] PROPFIND Request failed: {}\n", e),
    }

    // ==========================================
    // THEORY B: Nginx JSON Autoindex (?F=1)
    // ==========================================
    let json_url = format!("{}?F=1", base_url);
    println!("-> [Theory B] Requesting native Nginx JSON format (?F=1)...");
    match client.get(&json_url).send().await {
        Ok(resp) => {
            println!("   Status: {}", resp.status());
            let content_type = resp
                .headers()
                .get("content-type")
                .and_then(|h| h.to_str().ok())
                .unwrap_or("unknown");
            println!("   Content-Type: {}", content_type);

            if content_type.contains("application/json") {
                if let Ok(text) = resp.text().await {
                    let preview = text.chars().take(300).collect::<String>();
                    println!(
                        "   [✅ SUCCESS] JSON Format Extracted! Payload Preview:\n{}\n",
                        preview
                    );
                }
            } else {
                println!("   [❌ FAILED] Server ignored format flag and returned HTML.\n");
            }
        }
        Err(e) => println!("   [❌ ERROR] JSON Request failed: {}\n", e),
    }

    // ==========================================
    // THEORY C: Server-Side Zip Archive
    // ==========================================
    let zip_url = format!("{}?download=1", base_url);
    println!("-> [Theory C] Probing for hidden archive endpoint (?download=1)...");
    match client.get(&zip_url).send().await {
        Ok(resp) => {
            println!("   Status: {}", resp.status());
            let content_type = resp
                .headers()
                .get("content-type")
                .and_then(|h| h.to_str().ok())
                .unwrap_or("unknown");
            let disposition = resp
                .headers()
                .get("content-disposition")
                .and_then(|h| h.to_str().ok())
                .unwrap_or("none");

            println!("   Content-Type: {}", content_type);
            println!("   Content-Disposition: {}", disposition);

            if content_type.contains("application/zip")
                || content_type.contains("application/x-zip")
                || disposition.contains("attachment")
            {
                println!("   [✅ SUCCESS] Server-side zip trigger discovered!\n");
            } else {
                println!("   [❌ FAILED] Archive endpoint not present. Target responded with standard HTML.\n");
            }
        }
        Err(e) => println!("   [❌ ERROR] Archive Request failed: {}\n", e),
    }

    drop(swarm_guard);
}
