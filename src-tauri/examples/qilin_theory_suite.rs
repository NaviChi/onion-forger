use crawli_lib::tor::TorProcessGuard;
use reqwest::{Client, Method, Proxy};
use std::time::Duration;
use tokio::time::sleep;

/// Standalone Evaluation Suite for Theory Vectors A-C on QData.
///
/// Target: http://ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion
/// Route: /site/data?uuid=c9d2ba19-6aa1-3087-8773-f63d023179ed/
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n=======================================================");
    println!(" QILIN ADVANCED THEORETICAL VALIDATION SUITE (PHASE 25)");
    println!("=======================================================\n");

    // 1. Boot up Tor Daemon for Proxy
    println!("[*] Initializing Tor Swarm on 127.0.0.1:9051...");
    let swarm_guard = TorProcessGuard::new();
    sleep(Duration::from_secs(4)).await;

    // 2. Build explicit HTTP/1.1 client instance (Disabling Tor TLS/Cert Validate)
    let proxy = Proxy::all("socks5h://127.0.0.1:9051")?;
    let client = Client::builder()
        .proxy(proxy)
        .danger_accept_invalid_certs(true)
        .http2_prior_knowledge() // TUNNEL BORE OVERRIDE (Theory 7.1)
        .timeout(Duration::from_secs(120))
        .connect_timeout(Duration::from_secs(45))
        .pool_max_idle_per_host(8) // Keep-alives
        .tcp_nodelay(true)
        .build()?;

    // Base URL context
    let host = "http://25mjg55vcbjzwykz2uqsvaw7hcevm4pqxl42o324zr6qf5zgddmghkqd.onion";
    let base_route = "/6a230577-89fc-4731-bec1-6fdfa81656fc/";
    let target_dir = format!("{}{}", host, base_route);

    println!("[*] Qilin Target Acquired: {}\n", host);

    // ==========================================
    // THEORY A: WebDAV PROPFIND (Depth: infinity)
    // ==========================================
    println!("[TEST A] Executing WebDAV PROPFIND Vector...");
    println!("   -> Action: Requesting raw XML directory payload via PROPFIND method.");
    let req_a = client
        .request(Method::from_bytes(b"PROPFIND").unwrap(), &target_dir)
        .header("Depth", "infinity")
        .build()?;

    match client.execute(req_a).await {
        Ok(res) => {
            println!("   -> Response Status: {}", res.status());
            if res.status().is_success() || res.status().as_u16() == 207 {
                println!("   -> [SUCCESS] Target exposes WebDAV XML payloads!");
                let body = res.text().await.unwrap_or_default();
                println!(
                    "   -> Snippet: {:?}",
                    &body.chars().take(200).collect::<String>()
                );
            } else {
                println!("   -> [FAILED] Target rejected PROPFIND logic. Not a WebDAV node.");
            }
        }
        Err(e) => println!("   -> [ERROR] Request dropped: {}", e),
    }

    println!("\n[!] Awaiting 60 seconds to flush QData Rate-Limiter State...");
    sleep(Duration::from_secs(60)).await;

    // ==========================================
    // THEORY B: Nginx JSON/XML Formatting Flags
    // ==========================================
    println!("\n[TEST B] Executing Nginx Autoindex Format Extraction...");
    println!("   -> Action: Injecting `?search=` JSON parameters.");

    // QData now uses Search/Pagination routing logic. If it autoindexes JSON, it'll likely expose it via a clean query or `?format=json`
    let target_b = format!("{}?search=M", target_dir);
    match client.get(&target_b).send().await {
        Ok(res) => {
            println!("   -> Response Status: {}", res.status());
            let content_type = res
                .headers()
                .get("content-type")
                .and_then(|h| h.to_str().ok())
                .unwrap_or("unknown");
            println!("   -> Content-Type: {}", content_type);

            if content_type.contains("application/json") {
                println!("   -> [SUCCESS] Nginx Autoindex yielded structural JSON!");
            } else {
                println!(
                    "   -> [FAILED] Nginx JSON Autoindex formatting is disabled or unrecognized."
                );
            }
        }
        Err(e) => println!("   -> [ERROR] Request dropped: {}", e),
    }

    println!("\n[!] Awaiting 60 seconds to flush QData Rate-Limiter State...");
    sleep(Duration::from_secs(60)).await;

    // ==========================================
    // THEORY C: Server-Side Archive Triggers
    // ==========================================
    println!("\n[TEST C] Executing Server-Side Archive Trigger Fuzzing...");
    let archive_suffixes = vec!["&download=1", "&zip=true", "&archive=tar"];

    for suffix in archive_suffixes {
        println!("   -> Injecting suffix: {}", suffix);
        let target_c = format!("{}{}", target_dir, suffix);
        match client.get(&target_c).send().await {
            Ok(res) => {
                let content_type = res
                    .headers()
                    .get("content-type")
                    .and_then(|h| h.to_str().ok())
                    .unwrap_or("unknown");
                let status = res.status();
                if status.is_success()
                    && (content_type.contains("application/zip")
                        || content_type.contains("application/x-tar"))
                {
                    println!(
                        "      -> [SUCCESS] Archive endpoint found! ({})",
                        content_type
                    );
                } else {
                    println!(
                        "      -> [FAILED] Status: {}, Type: {}",
                        status, content_type
                    );
                }
            }
            Err(e) => println!("      -> [ERROR] Request dropped: {}", e),
        }
    }

    println!("\n[!] Awaiting 60 seconds to flush QData Rate-Limiter State...");
    sleep(Duration::from_secs(60)).await;

    // ==========================================
    // THEORY E: Headless API Hydration (CMS)
    // ==========================================
    println!("\n[TEST E] Executing Headless API Hydration Probe...");
    println!("   -> Action: Spoofing XMLHTTPRequest to force Next.js/Vue API JSON payloads.");

    let req_e = client
        .get(&target_dir)
        .header("Accept", "application/json")
        .header("X-Requested-With", "XMLHttpRequest")
        .header("Sec-Fetch-Mode", "cors")
        .header("Sec-Fetch-Site", "same-origin")
        .build()?;

    match client.execute(req_e).await {
        Ok(res) => {
            println!("   -> Response Status: {}", res.status());
            let content_type = res
                .headers()
                .get("content-type")
                .and_then(|h| h.to_str().ok())
                .unwrap_or("unknown");
            println!("   -> Content-Type: {}", content_type);

            if content_type.contains("application/json") {
                println!(
                    "   -> [SUCCESS] Headless CMS API Endpoint discovered! ({})",
                    content_type
                );
                let text = res.text().await.unwrap_or_default();
                println!(
                    "   -> Snippet: {}",
                    &text.chars().take(250).collect::<String>()
                );
            } else {
                println!(
                    "   -> [FAILED] Target ignored API headers and returned standard HTML/DOM."
                );
            }
        }
        Err(e) => println!("   -> [ERROR] Request dropped: {}", e),
    }

    println!("\n[!] Awaiting 60 seconds to flush QData Rate-Limiter State...");
    sleep(Duration::from_secs(60)).await;

    // Drop tor proxy
    println!("\n[*] Validation Suite Finished.");
    drop(swarm_guard);

    Ok(())
}
