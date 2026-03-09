/// Engine Integration Test
/// Tests the CrawlerFrontier, Adapter matching, and crawl execution
/// without requiring a Tauri AppHandle — pure Rust backend validation.
use std::sync::Arc;
use std::time::Instant;

use crawli_lib::adapters::{AdapterRegistry, SiteFingerprint};
use crawli_lib::frontier::{CrawlOptions, CrawlerFrontier};
use reqwest::header::HeaderMap;

fn sanitize_for_wal(url: &str) -> String {
    url.chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect()
}

#[test]
fn test_adapter_support_catalog_shape() {
    let catalog = crawli_lib::adapters::support_catalog();
    assert!(
        catalog.len() >= 8,
        "Support catalog should include all registered adapters"
    );

    let names: Vec<&str> = catalog.iter().map(|item| item.name).collect();
    assert!(names.contains(&"WorldLeaks SPA"));
    assert!(names.contains(&"DragonForce Iframe SPA"));
    assert!(names.contains(&"Play Ransomware (Autoindex)"));
    assert!(names.contains(&"Generic Autoindex"));

    let play = catalog
        .iter()
        .find(|item| item.id == "play")
        .expect("Play adapter should exist");
    assert!(
        !play.tested_for.is_empty(),
        "Play adapter should expose test coverage metadata"
    );
    assert!(
        !play.sample_urls.is_empty(),
        "Play adapter should expose at least one sample URL"
    );

    let lockbit = catalog
        .iter()
        .find(|item| item.id == "lockbit")
        .expect("LockBit adapter should exist");
    assert!(
        !lockbit.sample_urls.is_empty(),
        "LockBit adapter should expose at least one sample URL"
    );
    assert_eq!(
        lockbit.support_level, "Full Crawl",
        "LockBit support catalog entry must reflect crawler delegation support"
    );

    let nu_server = catalog
        .iter()
        .find(|item| item.id == "nu_server")
        .expect("Nu Server adapter should exist");
    assert_eq!(
        nu_server.support_level, "Full Crawl",
        "Nu Server support catalog entry must reflect crawler delegation support"
    );
}

#[tokio::test]
async fn test_runtime_plugin_manifest_matches_without_rebuild() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let plugin_dir = std::env::temp_dir().join(format!("crawli_plugin_fixture_{unique}"));
    std::fs::create_dir_all(&plugin_dir).expect("plugin dir should be creatable");
    let manifest_path = plugin_dir.join("fixture_autoindex.json");
    std::fs::write(
        &manifest_path,
        r#"{
            "id": "fixture_autoindex",
            "name": "Fixture Autoindex Plugin",
            "host_pipeline": "autoindex",
            "known_domains": ["fixture-plugin.onion"],
            "url_contains_any": ["/fixture"],
            "body_contains_all": ["Index of /fixture/", "Special Fixture Marker"]
        }"#,
    )
    .expect("manifest should be writable");

    let registry = crawli_lib::adapters::AdapterRegistry::with_plugin_dir(Some(&plugin_dir));
    let fp = SiteFingerprint {
        url: "http://fixture-plugin.onion/fixture".to_string(),
        status: 200,
        headers: HeaderMap::new(),
        body: "Special Fixture Marker\nIndex of /fixture/".to_string(),
    };

    let adapter = registry
        .determine_adapter(&fp)
        .await
        .expect("plugin adapter should match");
    assert_eq!(adapter.name(), "Fixture Autoindex Plugin");

    let _ = std::fs::remove_dir_all(plugin_dir);
}

#[tokio::test]
async fn test_frontier_initialization() {
    let options = CrawlOptions::default();
    let frontier = CrawlerFrontier::new(
        None,
        "http://example.onion/test".to_string(),
        4,
        true,
        vec![9051, 9052, 9053, 9054],
        vec![],
        options,
        None, // Persistent ledger bound
    );

    // Validate connection pool size: 4 daemons * 30 circuits = 120
    assert_eq!(
        frontier.http_clients.len(),
        120,
        "Expected 120 persistent Tor circuit clients"
    );
    assert_eq!(frontier.num_daemons, 4);
    assert!(frontier.is_onion);
    println!(
        "✅ Frontier initialized: {} clients across {} daemons",
        frontier.http_clients.len(),
        frontier.num_daemons
    );
    assert_eq!(
        frontier.worker_target(),
        frontier.max_worker_permits,
        "Onion listing crawl should pin worker target to configured max circuits"
    );
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
        vec![],
        options,
        None, // No persistent ledger for test stubs
    );

    // Clearnet: 1 client per daemon (breaks after first), minimum 1
    assert!(
        !frontier.http_clients.is_empty(),
        "Expected at least 1 clearnet client"
    );
    assert!(!frontier.is_onion);
    println!(
        "✅ Clearnet frontier: {} clients",
        frontier.http_clients.len()
    );
}

#[tokio::test]
async fn test_onion_listing_worker_target_stays_pinned_after_failures() {
    let options = CrawlOptions {
        listing: true,
        sizes: true,
        download: false,
        circuits: Some(120),
        daemons: Some(4),
        agnostic_state: false,
        resume: false,
        resume_index: None,
        mega_password: None,
        stealth_ramp: false,
    };
    let frontier = CrawlerFrontier::new(
        None,
        "http://example.onion/deep".to_string(),
        4,
        true,
        vec![9051, 9052, 9053, 9054],
        vec![],
        options,
        None, // Persistent ledger bound
    );

    // Force AIMD failure path repeatedly; onion listing mode should still keep full fanout.
    for _ in 0..12 {
        frontier.record_failure(0);
    }

    assert_eq!(
        frontier.worker_target(),
        frontier.max_worker_permits,
        "Worker target should remain pinned to full configured circuits for onion crawl"
    );
}

#[tokio::test]
async fn test_frontier_fresh_crawl_ignores_stale_wal_by_default() {
    let unique = format!(
        "http://wal-reset.onion/{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    let wal_name = sanitize_for_wal(&unique);
    let wal_path = std::path::PathBuf::from(format!("/tmp/crawli_{}.wal", wal_name));
    let _ = std::fs::write(&wal_path, b"http://wal-reset.onion/preseed\n");

    let frontier = CrawlerFrontier::new(
        None,
        unique,
        1,
        true,
        vec![9051],
        vec![],
        CrawlOptions::default(),
        None, // Persistent ledger bound
    );
    assert_eq!(
        frontier.visited_count(),
        0,
        "Fresh crawl should ignore stale WAL unless resume is explicitly enabled"
    );

    let _ = std::fs::remove_file(wal_path);
}

#[tokio::test]
async fn test_bloom_filter_dedup() {
    let frontier = CrawlerFrontier::new(
        None,
        "http://example.onion".to_string(),
        1,
        true,
        vec![9051],
        vec![],
        CrawlOptions::default(),
        None, // Persistent ledger bound
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
        vec![],
        CrawlOptions::default(),
        None,
    );

    for _ in 0..240 {
        let _client = frontier.get_client();
    }

    let counter_val = frontier
        .client_counter
        .load(std::sync::atomic::Ordering::Relaxed);
    assert_eq!(counter_val, 240);
    println!(
        "✅ Round-robin cycling: {} get_client() calls processed",
        counter_val
    );
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
    let adapter: Option<&dyn crawli_lib::adapters::CrawlerAdapter> =
        registry.determine_adapter(&inc_fp).await;
    assert!(adapter.is_some(), "INC Ransom adapter should match");
    println!("✅ INC Ransom adapter matched: {}", adapter.unwrap().name());

    // --- Play ---
    let play_fp = SiteFingerprint {
        url: "http://b3pzp6qwelgeygmzn6awkduym6s4gxh6htwxuxeydrziwzlx63zergyd.onion/FALOp"
            .to_string(),
        status: 200,
        headers: HeaderMap::new(),
        body: "Index of /FALOp/\n<a href=\"2 Sally Personal.part01.rar\">".to_string(),
    };
    let adapter: Option<&dyn crawli_lib::adapters::CrawlerAdapter> =
        registry.determine_adapter(&play_fp).await;
    assert!(adapter.is_some(), "Play adapter should match");
    println!("✅ Play adapter matched: {}", adapter.unwrap().name());

    // --- Pear ---
    let pear_fp = SiteFingerprint {
        url: "http://m3wwhkus4dxbnxbtihexlyd2cv63qrvex6jiebc4vqe22kg2z3udebid.onion/sdeb.org/"
            .to_string(),
        status: 200,
        headers: HeaderMap::new(),
        body: "<html>Some content</html>".to_string(),
    };
    let adapter: Option<&dyn crawli_lib::adapters::CrawlerAdapter> =
        registry.determine_adapter(&pear_fp).await;
    assert!(adapter.is_some(), "Pear adapter should match");
    println!("✅ Pear adapter matched: {}", adapter.unwrap().name());

    // --- WorldLeaks ---
    let wl_fp = SiteFingerprint {
        url: "http://worldleaks.onion".to_string(),
        status: 200,
        headers: HeaderMap::new(),
        body: "<html><app-root></app-root>worldleaks</html>".to_string(),
    };
    let adapter: Option<&dyn crawli_lib::adapters::CrawlerAdapter> =
        registry.determine_adapter(&wl_fp).await;
    assert!(adapter.is_some(), "WorldLeaks adapter should match");
    println!("✅ WorldLeaks adapter matched: {}", adapter.unwrap().name());

    // --- DragonForce ---
    let df_fp = SiteFingerprint {
        url: "http://dragonforce.onion".to_string(),
        status: 200,
        headers: HeaderMap::new(),
        body: "<html>fsguest dragonforce</html>".to_string(),
    };
    let adapter: Option<&dyn crawli_lib::adapters::CrawlerAdapter> =
        registry.determine_adapter(&df_fp).await;
    assert!(adapter.is_some(), "DragonForce adapter should match");
    println!(
        "✅ DragonForce adapter matched: {}",
        adapter.unwrap().name()
    );

    // --- LockBit ---
    let lockbit_fp = SiteFingerprint {
        url: "http://lockbit.onion".to_string(),
        status: 200,
        headers: HeaderMap::new(),
        body: "<!-- Start of nginx output --><html>lockbit</html>".to_string(),
    };
    let adapter: Option<&dyn crawli_lib::adapters::CrawlerAdapter> =
        registry.determine_adapter(&lockbit_fp).await;
    assert!(adapter.is_some(), "LockBit adapter should match");
    assert_eq!(adapter.unwrap().name(), "LockBit Embedded Nginx");
    println!("✅ LockBit adapter matched: {}", adapter.unwrap().name());

    // --- LockBit direct artifact URL (binary placeholder body) ---
    let lockbit_direct_fp = SiteFingerprint {
        url: "http://lockbit6vhrjaqzsdj6pqalyideigxv4xycfeyunpx35znogiwmojnid.onion/secret/sample/archive.7z".to_string(),
        status: 200,
        headers: HeaderMap::new(),
        body: "[BINARY_OR_ARCHIVE_DATA]".to_string(),
    };
    let adapter: Option<&dyn crawli_lib::adapters::CrawlerAdapter> =
        registry.determine_adapter(&lockbit_direct_fp).await;
    assert!(
        adapter.is_some(),
        "LockBit direct artifact URL should still resolve to LockBit adapter"
    );
    assert_eq!(adapter.unwrap().name(), "LockBit Embedded Nginx");
    println!(
        "✅ LockBit direct artifact adapter matched: {}",
        adapter.unwrap().name()
    );

    // --- Nu Server ---
    let nu_fp = SiteFingerprint {
        url: "http://nu-server.onion".to_string(),
        status: 200,
        headers: HeaderMap::new(),
        body: "# acct: root\n# srvinf: nu\n".to_string(),
    };
    let adapter: Option<&dyn crawli_lib::adapters::CrawlerAdapter> =
        registry.determine_adapter(&nu_fp).await;
    assert!(adapter.is_some(), "Nu Server adapter should match");
    assert_eq!(adapter.unwrap().name(), "Nu Server");
    println!("✅ Nu Server adapter matched: {}", adapter.unwrap().name());

    // --- Qilin Pre-Authentication Intelligence ---
    let qilin_fp = SiteFingerprint {
        url: "http://unknown-domain-for-qilin.onion/".to_string(),
        status: 200,
        headers: HeaderMap::new(),
        body: "<html><div class=\"page-header-title\">QData</div><input class=\"form-control\" type=\"text\" readonly value=\"ijzn3sicrcy7guixkzjkib4ukbiilwc3xhnmby4mcbccnsd7j2rekvqd.onion\"></html>".to_string(),
    };
    let adapter: Option<&dyn crawli_lib::adapters::CrawlerAdapter> =
        registry.determine_adapter(&qilin_fp).await;
    assert!(
        adapter.is_some(),
        "Qilin adapter should match unknown domain via DOM heuristic"
    );
    assert_eq!(adapter.unwrap().name(), "Qilin Nginx Autoindex / CMS");
    println!(
        "✅ Qilin Autonomous Detection matched: {}",
        adapter.unwrap().name()
    );

    // --- Autoindex fallback ---
    let ai_fp = SiteFingerprint {
        url: "http://unknown.onion/files/".to_string(),
        status: 200,
        headers: HeaderMap::new(),
        body: "<html>Index of /files/</html>".to_string(),
    };
    let adapter: Option<&dyn crawli_lib::adapters::CrawlerAdapter> =
        registry.determine_adapter(&ai_fp).await;
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
        vec![],
        CrawlOptions {
            listing: false,
            sizes: false,
            download: false,
            circuits: None,
            daemons: None,
            agnostic_state: false,
            resume: false,
            resume_index: None,
            mega_password: None,
            stealth_ramp: false,
        },
        None, // Persistent ledger bound
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
        vec![],
        CrawlOptions {
            listing: true,
            sizes: true,
            download: true,
            circuits: None,
            daemons: None,
            agnostic_state: false,
            resume: false,
            resume_index: None,
            mega_password: None,
            stealth_ramp: false,
        },
        None, // Persistent ledger bound
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
        vec![],
        CrawlOptions::default(),
        None,
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
    println!(
        "✅ Bloom stress test: 100,000 URLs indexed in {:?}",
        elapsed
    );
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
    println!(
        "✅ Bloom dedup verification: 100,000 duplicates rejected in {:?}",
        elapsed2
    );
}

#[tokio::test]
async fn test_concurrent_worker_simulation() {
    let frontier = Arc::new(CrawlerFrontier::new(
        None,
        "http://concurrent.onion".to_string(),
        4,
        true,
        vec![9051, 9052, 9053, 9054],
        vec![],
        CrawlOptions::default(),
        None, // Persistent ledger bound
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
    assert_eq!(
        total_discovered, 12_000,
        "120 workers * 100 items = 12,000 unique URLs"
    );
    println!(
        "✅ Concurrent stress test: 120 workers discovered {} URLs in {:?}",
        total_discovered, elapsed
    );
}

#[test]
fn test_dragonforce_nextjs_predictive_hydration() {
    let mock_html = r#"
    <!DOCTYPE html>
    <html>
    <body>
        <script id="__NEXT_DATA__" type="application/json">
        {
          "props": {
            "pageProps": {
              "data": [
                {
                  "name": "internal_accounting",
                  "type": "dir",
                  "size": 0,
                  "path": "/internal_accounting"
                },
                {
                  "name": "dragonforce_manifest.json",
                  "type": "file",
                  "size": 55621,
                  "path": "/internal_accounting/dragonforce_manifest.json"
                }
              ]
            }
          }
        }
        </script>
    </body>
    </html>
    "#;

    // Simulate arriving from a deeply nested API URL that contains a path parameter
    let simulated_parent_url = "http://fsguest.onion/?path=RJZ-APP1/G/01&token=ABC123XYZ";
    let entries = crawli_lib::adapters::dragonforce::parse_dragonforce_fsguest(
        mock_html,
        "fsguest.onion",
        simulated_parent_url,
    );

    assert_eq!(
        entries.len(),
        2,
        "Expected exactly 2 entries extracted from the NextJS mock JSON"
    );

    let folder = entries
        .iter()
        .find(|e| e.entry_type == crawli_lib::adapters::EntryType::Folder)
        .unwrap();
    let file = entries
        .iter()
        .find(|e| e.entry_type == crawli_lib::adapters::EntryType::File)
        .unwrap();

    // Verify Path Context Preservation and HTML routing (/?path=...)
    assert_eq!(folder.path, "/internal_accounting");
    assert!(
        folder.raw_url.contains("/?path="),
        "Folders must route to HTML endpoint"
    );
    assert!(
        !folder.raw_url.contains("/download?path="),
        "Folders must not map to the download API"
    );

    // Verify API Segregation (/download?path=...)
    assert_eq!(file.path, "/internal_accounting/dragonforce_manifest.json");
    assert_eq!(file.size_bytes, Some(55621));
    assert!(
        file.raw_url.contains("/download?path="),
        "Files MUST route to the backend download API"
    );
    assert!(
        !file.raw_url.contains("/?path="),
        "Files must not map to the HTML viewer endpoint"
    );

    println!("✅ DragonForce Predictive State Hydrator parsed NextJS DOM correctly.");
}
