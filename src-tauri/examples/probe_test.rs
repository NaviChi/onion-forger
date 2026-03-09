use anyhow::Result;
use crawli_lib::adapters::qilin_nodes::QilinNodeCache;
use crawli_lib::arti_client::ArtiClient;
use crawli_lib::tor_native::spawn_tor_node;

#[tokio::main]
async fn main() -> Result<()> {
    println!("Booting direct TorForge Arti client...");

    let tor_client = spawn_tor_node(0, false).await?;
    let client = ArtiClient::new(tor_client, Some(arti_client::IsolationToken::new()));

    let seed = "http://25j35d6uf37tvfqt5pmz457yicgu35yhizojqxbfzv33dni2d73q3oad.onion/80349839-d06f-41a8-b954-3602fe60725a/";
    let uuid = "c9d2ba19-6aa1-3087-8773-f63d023179ed";

    let node_cache = QilinNodeCache::default();
    node_cache.seed_known_mirrors(uuid).await;

    if let Some(best_node) = node_cache.discover_and_resolve(seed, uuid, &client, None).await {
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
                            if href_str == "../" || href_str == "/" || href_str.starts_with('?') {
                                continue;
                            }

                            let is_dir = href_str.ends_with('/');
                            if !is_dir {
                                let raw_size = size_str.as_str().trim();
                                if raw_size == "0"
                                    || raw_size == "0 B"
                                    || raw_size == "0.0 B"
                                    || raw_size == "0.00 B"
                                {
                                    zero_byte_matches += 1;
                                }
                                v3_matches += 1;
                            }
                        }
                    }

                    println!("Total V3 Files Captured: {}", v3_matches);
                    println!(
                        "Zero-byte structural files recovered: {}",
                        zero_byte_matches
                    );
                }
            }
            Err(err) => println!("Failed to fetch storage node: {}", err),
        }
    } else {
        println!("FAILED TO RESOLVE STORAGE NODE.");
    }
    Ok(())
}
