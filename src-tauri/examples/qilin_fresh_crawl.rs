use reqwest::{Client, Proxy};
use std::collections::HashSet;
use std::time::Duration;

#[tokio::main]
async fn main() {
    println!("\n=======================================================");
    println!("🔍 TBC CONSOLES — TARGETED EXTRACTION");
    println!("=======================================================\n");

    // Both known storage nodes for TBC Consoles
    let targets = vec![
        ("Alt Storage (302 redirect)", "http://7mnkv5nvnjyifezlfyba6gek7aeimg5eghej5vp65qxnb2hjbtlttlyd.onion/d4ccd219-d197-4d24-81ef-a42fed816b8a/"),
        ("Primary Storage (screenshot)", "http://25mjg55vcbjzwykz2uqsvaw7hcevm4pqxl42o324zr6qf5zgddmghkqd.onion/6a230577-89fc-4731-bec1-6fdfa81656fc/"),
        ("CMS Data redirect", "http://ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion/site/data?uuid=c9d2ba19-6aa1-3087-8773-f63d023179ed"),
    ];

    let tor_ports = vec![9052, 9053, 9151];

    for port in &tor_ports {
        let proxy_url = format!("socks5h://127.0.0.1:{}", port);
        let Ok(proxy) = Proxy::all(&proxy_url) else {
            continue;
        };
        let Ok(client) = Client::builder()
            .proxy(proxy)
            .timeout(Duration::from_secs(120))
            .danger_accept_invalid_certs(true)
            .build()
        else {
            continue;
        };

        for (label, url) in &targets {
            println!("[Port {}] [{}] Fetching: {}", port, label, url);
            match client.get(*url).send().await {
                Ok(resp) => {
                    let status = resp.status();
                    let final_url = resp.url().as_str().to_string();
                    println!("  [✓] Status: {} | Final URL: {}", status, final_url);

                    if let Ok(body) = resp.text().await {
                        println!("  [✓] Body: {} bytes\n", body.len());

                        let (dirs, files) = parse_autoindex(&body);

                        if dirs.is_empty() && files.is_empty() {
                            println!("  [⚠] No autoindex entries found. HTML preview:");
                            let preview: String = body.chars().take(1500).collect();
                            println!("{}\n", preview);
                            continue;
                        }

                        println!("  ═══════════════════════════════════════");
                        println!("  📊 ROOT: {} dirs, {} files", dirs.len(), files.len());
                        println!("  ═══════════════════════════════════════\n");

                        println!("  📁 DIRECTORIES:");
                        for d in &dirs {
                            println!(
                                "    📂 {}",
                                urlencoding::decode(d).unwrap_or_else(|_| d.clone().into())
                            );
                        }
                        println!("\n  📄 FILES:");
                        for f in &files {
                            println!(
                                "    📄 {}",
                                urlencoding::decode(f).unwrap_or_else(|_| f.clone().into())
                            );
                        }

                        // Crawl subdirectories
                        println!("\n  🔍 CRAWLING SUBDIRECTORIES...\n");
                        let base = if final_url.ends_with('/') {
                            final_url.clone()
                        } else {
                            format!("{}/", final_url)
                        };

                        for dir in &dirs {
                            let sub_url = format!("{}{}", base, dir);
                            let decoded_dir =
                                urlencoding::decode(dir).unwrap_or_else(|_| dir.clone().into());
                            println!("  ┌─ 📂 {}", decoded_dir);

                            match client.get(&sub_url).send().await {
                                Ok(sub_resp) => {
                                    if let Ok(sub_body) = sub_resp.text().await {
                                        let (sub_dirs, sub_files) = parse_autoindex(&sub_body);
                                        for sf in &sub_files {
                                            println!(
                                                "  │  📄 {}",
                                                urlencoding::decode(sf)
                                                    .unwrap_or_else(|_| sf.clone().into())
                                            );
                                        }
                                        for sd in &sub_dirs {
                                            println!(
                                                "  │  📂 {}",
                                                urlencoding::decode(sd)
                                                    .unwrap_or_else(|_| sd.clone().into())
                                            );
                                        }
                                        println!(
                                            "  │  ({} files, {} subdirs)",
                                            sub_files.len(),
                                            sub_dirs.len()
                                        );
                                    }
                                }
                                Err(e) => println!("  │  [✗] {}", e),
                            }
                            println!("  └─");
                        }

                        println!("\n  ✅ TBC Consoles extraction complete from this node!");
                        println!("\n=======================================================");
                        println!("🏁 DONE");
                        println!("=======================================================");
                        return; // Success! Stop trying other targets/ports.
                    }
                }
                Err(e) => {
                    println!("  [✗] Failed: {}\n", e);
                }
            }
        }
    }

    println!("\n[✗] All storage nodes and ports exhausted.");
    println!("=======================================================");
}

fn parse_autoindex(body: &str) -> (Vec<String>, Vec<String>) {
    let link_re = regex::Regex::new(r#"href="([^"]+)""#).unwrap();
    let mut files = Vec::new();
    let mut dirs = Vec::new();
    let mut seen = HashSet::new();

    for cap in link_re.captures_iter(body) {
        let href = cap[1].to_string();
        if href == "../" || href == "./" || href.starts_with("?") || href.starts_with("#") {
            continue;
        }
        if href.starts_with("http") {
            continue;
        }
        if href.starts_with("/fancy/") || href.starts_with("/icons/") {
            continue;
        }
        if href == "${href}" {
            continue;
        }
        if seen.contains(&href) {
            continue;
        }
        seen.insert(href.clone());

        if href.ends_with('/') {
            dirs.push(href);
        } else {
            files.push(href);
        }
    }

    (dirs, files)
}
