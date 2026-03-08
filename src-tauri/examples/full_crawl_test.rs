use anyhow::Result;
use crawli_lib::frontier::{CrawlOptions, CrawlerFrontier};

#[tokio::main(flavor = "multi_thread", worker_threads = 8)]
async fn main() -> Result<()> {
    let target_url = "http://ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion/site/view?uuid=c9d2ba19-6aa1-3087-8773-f63d023179ed";
    let options = CrawlOptions {
        listing: true,
        sizes: true,
        download: false,
        circuits: Some(120),
        daemons: Some(12),
        agnostic_state: false,
        resume: false,
        resume_index: None,
        mega_password: None,
    };

    let frontier = CrawlerFrontier::new(
        None,
        target_url.to_string(),
        12,
        true,
        (9051..=9062).collect(),
        Vec::new(),
        options,
        None,
    );

    println!("=== FULL SCALE QILIN CRAWLER DIAGNOSTIC ===");
    println!("Target: {}", target_url);
    println!("HTTP client pool: {}", frontier.http_clients.len());
    println!("Worker target: {}", frontier.worker_target());
    println!("This smoke test validates the frontier wiring only.");
    Ok(())
}
