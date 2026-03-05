use reqwest::{Client, Proxy};
use std::time::Duration;
use regex::Regex;
use std::sync::Arc;
use crossbeam_queue::SegQueue;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=======================================================");
    println!("🔍 INITIATING QDATA FULL-SCALE 120-THREAD EXTRACTION PROBE 🔍");
    println!("Target: http://25mjg55vcbjzwykz2uqsvaw7hcevm4pqxl42o324zr6qf5zgddmghkqd.onion/site/data?uuid=c9d2ba19-6aa1-3087-8773-f63d023179ed/");
    println!("=======================================================\n");

    println!("[0] Booting ephemeral OS Tor daemon sequence...");
    
    let mut child = std::process::Command::new("tor")
        .arg("--SocksPort")
        .arg("9060")
        .arg("--DataDirectory")
        .arg("/tmp/tor_headless_probe_m4")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
        
    println!("    ✅ Tor daemon spawned on SOCKS 9060. Waiting 20s for full bootstrap...");
    tokio::time::sleep(Duration::from_secs(20)).await;

    let target_url = "http://25mjg55vcbjzwykz2uqsvaw7hcevm4pqxl42o324zr6qf5zgddmghkqd.onion/site/data?uuid=c9d2ba19-6aa1-3087-8773-f63d023179ed/";
    
    let queue = Arc::new(SegQueue::new());
    queue.push(target_url.to_string());
    
    let pending = Arc::new(std::sync::atomic::AtomicUsize::new(1));
    let mut workers = tokio::task::JoinSet::new();

    for worker_id in 0..60 {
        let q_clone = queue.clone();
        let p_clone = pending.clone();
        
        workers.spawn(async move {
            let proxy_url = "socks5h://127.0.0.1:9060";
            let proxy = match Proxy::all(proxy_url) {
                Ok(p) => p,
                Err(_) => return,
            };
            
            let client = match Client::builder()
                .proxy(proxy)
                .timeout(Duration::from_secs(120))
                .danger_accept_invalid_certs(true)
                .pool_max_idle_per_host(100)
                .build() {
                    Ok(c) => c,
                    Err(_) => return,
                };
                
            loop {
                let url = match q_clone.pop() {
                    Some(u) => u,
                    None => {
                        if p_clone.load(std::sync::atomic::Ordering::SeqCst) == 0 {
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(50)).await;
                        continue;
                    }
                };
                
                struct Guard { c: Arc<std::sync::atomic::AtomicUsize> }
                impl Drop for Guard { fn drop(&mut self) { self.c.fetch_sub(1, std::sync::atomic::Ordering::SeqCst); } }
                let _g = Guard { c: p_clone.clone() };

                println!("[Worker {}] Fetching {}...", worker_id, url);
                
                let mut html = String::new();
                for attempt in 1..=5 {
                    println!("[Worker {}] Attempt {}...", worker_id, attempt);
                    match client.get(&url).send().await {
                        Ok(resp) => {
                            if let Ok(text) = resp.text().await {
                                html = text;
                                println!("✅ [Worker {}] Yield: {} bytes", worker_id, html.len());
                                break;
                            }
                        },
                        Err(e) => {
                            eprintln!("⚠️ [Worker {}] Try {} Err: {}", worker_id, attempt, e);
                        }
                    }
                    tokio::time::sleep(Duration::from_secs(1 << attempt)).await;
                }
                
                if html.is_empty() {
                    eprintln!("❌ [Worker {}] Discarding node.", worker_id);
                    continue;
                }
                
                let document = scraper::Html::parse_document(&html);
                if let Ok(selector) = scraper::Selector::parse("script#__NEXT_DATA__") {
                    for script in document.select(&selector) {
                         let json_text = script.inner_html();
                         println!("\n🚀🚀🚀 [HEADLESS PROBE] Located __NEXT_DATA__ blob! Size: {} bytes\n[PREVIEW]: {}\n...", json_text.len(), &json_text[..std::cmp::min(1500, json_text.len())]);
                         std::process::exit(0);
                    }
                }
                let api_regex = Regex::new(r#"(?i)/api/[a-z0-9_/-]+"#).unwrap();
                for cap in api_regex.captures_iter(&html) {
                     println!("⚠️ [HEADLESS PROBE] Found Potential Hidden API Route: {}", &cap[0]);
                }
                let token_regex = Regex::new(r#"Authorization:\s*Bearer\s*([a-zA-Z0-9\-_]+\.[a-zA-Z0-9\-_]+\.[a-zA-Z0-9\-_]+)"#).unwrap();
                for cap in token_regex.captures_iter(&html) {
                     println!("⚠️ [HEADLESS PROBE] Found Potential Hidden JWT: {}", &cap[1]);
                }
                if html.contains("_csrf") {
                    println!("⚠️ [HEADLESS PROBE] CSRF token signature detected in payload.");
                }
            }
        });
    }

    while let Some(_) = workers.join_next().await {}

    let _ = child.kill();
    Ok(())
}
