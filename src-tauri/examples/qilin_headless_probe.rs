use regex::Regex;
use reqwest::{Client, Proxy};
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=======================================================");
    println!("🔍 INITIATING QDATA TIER-3 HEADLESS API PROBE (MULTI-THREADED) 🔍");
    println!("Target: http://25mjg55vcbjzwykz2uqsvaw7hcevm4pqxl42o324zr6qf5zgddmghkqd.onion/site/data?uuid=c9d2ba19-6aa1-3087-8773-f63d023179ed/");
    println!("=======================================================\n");

    println!("[0] Booting ephemeral OS Tor daemon for probe...");

    let mut child = std::process::Command::new("tor")
        .arg("--SocksPort")
        .arg("9057")
        .arg("--DataDirectory")
        .arg("/tmp/tor_headless_probe_m2")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;

    println!("    ✅ Tor daemon spawned on SOCKS 9057. Waiting 15s for bootstrap...");
    tokio::time::sleep(Duration::from_secs(15)).await;

    let proxy_url = "socks5h://127.0.0.1:9057";
    let proxy = Proxy::all(proxy_url)?;
    let client = Client::builder()
        .proxy(proxy)
        .timeout(Duration::from_secs(120))
        .danger_accept_invalid_certs(true)
        .pool_max_idle_per_host(100)
        .build()?;

    let target_url = "http://25mjg55vcbjzwykz2uqsvaw7hcevm4pqxl42o324zr6qf5zgddmghkqd.onion/site/data?uuid=c9d2ba19-6aa1-3087-8773-f63d023179ed/";

    println!("\n[1] Fetching root UI application (Exponential Backoff Strategy)...");

    let mut html = String::new();
    let mut retries = 0;
    let max_retries = 7;
    let mut delay = 2;

    while retries < max_retries {
        let start = std::time::Instant::now();
        println!("    ⏳ Attempt {} (timeout 120s)...", retries + 1);
        match client.get(target_url).send().await {
            Ok(resp) => {
                if let Ok(text) = resp.text().await {
                    html = text;
                    println!(
                        "    ✅ Fetched {} bytes of HTML in {:?}",
                        html.len(),
                        start.elapsed()
                    );
                    break;
                }
            }
            Err(e) => {
                println!("    ⚠️ Attempt {} Failed: {}", retries + 1, e);
            }
        }
        retries += 1;

        println!("    💤 Backing off for {} seconds...", delay);
        tokio::time::sleep(Duration::from_secs(delay)).await;
        delay *= 2;
    }

    if html.is_empty() {
        println!("    ❌ PROBE FATAL: Tor connection dropped entirely. Cannot scan Document.");
        let _ = child.kill();
        let _ = child.wait();
        return Ok(());
    }

    println!("\n[2] Scanning AST for Next.js/React hydration blobs...");
    let document = scraper::Html::parse_document(&html);
    let selector = scraper::Selector::parse("script#__NEXT_DATA__").unwrap();

    let mut found_json = false;
    for script in document.select(&selector) {
        found_json = true;
        let json_text = script.inner_html();
        println!(
            "    ✅ Located __NEXT_DATA__ blob! Size: {} bytes",
            json_text.len()
        );

        let dump_size = std::cmp::min(1500, json_text.len());
        println!("    [PREVIEW]: {}\n...", &json_text[..dump_size]);
    }

    if !found_json {
        println!("    ❌ PROBE FAILED: No React Data Blobs found. Qilin CMS might use a different framework.");

        println!("\n[3] Fallback: Scanning for traditional API endpoints in raw HTML...");
        let api_regex = Regex::new(r#"(?i)/api/[a-z0-9_/-]+"#).unwrap();
        for cap in api_regex.captures_iter(&html) {
            println!("    ⚠️ Found Potential Hidden API Route: {}", &cap[0]);
        }

        if html.to_lowercase().contains("bearer") {
            println!("    ⚠️ Found 'Bearer' string in payload. Scanning surrounding text...");
        }

        let csrf_regex = Regex::new(r#"_csrf"?\s*:\s*"([^"]+)""#).unwrap();
        for cap in csrf_regex.captures_iter(&html) {
            println!("    ⚠️ Found Potential CSRF Session Token: {}", &cap[1]);
        }

        println!("\n[DEBUG] Raw HTML Head Preview:");
        let peek_size = std::cmp::min(3000, html.len());
        println!("{}", &html[0..peek_size]);
    }

    let _ = child.kill();
    let _ = child.wait();
    Ok(())
}
