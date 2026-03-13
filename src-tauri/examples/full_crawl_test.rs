use anyhow::Result;
use crawli_lib::frontier::{CrawlOptions, CrawlerFrontier};

#[tokio::main(flavor = "multi_thread", worker_threads = 8)]
async fn main() -> Result<()> {
    let target_url = "http://25j35d6uf37tvfqt5pmz457yicgu35yhizojqxbfzv33dni2d73q3oad.onion/80349839-d06f-41a8-b954-3602fe60725a/";
    let options = CrawlOptions {
        listing: true,
        sizes: true,
        download: false,
        circuits: Some(120),
        agnostic_state: false,
        resume: false,
        resume_index: None,
        mega_password: None,
        stealth_ramp: true, parallel_download: false,
            download_mode: crawli_lib::frontier::DownloadMode::Medium,
            force_clearnet: false,
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
