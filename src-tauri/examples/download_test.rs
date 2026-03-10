use anyhow::Result;
use crawli_lib::adapters::qilin_nodes::QilinNodeCache;
use crawli_lib::arti_client::ArtiClient;
use crawli_lib::tor_native::spawn_tor_node;
use reqwest::{header::RANGE, StatusCode};
use std::fs::File;
use std::io::Write;

#[tokio::main]
async fn main() -> Result<()> {
    println!("=== QILIN END-TO-END FILE DOWNLOAD TEST ===");
    println!("[1] Bootstrapping direct TorForge Arti client...");

    let tor_client = spawn_tor_node(0, false).await?;
    let client = ArtiClient::new(tor_client, Some(arti_client::IsolationToken::new()));

    let seed = "http://25j35d6uf37tvfqt5pmz457yicgu35yhizojqxbfzv33dni2d73q3oad.onion/80349839-d06f-41a8-b954-3602fe60725a/";
    let uuid = "c9d2ba19-6aa1-3087-8773-f63d023179ed";

    let node_cache = QilinNodeCache::default();
    node_cache.seed_known_mirrors(uuid).await;

    let target_url = if let Some(best_node) = node_cache
        .discover_and_resolve(seed, uuid, &client, None)
        .await
    {
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
            Err(err) => println!("Failed to fetch storage node: {}", err),
        }
        extracted_file_url
    } else {
        String::new()
    };

    if target_url.is_empty() {
        println!("[!] Failed to find a file to download.");
        return Ok(());
    }

    let target_disk = std::env::temp_dir().join("qilin_dynamic_dl.bin");

    println!(
        "[3] Probing Qilin target [{}] (HEAD -> GET Range)...",
        target_url
    );

    let mut size_known = false;
    if let Ok(Ok(head_resp)) = tokio::time::timeout(
        std::time::Duration::from_secs(8),
        client.head(&target_url).send(),
    )
    .await
    {
        if let Some(len) = head_resp.content_length() {
            println!("[*] HEAD returned size: {}", len);
            size_known = true;
        }
    }

    if !size_known {
        println!("[!] HEAD dropped/empty. Executing GET range proxy override...");
        if let Ok(Ok(range_resp)) = tokio::time::timeout(
            std::time::Duration::from_secs(8),
            client
                .get(&target_url)
                .header(RANGE.as_str(), "bytes=0-1")
                .send(),
        )
        .await
        {
            if range_resp.status() == StatusCode::PARTIAL_CONTENT
                || range_resp.status() == StatusCode::OK
            {
                println!("[*] Range Override Successful.");
            }
        }
    }

    println!("[4] Downloading File Payload into Single Stream...");
    match client.get(&target_url).send().await {
        Ok(resp) if resp.status().is_success() => {
            let bytes = resp.bytes().await?;
            let mut file = File::create(&target_disk)?;
            file.write_all(&bytes)?;
            println!(
                "[SUCCESS] Extracted {} bytes to {}",
                bytes.len(),
                target_disk.display()
            );
        }
        Ok(resp) => println!("[FAIL] Download returned HTTP {}", resp.status()),
        Err(err) => println!("[FAIL] Connection blocked: {}", err),
    }
    Ok(())
}
