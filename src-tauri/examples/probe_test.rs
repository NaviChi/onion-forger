use anyhow::Result;
use crawli_lib::adapters::qilin_nodes::QilinNodeCache;
use reqwest::Proxy;

#[tokio::main]
async fn main() -> Result<()> {
    println!("Booting embedded Tor daemon on 9051...");
    let tor_dir = std::env::temp_dir().join("crawli_html_test3");
    std::fs::create_dir_all(&tor_dir)?;
    
    let mut cmd = std::process::Command::new("/Users/navi/Documents/Projects/LOKI TOOLS/Onion Forger/crawli/src-tauri/bin/mac_aarch64/tor/tor");
    let mut child = cmd
        .arg("--SocksPort")
        .arg("9051")
        .arg("--DataDirectory")
        .arg(&tor_dir)
        .arg("--Log")
        .arg("notice stdout")
        .spawn()?;

    tokio::time::sleep(std::time::Duration::from_secs(30)).await;

    let proxy = Proxy::all("socks5h://127.0.0.1:9051")?;
    let client = reqwest::Client::builder()
        .proxy(proxy)
        .danger_accept_invalid_certs(true)
        .timeout(std::time::Duration::from_secs(120))
        .build()?;

    let seed = "http://ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion/site/view?uuid=c9d2ba19-6aa1-3087-8773-f63d023179ed";
    let uuid = "c9d2ba19-6aa1-3087-8773-f63d023179ed";
    
    let node_cache = QilinNodeCache::default();
    node_cache.seed_known_mirrors(uuid).await;

    if let Some(best_node) = node_cache.discover_and_resolve(seed, uuid, &client).await {
        println!("Resolved to Storage Node: {}", best_node.url);
        
        match client.get(&best_node.url).send().await {
            Ok(resp) => {
                if let Ok(html) = resp.text().await {
                    println!("Storage Node HTML size: {} bytes", html.len());
                    let mut v3_matches = 0;
                    let mut zero_byte_matches = 0;

                    let v3_row_re = regex::Regex::new(r#"<td class="link"><a href="([^"]+)"[^>]*>.*?</a></td><td class="size">([^<]*)</td>"#).unwrap();
                    for cap in v3_row_re.captures_iter(&html) {
                        if let (Some(href), Some(size_str)) = (cap.get(1), cap.get(2)) {
                            let href_str = href.as_str();
                            if href_str == "../" || href_str == "/" || href_str.starts_with("?") {
                                continue;
                            }
                            
                            let is_dir = href_str.ends_with('/');
                            if !is_dir {
                                let raw_size = size_str.as_str().trim();
                                if raw_size == "0" || raw_size == "0 B" || raw_size == "0.0 B" || raw_size == "0.00 B" {
                                    zero_byte_matches += 1;
                                }
                                v3_matches += 1;
                            }
                        }
                    }
                    
                    println!("Total V3 Files Captured: {}", v3_matches);
                    println!("Zero-byte structural files recovered: {}", zero_byte_matches);
                }
            }
            Err(e) => println!("Failed to fetch storage node: {}", e)
        }
    } else {
        println!("FAILED TO RESOLVE STORAGE NODE.");
    }

    let _ = child.kill();
    let _ = std::fs::remove_dir_all(&tor_dir);
    Ok(())
}
