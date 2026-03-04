use std::path::PathBuf;
/// Play Ransomware End-to-End Integration Test
/// Tests: Adapter matching → Crawl simulation → Download scaffolding → Folder structure verification
/// Simulates full pipeline without Tauri AppHandle dependency
use std::sync::Arc;
use std::time::Instant;

use crawli_lib::adapters::{AdapterRegistry, EntryType, FileEntry, SiteFingerprint};
use crawli_lib::frontier::{CrawlOptions, CrawlerFrontier};
use reqwest::header::HeaderMap;

/// Helper: Build a Play-style set of FileEntries exactly as the adapter would produce
fn simulate_play_crawl(listing: bool, sizes: bool) -> Vec<FileEntry> {
    let mut files = Vec::new();
    if !listing {
        return files;
    }

    for i in 1..=11 {
        let filename = format!("2 Sally Personal.part{:02}.rar", i);
        let size = if sizes {
            if i == 11 {
                Some(60844542)
            } else {
                Some(524288000)
            }
        } else {
            None
        };

        files.push(FileEntry {
            path: format!("/FALOp/{}", filename),
            size_bytes: size,
            entry_type: EntryType::File,
            raw_url: format!(
                "http://b3pzp6qwelgeygmzn6awkduym6s4gxh6htwxuxeydrziwzlx63zergyd.onion/FALOp/{}",
                filename
            ),
        });
    }
    files
}

/// Helper: scaffold_download logic extracted for test use (mirrors lib.rs logic without AppHandle)
async fn test_scaffold_download(entries: &[FileEntry], output_dir: &str) -> anyhow::Result<u32> {
    let base = PathBuf::from(output_dir);
    tokio::fs::create_dir_all(&base).await?;

    let mut written: u32 = 0;

    for entry in entries.iter() {
        let relative = entry.path.trim_start_matches('/');
        if relative.is_empty() {
            continue;
        }

        let full_path = base.join(relative);

        match entry.entry_type {
            EntryType::Folder => {
                tokio::fs::create_dir_all(&full_path).await?;
                let gitkeep = full_path.join(".gitkeep");
                if !gitkeep.exists() {
                    tokio::fs::write(&gitkeep, b"").await?;
                }
            }
            EntryType::File => {
                if let Some(parent) = full_path.parent() {
                    tokio::fs::create_dir_all(parent).await?;
                }
                if !full_path.exists() {
                    tokio::fs::write(&full_path, b"").await?;
                }
                // Sidecar meta
                let meta_path = PathBuf::from(format!("{}.onionforge.meta", full_path.display()));
                let size_str = entry
                    .size_bytes
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "0".to_string());
                let meta_content = format!("url={}\nsize={}\ntype=file\n", entry.raw_url, size_str);
                tokio::fs::write(&meta_path, meta_content.as_bytes()).await?;
            }
        }
        written += 1;
    }

    // Write manifest
    let manifest_path = base.join("_onionforge_manifest.txt");
    let mut manifest = String::new();
    manifest.push_str("# OnionForge Download Manifest\n");
    manifest.push_str(&format!("# Total Entries: {}\n\n", entries.len()));
    for entry in entries {
        let type_tag = match entry.entry_type {
            EntryType::Folder => "DIR ",
            EntryType::File => "FILE",
        };
        let size_tag = entry
            .size_bytes
            .map(|s| format!("{} B", s))
            .unwrap_or_else(|| "0 B".to_string());
        manifest.push_str(&format!(
            "{} {:>12}  {}  {}\n",
            type_tag, size_tag, entry.path, entry.raw_url
        ));
    }
    tokio::fs::write(&manifest_path, manifest.as_bytes()).await?;

    Ok(written)
}

// ═══════════════════════════════════════════════════════════════
// TEST 1: Play adapter fingerprint matching (all 3 trigger conditions)
// ═══════════════════════════════════════════════════════════════
#[tokio::test]
async fn test_play_fingerprint_by_onion_hash() {
    let registry = AdapterRegistry::new();
    let fp = SiteFingerprint {
        url: "http://b3pzp6qwelgeygmzn6awkduym6s4gxh6htwxuxeydrziwzlx63zergyd.onion/FALOp"
            .to_string(),
        status: 200,
        headers: HeaderMap::new(),
        body: "<html>Some random content</html>".to_string(), // No "Index of /" in body
    };
    let adapter: Option<&dyn crawli_lib::adapters::CrawlerAdapter> =
        registry.determine_adapter(&fp).await;
    assert!(adapter.is_some());
    assert_eq!(adapter.unwrap().name(), "Play Ransomware (Autoindex)");
    println!("✅ Play matched by onion hash (body irrelevant)");
}

#[tokio::test]
async fn test_play_fingerprint_by_url_path() {
    let registry = AdapterRegistry::new();
    let fp = SiteFingerprint {
        url: "http://some-other-host.onion/FALOp".to_string(),
        status: 200,
        headers: HeaderMap::new(),
        body: "".to_string(),
    };
    let adapter: Option<&dyn crawli_lib::adapters::CrawlerAdapter> =
        registry.determine_adapter(&fp).await;
    assert!(adapter.is_some());
    assert_eq!(adapter.unwrap().name(), "Play Ransomware (Autoindex)");
    println!("✅ Play matched by /FALOp URL path");
}

#[tokio::test]
async fn test_play_fingerprint_by_body() {
    let registry = AdapterRegistry::new();
    let fp = SiteFingerprint {
        url: "http://totally-different.onion/files/".to_string(),
        status: 200,
        headers: HeaderMap::new(),
        body: "Index of /FALOp/\n<a href=\"test.rar\">".to_string(),
    };
    let adapter: Option<&dyn crawli_lib::adapters::CrawlerAdapter> =
        registry.determine_adapter(&fp).await;
    assert!(adapter.is_some());
    assert_eq!(adapter.unwrap().name(), "Play Ransomware (Autoindex)");
    println!("✅ Play matched by body content: 'Index of /FALOp/'");
}

// ═══════════════════════════════════════════════════════════════
// TEST 2: Play crawl with listing=ON, sizes=ON
// ═══════════════════════════════════════════════════════════════
#[tokio::test]
async fn test_play_crawl_listing_and_sizes() {
    let start = Instant::now();
    let files = simulate_play_crawl(true, true);
    let elapsed = start.elapsed();

    assert_eq!(files.len(), 11, "Play should produce 11 .rar entries");

    // Verify first 10 are 524288000 bytes (500 MB)
    for file in &files[0..10] {
        assert_eq!(file.size_bytes, Some(524288000));
        assert_eq!(file.entry_type, EntryType::File);
        assert!(file.path.starts_with("/FALOp/"));
        assert!(file.raw_url.contains(".onion/FALOp/"));
    }

    // Verify last file is 60844542 bytes (~58 MB)
    assert_eq!(files[10].size_bytes, Some(60844542));
    assert!(files[10].path.contains("part11.rar"));

    // Total size calculation
    let total_bytes: u64 = files.iter().filter_map(|f| f.size_bytes).sum();
    let total_gb = total_bytes as f64 / 1_073_741_824.0;
    println!(
        "✅ Play crawl (listing+sizes): {} files, {:.2} GB total, completed in {:?}",
        files.len(),
        total_gb,
        elapsed
    );
    assert!((total_gb - 4.93).abs() < 0.1, "Expected ~4.93 GB total");
}

// ═══════════════════════════════════════════════════════════════
// TEST 3: Play crawl with listing=ON, sizes=OFF (speed optimization)
// ═══════════════════════════════════════════════════════════════
#[tokio::test]
async fn test_play_crawl_listing_no_sizes() {
    let files = simulate_play_crawl(true, false);
    assert_eq!(files.len(), 11);

    for file in &files {
        assert_eq!(file.size_bytes, None, "Sizes should be None when disabled");
    }
    println!(
        "✅ Play crawl (listing only, no sizes): {} files, all sizes=None",
        files.len()
    );
}

// ═══════════════════════════════════════════════════════════════
// TEST 4: Play crawl with listing=OFF (skip everything)
// ═══════════════════════════════════════════════════════════════
#[tokio::test]
async fn test_play_crawl_listing_off() {
    let files = simulate_play_crawl(false, true);
    assert_eq!(files.len(), 0, "No files when listing is disabled");
    println!("✅ Play crawl (listing=OFF): 0 files correctly returned");
}

// ═══════════════════════════════════════════════════════════════
// TEST 5: Full download scaffold to temp directory
// ═══════════════════════════════════════════════════════════════
#[tokio::test]
async fn test_play_full_download_scaffold() {
    let tmp_dir = std::env::temp_dir().join("onionforge_play_test");
    // Clean up from previous runs
    let _ = tokio::fs::remove_dir_all(&tmp_dir).await;

    let files = simulate_play_crawl(true, true);
    let start = Instant::now();
    let count = test_scaffold_download(&files, tmp_dir.to_str().unwrap())
        .await
        .unwrap();
    let elapsed = start.elapsed();

    assert_eq!(count, 11);
    println!(
        "✅ Scaffold download: {} files written in {:?}",
        count, elapsed
    );

    // Verify directory structure
    let falop_dir = tmp_dir.join("FALOp");
    assert!(falop_dir.exists(), "FALOp directory should exist");
    assert!(falop_dir.is_dir(), "FALOp should be a directory");

    // Verify all 11 placeholder files exist
    for i in 1..=11 {
        let filename = format!("2 Sally Personal.part{:02}.rar", i);
        let file_path = falop_dir.join(&filename);
        assert!(file_path.exists(), "File should exist: {}", filename);

        // Verify it's a 0-byte placeholder
        let metadata = tokio::fs::metadata(&file_path).await.unwrap();
        assert_eq!(metadata.len(), 0, "Placeholder should be 0 bytes");

        // Verify .onionforge.meta sidecar
        let meta_path = PathBuf::from(format!("{}.onionforge.meta", file_path.display()));
        assert!(
            meta_path.exists(),
            "Meta sidecar should exist for: {}",
            filename
        );

        let meta_content = tokio::fs::read_to_string(&meta_path).await.unwrap();
        assert!(meta_content.contains("url="), "Meta should contain url");
        assert!(meta_content.contains("size="), "Meta should contain size");
        assert!(
            meta_content.contains("type=file"),
            "Meta should contain type=file"
        );

        // Verify sizes in meta match expected
        if i == 11 {
            assert!(
                meta_content.contains("size=60844542"),
                "Part11 meta should have 60844542"
            );
        } else {
            assert!(
                meta_content.contains("size=524288000"),
                "Part{:02} meta should have 524288000",
                i
            );
        }
    }
    println!("✅ All 11 files + 11 meta sidecars verified on disk");

    // Verify manifest
    let manifest_path = tmp_dir.join("_onionforge_manifest.txt");
    assert!(manifest_path.exists(), "Manifest should exist");

    let manifest = tokio::fs::read_to_string(&manifest_path).await.unwrap();
    assert!(manifest.contains("# OnionForge Download Manifest"));
    assert!(manifest.contains("Total Entries: 11"));

    let file_lines: Vec<&str> = manifest.lines().filter(|l| l.starts_with("FILE")).collect();
    assert_eq!(file_lines.len(), 11, "Manifest should have 11 FILE entries");
    println!("✅ Manifest verified: {} FILE entries", file_lines.len());

    // Cleanup
    let _ = tokio::fs::remove_dir_all(&tmp_dir).await;
}

// ═══════════════════════════════════════════════════════════════
// TEST 6: Download with sizes=None (0-byte edge case)
// ═══════════════════════════════════════════════════════════════
#[tokio::test]
async fn test_play_download_zero_size_files() {
    let tmp_dir = std::env::temp_dir().join("onionforge_play_zero_test");
    let _ = tokio::fs::remove_dir_all(&tmp_dir).await;

    let files = simulate_play_crawl(true, false); // sizes=OFF → all None
    let count = test_scaffold_download(&files, tmp_dir.to_str().unwrap())
        .await
        .unwrap();
    assert_eq!(count, 11);

    // Verify meta files say size=0
    for i in 1..=11 {
        let filename = format!("2 Sally Personal.part{:02}.rar", i);
        let meta_path = tmp_dir.join(format!("FALOp/{}.onionforge.meta", filename));
        let meta_content = tokio::fs::read_to_string(&meta_path).await.unwrap();
        assert!(
            meta_content.contains("size=0"),
            "Size should be 0 when disabled"
        );
    }
    println!("✅ Zero-size edge case: All 11 meta sidecars correctly show size=0");

    let _ = tokio::fs::remove_dir_all(&tmp_dir).await;
}

// ═══════════════════════════════════════════════════════════════
// TEST 7: Download with mixed folders + files (deeply nested)
// ═══════════════════════════════════════════════════════════════
#[tokio::test]
async fn test_play_download_mixed_entries_deep_nesting() {
    let tmp_dir = std::env::temp_dir().join("onionforge_play_deep_test");
    let _ = tokio::fs::remove_dir_all(&tmp_dir).await;

    let entries = vec![
        FileEntry {
            path: "/target_company".to_string(),
            size_bytes: None,
            entry_type: EntryType::Folder,
            raw_url: "http://play.onion/target_company".to_string(),
        },
        FileEntry {
            path: "/target_company/financials".to_string(),
            size_bytes: None,
            entry_type: EntryType::Folder,
            raw_url: "http://play.onion/target_company/financials".to_string(),
        },
        FileEntry {
            path: "/target_company/financials/2026/Q1/report.xlsx".to_string(),
            size_bytes: Some(45_000),
            entry_type: EntryType::File,
            raw_url: "http://play.onion/target_company/financials/2026/Q1/report.xlsx".to_string(),
        },
        FileEntry {
            path: "/target_company/empty_evidence_folder".to_string(),
            size_bytes: None,
            entry_type: EntryType::Folder,
            raw_url: "http://play.onion/target_company/empty_evidence_folder".to_string(),
        },
        FileEntry {
            path: "".to_string(), // Edge case: empty path
            size_bytes: None,
            entry_type: EntryType::File,
            raw_url: "http://play.onion/".to_string(),
        },
    ];

    let count = test_scaffold_download(&entries, tmp_dir.to_str().unwrap())
        .await
        .unwrap();
    assert_eq!(count, 4, "Empty path should be skipped, 4 items written");

    // Verify deep nesting
    assert!(tmp_dir.join("target_company").is_dir());
    assert!(tmp_dir.join("target_company/financials").is_dir());
    assert!(tmp_dir
        .join("target_company/financials/2026/Q1/report.xlsx")
        .exists());
    assert!(tmp_dir
        .join("target_company/empty_evidence_folder")
        .is_dir());
    assert!(tmp_dir
        .join("target_company/empty_evidence_folder/.gitkeep")
        .exists());

    println!("✅ Deep nesting: 4-level directory hierarchy + empty folder + .gitkeep verified");

    let _ = tokio::fs::remove_dir_all(&tmp_dir).await;
}

// ═══════════════════════════════════════════════════════════════
// TEST 8: Resume safety — don't overwrite existing files
// ═══════════════════════════════════════════════════════════════
#[tokio::test]
async fn test_play_download_resume_no_overwrite() {
    let tmp_dir = std::env::temp_dir().join("onionforge_play_resume_test");
    let _ = tokio::fs::remove_dir_all(&tmp_dir).await;

    // First scaffold
    let files = simulate_play_crawl(true, true);
    test_scaffold_download(&files, tmp_dir.to_str().unwrap())
        .await
        .unwrap();

    // Write real content into part01 to simulate a partial download
    let part01_path = tmp_dir.join("FALOp/2 Sally Personal.part01.rar");
    tokio::fs::write(&part01_path, b"REAL_PARTIAL_CONTENT")
        .await
        .unwrap();

    // Re-scaffold — should NOT overwrite the file with real content
    test_scaffold_download(&files, tmp_dir.to_str().unwrap())
        .await
        .unwrap();

    let content = tokio::fs::read(&part01_path).await.unwrap();
    assert_eq!(
        content, b"REAL_PARTIAL_CONTENT",
        "Existing file should NOT be overwritten"
    );
    println!("✅ Resume safety: Existing files preserved during re-scaffold");

    let _ = tokio::fs::remove_dir_all(&tmp_dir).await;
}

// ═══════════════════════════════════════════════════════════════
// TEST 9: Politeness semaphore bottleneck analysis
// ═══════════════════════════════════════════════════════════════
#[tokio::test]
async fn test_politeness_semaphore_bottleneck() {
    let frontier = Arc::new(CrawlerFrontier::new(
        None,
        "http://play.onion/FALOp".to_string(),
        4,
        true,
        vec![9051, 9052, 9053, 9054],
        CrawlOptions {
            listing: true,
            sizes: true,
            download: true,
            circuits: None,
            daemons: None,
        },
    ));

    // For high-speed onion crawling, semaphore capacity should track circuit pool.
    let permits = frontier.politeness_semaphore.available_permits();
    let total_clients = frontier.http_clients.len();
    let worker_target = frontier.worker_target();

    println!("🔍 Worker Throughput Analysis:");
    println!("   Semaphore permits: {}", permits);
    println!("   Total HTTP clients: {}", total_clients);
    println!("   Initial worker target: {}", worker_target);
    println!(
        "   Client utilization ceiling: {:.1}% ({}/{} active at any time)",
        (permits as f64 / total_clients as f64) * 100.0,
        permits,
        total_clients
    );

    if permits < total_clients / 4 {
        println!(
            "   ⚠️  BOTTLENECK DETECTED: Semaphore ({}) is much smaller than client pool ({})",
            permits, total_clients
        );
        println!("   💡 RECOMMENDATION: Increase politeness_semaphore to at least {} for full throughput", total_clients / 2);
    }

    assert_eq!(permits, 120);
    assert_eq!(total_clients, 120);
    assert_eq!(worker_target, 120);
}

// ═══════════════════════════════════════════════════════════════
// TEST 10: Timing benchmark — full pipeline
// ═══════════════════════════════════════════════════════════════
#[tokio::test]
async fn test_play_full_pipeline_benchmark() {
    let tmp_dir = std::env::temp_dir().join("onionforge_play_bench");
    let _ = tokio::fs::remove_dir_all(&tmp_dir).await;

    let overall_start = Instant::now();

    // Phase 1: Fingerprint matching
    let fp_start = Instant::now();
    let registry = AdapterRegistry::new();
    let fp = SiteFingerprint {
        url: "http://b3pzp6qwelgeygmzn6awkduym6s4gxh6htwxuxeydrziwzlx63zergyd.onion/FALOp"
            .to_string(),
        status: 200,
        headers: HeaderMap::new(),
        body: "Index of /FALOp/".to_string(),
    };
    let adapter: Option<&dyn crawli_lib::adapters::CrawlerAdapter> =
        registry.determine_adapter(&fp).await;
    assert!(adapter.is_some());
    let fp_elapsed = fp_start.elapsed();

    // Phase 2: Simulate crawl
    let crawl_start = Instant::now();
    let files = simulate_play_crawl(true, true);
    let crawl_elapsed = crawl_start.elapsed();

    // Phase 3: Download scaffold
    let dl_start = Instant::now();
    let count = test_scaffold_download(&files, tmp_dir.to_str().unwrap())
        .await
        .unwrap();
    let dl_elapsed = dl_start.elapsed();

    let overall_elapsed = overall_start.elapsed();

    println!("\n╔══════════════════════════════════════════════╗");
    println!("║  PLAY RANSOMWARE - FULL PIPELINE BENCHMARK   ║");
    println!("╠══════════════════════════════════════════════╣");
    println!("║  Phase 1: Fingerprint match  {:>12?}   ║", fp_elapsed);
    println!("║  Phase 2: Crawl simulation   {:>12?}   ║", crawl_elapsed);
    println!("║  Phase 3: Download scaffold  {:>12?}   ║", dl_elapsed);
    println!("╠══════════════════════════════════════════════╣");
    println!(
        "║  Total pipeline              {:>12?}   ║",
        overall_elapsed
    );
    println!("║  Files discovered            {:>12}   ║", files.len());
    println!("║  Files written to disk       {:>12}   ║", count);
    println!(
        "║  Total indexed size         {:>10.2} GB   ║",
        files.iter().filter_map(|f| f.size_bytes).sum::<u64>() as f64 / 1_073_741_824.0
    );
    println!("╚══════════════════════════════════════════════╝\n");

    let _ = tokio::fs::remove_dir_all(&tmp_dir).await;
}
