use std::time::Duration;
use reqwest::{Client, Proxy};
use rand::Rng;

/// The "Amnesiac Ephemeral Sweeper" Architecture
/// This process enforces strict statelessness. Every single HTTP fetch is executed on a 
/// completely new Tor circuit identity, isolated from a connection pool, with 
/// Gaussian Jitter to prevent predictable Anti-DDoS rate limit triggers.
#[tokio::main]
async fn main() {
    let app = tauri::Builder::default()
        .build(tauri::generate_context!())
        .expect("build tauri app");

    crawli_lib::tor::cleanup_stale_tor_daemons();

    // We only need 1 Daemon for the Ephemeral Sweeper, but we will constantly rotate its identity.
    let (swarm_guard, ports) = crawli_lib::tor::bootstrap_tor_cluster(app.handle().clone(), 1)
        .await
        .unwrap();

    let proxy_port = ports[0];
    // The control port is historically mapped as SOCKS + 1 in this infrastructure
    let control_port = proxy_port + 1; 

    // Target the specific failing Root Qilin URL
    let target_url = "http://a7r2n577n6jqzqexu5an3j2aej3ezb4klm7pkbp44243cqbwi43brjid.onion/72a4c05f-f711-498a-a038-758efa78aa09/";

    println!("\n=======================================================");
    println!("🌪️ INITIATING THE AMNESIAC EPHEMERAL SWEEPER PROTOCOL 🌪️");
    println!("Target: {}", target_url);
    println!("Strategy: Stateless 1-File Micro-Bursting with Jitter");
    println!("=======================================================\n");

    let mut successful_fetches = 0;

    // We will simulate crawling by making 5 isolated state-fetches.
    for crawl_step in 1..=5 {
        println!("\n--- [Step {}/5] Preparing Ephemeral Micro-Burst ---", crawl_step);

        // 1. SIGNAL NEWNYM to Tor Control Port to rotate IP
        println!("  -> Sending SIGNAL NEWNYM to Control Port for SOCKS {}...", proxy_port);
        match crawli_lib::tor::request_newnym(proxy_port).await {
            Ok(_) => println!("     [✅] Tor Identity Rotated."),
            Err(e) => println!("     [⚠️] Failed to rotate identity: {}", e),
        }

        // 2. Introduce Algorithmic Gaussian Jitter (Ghost Polling)
        // We pause between 3.5 and 9.5 seconds randomly to evade pattern-matching proxies.
        let jitter_ms = rand::thread_rng().gen_range(3500..9500);
        println!("  -> Engaging Gaussian Jitter: Sleeping for {}ms...", jitter_ms);
        tokio::time::sleep(Duration::from_millis(jitter_ms)).await;

        // 3. Construct a completely fresh, stateless HTTP client. No connection pooling across iterations.
        println!("  -> Constructing Ephemeral Stateless Socket...");
        let proxy = Proxy::all(format!("socks5h://127.0.0.1:{}", proxy_port)).unwrap();
        let client = Client::builder()
            .proxy(proxy)
            .timeout(Duration::from_secs(45)) // Shorter timeout: if it hangs, we drop the socket immediately
            .pool_idle_timeout(Duration::from_millis(1)) // Forcibly drop keep-alives instantly
            .build()
            .unwrap();

        // 4. Fire the Micro-Burst
        let start_time = std::time::Instant::now();
        match client.get(target_url).send().await {
            Ok(resp) => {
                let status = resp.status();
                println!("  -> Response Received [HTTP {}] in {}ms", status, start_time.elapsed().as_millis());
                if status.is_success() {
                    if let Ok(text) = resp.text().await {
                        let parsed = crawli_lib::adapters::autoindex::parse_autoindex_html(&text);
                        println!("     [✅ SUCCESS] Extracted {} directory elements transparently.", parsed.len());
                        if parsed.is_empty() {
                            println!("     [⚠️] But HTML payload was empty or structure changed. Preview:\n     {}", text.chars().take(150).collect::<String>());
                        } else {
                            successful_fetches += 1;
                        }
                    }
                } else {
                    println!("     [❌ FAILED] Target actively refused connection or issued rate-limit.");
                }
            }
            Err(e) => {
                println!("  -> [❌ ERROR] Socket dropped during Micro-Burst: {}", e);
            }
        }

        // Explicitly drop the client so tokio closes the TCP sockets gracefully with a FIN packet
        drop(client);
    }

    println!("\n=======================================================");
    println!("🏁 EPHEMERAL SWEEPER COMPLETE 🏁");
    println!("Successful Micro-Bursts Bypassing DDoS Filter: {}/5", successful_fetches);
    println!("=======================================================\n");

    drop(swarm_guard);
}
