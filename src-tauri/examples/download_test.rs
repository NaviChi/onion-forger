use anyhow::Result;
use crawli_lib::adapters::qilin_nodes::QilinNodeCache;
use reqwest::{header::RANGE, Proxy, StatusCode};
use std::fs::File;
use std::io::Write;

#[tokio::main]
async fn main() -> Result<()> {
    println!("=== QILIN END-TO-END FILE DOWNLOAD TEST ===");
    println!("[1] Bootstrapping local Tor Daemon on 9051...");
    let tor_dir = std::env::temp_dir().join("crawli_dl_test3");
    std::fs::create_dir_all(&tor_dir)?;
    
    let mut cmd = std::process::Command::new("/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/src-tauri/bin/mac_aarch64/tor/tor");
    let mut child = cmd
        .arg("--SocksPort").arg("9051")
        .arg("--DataDirectory").arg(&tor_dir)
        .arg("--Log").arg("notice stdout")
        .spawn()?;

    tokio::time::sleep(std::time::Duration::from_secs(30)).await;

    let proxy = Proxy::all("socks5h://127.0.0.1:9051")?;
    let client = reqwest::Client::builder()
        .proxy(proxy)
        .danger_accept_invalid_certs(true)
        .timeout(std::time::Duration::from_secs(60))
        .build()?;

    let seed = "http://ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion/site/view?uuid=c9d2ba19-6aa1-3087-8773-f63d023179ed";
    let uuid = "c9d2ba19-6aa1-3087-8773-f63d023179ed";
    
    let node_cache = QilinNodeCache::default();
    node_cache.seed_known_mirrors(uuid).await;

    let target_url = if let Some(best_node) = node_cache.discover_and_resolve(seed, uuid, &client).await {
        println!("[2] Resolved to Storage Node: {}", best_node.url);
        
        let mut extracted_file_url = String::new();
        match client.get(&best_node.url).send().await {
            Ok(resp) => {
                if let Ok(html) = resp.text().await {
                    let v3_row_re = regex::Regex::new(r#"<td class="link"><a href="([^"]+)"[^>]*>.*?</a></td><td class="size">([^<]*)</td>"#).unwrap();
                    for cap in v3_row_re.captures_iter(&html) {
                        if let Some(href) = cap.get(1) {
                            let href_str = href.as_str();
                            if !href_str.ends_with('/') && href_str != "../" {
                                extracted_file_url = format!("{}{}", best_node.url, href_str);
                                break;
                            }
                        }
                    }
                }
            }
            Err(e) => println!("Failed to fetch storage node: {}", e)
        }
        extracted_file_url
    } else {
        String::new()
    };

    if target_url.is_empty() {
        println!("[!] Failed to find a file to download.");
        let _ = child.kill();
        let _ = std::fs::remove_dir_all(&tor_dir);
        return Ok(());
    }

    let target_disk = "/tmp/qilin_dynamic_dl.bin";

    println!("[3] Probing Qilin target [{}] (HEAD -> GET Range)...", target_url);
    
    // Simulating crawli's arithmetic fallback
    let mut size_known = false;
    if let Ok(Ok(head_resp)) = tokio::time::timeout(std::time::Duration::from_secs(8), client.head(&target_url).send()).await {
        if let Some(l) = head_resp.content_length() {
            println!("[*] HEAD returned size: {}", l);
            size_known = true;
        }
    }
    
    if !size_known {
        println!("[!] HEAD dropped/empty. Executing GET range proxy override...");
        if let Ok(Ok(range_resp)) = tokio::time::timeout(std::time::Duration::from_secs(8), client.get(&target_url).header(RANGE, "bytes=0-1").send()).await {
            if range_resp.status() == StatusCode::PARTIAL_CONTENT || range_resp.status() == StatusCode::OK {
                println!("[*] Range Override Successful.");
            }
        }
    }

    println!("[4] Downloading File Payload into Single Stream...");
    match client.get(&target_url).send().await {
        Ok(resp) if resp.status().is_success() => {
            let bytes = resp.bytes().await?;
            let mut f = File::create(target_disk)?;
            f.write_all(&bytes)?;
            println!("[SUCCESS] Extracted {} bytes to {}", bytes.len(), target_disk);
        }
        Ok(resp) => println!("[FAIL] Download returned HTTP {}", resp.status()),
        Err(e) => println!("[FAIL] Connection blocked: {}", e),
    }

    let _ = child.kill();
    let _ = std::fs::remove_dir_all(&tor_dir);
    Ok(())
}
