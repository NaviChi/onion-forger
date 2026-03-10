use anyhow::Result;
use crawli_lib::frontier::{CrawlOptions, CrawlerFrontier};

#[tokio::main]
async fn main() -> Result<()> {
    let seed_url = "http://25j35d6uf37tvfqt5pmz457yicgu35yhizojqxbfzv33dni2d73q3oad.onion/80349839-d06f-41a8-b954-3602fe60725a/";
    let options = CrawlOptions {
        listing: true,
        sizes: true,
        download: false,
        circuits: Some(60),
        daemons: Some(1),
        agnostic_state: false,
        resume: false,
        resume_index: None,
        mega_password: None,
        stealth_ramp: true,
    };

    let frontier = CrawlerFrontier::new(
        None,
        seed_url.to_string(),
        1,
        true,
        vec![9051],
        Vec::new(),
        options,
        None,
    );

    println!("Qilin healing smoke test");
    println!("  seed url: {}", seed_url);
    println!("  proxy-backed clients: {}", frontier.http_clients.len());
    println!("  worker target: {}", frontier.worker_target());
    println!(
        "  daemon mapping entries: {}",
        frontier.client_daemon_map.len()
    );
    println!("Use the full application runtime to exercise live circuit healing.");
    Ok(())
}
