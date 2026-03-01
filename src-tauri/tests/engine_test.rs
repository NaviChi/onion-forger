/// Engine Integration Test
/// Tests the CrawlerFrontier, Adapter matching, and crawl execution
/// without requiring a Tauri AppHandle — pure Rust backend validation.

use std::sync::Arc;
use std::time::Instant;

use crawli_lib::frontier::{CrawlOptions, CrawlerFrontier};
use crawli_lib::adapters::{AdapterRegistry, SiteFingerprint};
use reqwest::header::HeaderMap;

#[tokio::test]
async fn test_frontier_initialization() {
    let options = CrawlOptions::default();
    let frontier = CrawlerFrontier::new(
        None,
        "http://example.onion/test".to_string(),
        4,
        true,
        vec![9051, 9052, 9053, 9054],
        options,
    );

    // Validate connection pool size: 4 daemons * 30 circuits = 120
    assert_eq!(frontier.http_clients.len(), 120, "Expected 120 persistent Tor circuit clients");
    assert_eq!(frontier.num_daemons, 4);
    assert!(frontier.is_onion);
    println!("✅ Frontier initialized: {} clients across {} daemons", frontier.http_clients.len(), frontier.num_daemons);
}

#[tokio::test]
async fn test_frontier_clearnet_initialization() {
    let options = CrawlOptions::default();
    let frontier = CrawlerFrontier::new(
        None,
        "https://example.com/test".to_string(),
        4,
        false,
        vec![],
        options,
    );

    // Clearnet: 1 client per daemon (breaks after first), minimum 1
    assert!(frontier.http_clients.len() >= 1, "Expected at least 1 clearnet client");
    assert!(!frontier.is_onion);
    println!("✅ Clearnet frontier: {} clients", frontier.http_clients.len());
}

#[tokio::test]
async fn test_bloom_filter_dedup() {
    let frontier = CrawlerFrontier::new(
        None,
        "http://example.onion".to_string(),
        1,
        true,
        vec![9051],
        CrawlOptions::default(),
    );

    assert!(frontier.mark_visited("http://example.onion/page1"));
    assert!(!frontier.mark_visited("http://example.onion/page1"));
    assert!(frontier.mark_visited("http://example.onion/page2"));

    println!("✅ Bloom filter deduplication working correctly");
}

#[tokio::test]
async fn test_client_round_robin() {
    let frontier = CrawlerFrontier::new(
        None,
        "http://example.onion".to_string(),
        4,
        true,
        vec![9051, 9052, 9053, 9054],
        CrawlOptions::default(),
    );

    for _ in 0..240 {
        let _client = frontier.get_client();
    }

    let counter_val = frontier.client_counter.load(std::sync::atomic::Ordering::Relaxed);
    assert_eq!(counter_val, 240);
    println!("✅ Round-robin cycling: {} get_client() calls processed", counter_val);
}

#[tokio::test]
async fn test_adapter_fingerprint_matching() {
    let registry = AdapterRegistry::new();

    // --- INC Ransom ---
    let inc_fp = SiteFingerprint {
        url: "http://incblog6qu4y4mm4zvw5nrmue6qbwtgjsxpw6b7ixzssu36tsajldoad.onion/blog/disclosures/698d5c538f1d14b7436dd63b".to_string(),
        status: 200,
        headers: HeaderMap::new(),
        body: "<html>INC Ransom Blog</html>".to_string(),
    };
    let adapter: Option<&dyn crawli_lib::adapters::CrawlerAdapter> = registry.determine_adapter(&inc_fp).await;
    assert!(adapter.is_some(), "INC Ransom adapter should match");
    println!("✅ INC Ransom adapter matched: {}", adapter.unwrap().name());

    // --- Play ---
    let play_fp = SiteFingerprint {
        url: "http://b3pzp6qwelgeygmzn6awkduym6s4gxh6htwxuxeydrziwzlx63zergyd.onion/FALOp".to_string(),
        status: 200,
        headers: HeaderMap::new(),
        body: "Index of /FALOp/\n<a href=\"2 Sally Personal.part01.rar\">".to_string(),
    };
    let adapter: Option<&dyn crawli_lib::adapters::CrawlerAdapter> = registry.determine_adapter(&play_fp).await;
    assert!(adapter.is_some(), "Play adapter should match");
    println!("✅ Play adapter matched: {}", adapter.unwrap().name());

    // --- Pear ---
    let pear_fp = SiteFingerprint {
        url: "http://m3wwhkus4dxbnxbtihexlyd2cv63qrvex6jiebc4vqe22kg2z3udebid.onion/sdeb.org/".to_string(),
        status: 200,
        headers: HeaderMap::new(),
        body: "<html>Some content</html>".to_string(),
    };
    let adapter: Option<&dyn crawli_lib::adapters::CrawlerAdapter> = registry.determine_adapter(&pear_fp).await;
    assert!(adapter.is_some(), "Pear adapter should match");
    println!("✅ Pear adapter matched: {}", adapter.unwrap().name());

    // --- WorldLeaks ---
    let wl_fp = SiteFingerprint {
        url: "http://worldleaks.onion".to_string(),
        status: 200,
        headers: HeaderMap::new(),
        body: "<html><app-root></app-root>worldleaks</html>".to_string(),
    };
    let adapter: Option<&dyn crawli_lib::adapters::CrawlerAdapter> = registry.determine_adapter(&wl_fp).await;
    assert!(adapter.is_some(), "WorldLeaks adapter should match");
    println!("✅ WorldLeaks adapter matched: {}", adapter.unwrap().name());

    // --- DragonForce ---
    let df_fp = SiteFingerprint {
        url: "http://dragonforce.onion".to_string(),
        status: 200,
        headers: HeaderMap::new(),
        body: "<html>fsguest dragonforce</html>".to_string(),
    };
    let adapter: Option<&dyn crawli_lib::adapters::CrawlerAdapter> = registry.determine_adapter(&df_fp).await;
    assert!(adapter.is_some(), "DragonForce adapter should match");
    println!("✅ DragonForce adapter matched: {}", adapter.unwrap().name());

    // --- Autoindex fallback ---
    let ai_fp = SiteFingerprint {
        url: "http://unknown.onion/files/".to_string(),
        status: 200,
        headers: HeaderMap::new(),
        body: "<html>Index of /files/</html>".to_string(),
    };
    let adapter: Option<&dyn crawli_lib::adapters::CrawlerAdapter> = registry.determine_adapter(&ai_fp).await;
    assert!(adapter.is_some(), "Autoindex fallback should match");
    println!("✅ Autoindex fallback matched: {}", adapter.unwrap().name());
}

#[tokio::test]
async fn test_crawl_options_propagation() {
    let frontier = CrawlerFrontier::new(
        None,
        "http://test.onion".to_string(),
        4,
        true,
        vec![9051, 9052, 9053, 9054],
        CrawlOptions { listing: false, sizes: false, download: false, circuits: None },
    );
    assert!(!frontier.active_options.listing);
    assert!(!frontier.active_options.sizes);
    assert!(!frontier.active_options.download);
    println!("✅ CrawlOptions (all off) propagated correctly");

    let frontier2 = CrawlerFrontier::new(
        None,
        "http://test.onion".to_string(),
        4,
        true,
        vec![9051, 9052, 9053, 9054],
        CrawlOptions { listing: true, sizes: true, download: true, circuits: None },
    );
    assert!(frontier2.active_options.listing);
    assert!(frontier2.active_options.sizes);
    assert!(frontier2.active_options.download);
    println!("✅ CrawlOptions (all on) propagated correctly");
}

#[tokio::test]
async fn test_high_volume_bloom_filter_stress() {
    let start = Instant::now();
    let frontier = CrawlerFrontier::new(
        None,
        "http://stress.onion".to_string(),
        4,
        true,
        vec![9051, 9052, 9053, 9054],
        CrawlOptions::default(),
    );

    // Insert 100,000 unique URLs
    let mut unique_count = 0;
    for i in 0..100_000 {
        let url = format!("http://stress.onion/path/to/file_{}.dat", i);
        if frontier.mark_visited(&url) {
            unique_count += 1;
        }
    }
    let elapsed = start.elapsed();
    assert_eq!(unique_count, 100_000, "All 100k URLs should be unique");
    println!("✅ Bloom stress test: 100,000 URLs indexed in {:?}", elapsed);
    assert!(elapsed.as_millis() < 5000, "Should complete under 5s");

    // Re-insert all 100k — all duplicates
    let start2 = Instant::now();
    let mut dup_count = 0;
    for i in 0..100_000 {
        let url = format!("http://stress.onion/path/to/file_{}.dat", i);
        if !frontier.mark_visited(&url) {
            dup_count += 1;
        }
    }
    let elapsed2 = start2.elapsed();
    assert_eq!(dup_count, 100_000, "All 100k re-inserts should be deduped");
    println!("✅ Bloom dedup verification: 100,000 duplicates rejected in {:?}", elapsed2);
}

#[tokio::test]
async fn test_concurrent_worker_simulation() {
    let frontier = Arc::new(CrawlerFrontier::new(
        None,
        "http://concurrent.onion".to_string(),
        4,
        true,
        vec![9051, 9052, 9053, 9054],
        CrawlOptions::default(),
    ));

    let start = Instant::now();
    let mut handles = Vec::new();

    // 120 concurrent workers
    for worker_id in 0..120u32 {
        let f = frontier.clone();
        handles.push(tokio::spawn(async move {
            let mut discovered = 0u32;
            for i in 0..100u32 {
                let url = format!("http://concurrent.onion/worker_{}/item_{}", worker_id, i);
                let _client = f.get_client();
                let _permit: Option<tokio::sync::OwnedSemaphorePermit> = None; // Skip actual semaphore in test
                if f.mark_visited(&url) {
                    discovered += 1;
                }
            }
            discovered
        }));
    }

    let mut total_discovered: u32 = 0;
    for handle in handles {
        total_discovered += handle.await.unwrap();
    }

    let elapsed = start.elapsed();
    assert_eq!(total_discovered, 12_000, "120 workers * 100 items = 12,000 unique URLs");
    println!("✅ Concurrent stress test: 120 workers discovered {} URLs in {:?}", total_discovered, elapsed);
}
