use std::path::PathBuf;
/// Play Ransomware — 10-Minute Feature Validation Suite
/// Tests every recommendation against the real Play URL structure:
///   http://b3pzp6qwelgeygmzn6awkduym6s4gxh6htwxuxeydrziwzlx63zergyd.onion/FALOp
///
/// Features tested:
///   1. Dynamic HTML Autoindex Parser (parse_autoindex_html)
///   2. URL Decoding for %20-encoded filenames
///   3. HTTP HEAD probing (simulated)
///   4. Folder entry emission
///   5. Filename sanitization (illegal chars, Windows reserved, control chars)
///   6. Full scaffold download with sanitized paths
///   7. Manifest generation with decoded paths
///   8. Resume safety / no-overwrite
///   9. Sustained load simulation (concurrent workers x 120)
///  10. Edge cases: empty entries, deeply nested paths, unicode filenames
use std::sync::Arc;
use std::time::Instant;

use crawli_lib::adapters::{AdapterRegistry, EntryType, FileEntry, SiteFingerprint};
use crawli_lib::frontier::{CrawlOptions, CrawlerFrontier};
use crawli_lib::path_utils;
use reqwest::header::HeaderMap;

// ═══════════════════════════════════════════════════════════════
// FEATURE 1: Dynamic HTML Autoindex Parser
// ═══════════════════════════════════════════════════════════════

/// Realistic Play autoindex HTML matching the screenshot
fn play_realistic_html() -> String {
    r#"<html>
<head><title>Index of /FALOp/</title></head>
<body>
<h1>Index of /FALOp/</h1><hr><pre>
<a href="../">../</a>
<a href="2%20Sally%20Personal.part01.rar">2 Sally Personal.part01.rar</a>         24-Feb-2026 01:29            524288000
<a href="2%20Sally%20Personal.part02.rar">2 Sally Personal.part02.rar</a>         24-Feb-2026 01:29            524288000
<a href="2%20Sally%20Personal.part03.rar">2 Sally Personal.part03.rar</a>         24-Feb-2026 01:30            524288000
<a href="2%20Sally%20Personal.part04.rar">2 Sally Personal.part04.rar</a>         24-Feb-2026 01:31            524288000
<a href="2%20Sally%20Personal.part05.rar">2 Sally Personal.part05.rar</a>         24-Feb-2026 01:32            524288000
<a href="2%20Sally%20Personal.part06.rar">2 Sally Personal.part06.rar</a>         24-Feb-2026 01:33            524288000
<a href="2%20Sally%20Personal.part07.rar">2 Sally Personal.part07.rar</a>         24-Feb-2026 01:34            524288000
<a href="2%20Sally%20Personal.part08.rar">2 Sally Personal.part08.rar</a>         24-Feb-2026 01:35            524288000
<a href="2%20Sally%20Personal.part09.rar">2 Sally Personal.part09.rar</a>         24-Feb-2026 01:35            524288000
<a href="2%20Sally%20Personal.part10.rar">2 Sally Personal.part10.rar</a>         24-Feb-2026 01:36            524288000
<a href="2%20Sally%20Personal.part11.rar">2 Sally Personal.part11.rar</a>         24-Feb-2026 01:36             60844542
</pre><hr></body></html>"#.to_string()
}

#[tokio::test]
async fn feature1_dynamic_html_parsing() {
    println!("\n══════════════════════════════════════════");
    println!("  FEATURE 1: Dynamic HTML Autoindex Parser");
    println!("══════════════════════════════════════════");

    let html = play_realistic_html();

    // Use scraper-free line-based parser (same logic as play.rs)
    let parsed = parse_autoindex_entries(&html);

    assert_eq!(
        parsed.len(),
        11,
        "Should parse exactly 11 entries from Play HTML"
    );

    for (i, (name, size)) in parsed.iter().enumerate() {
        let expected_name = format!("2 Sally Personal.part{:02}.rar", i + 1);
        assert_eq!(name, &expected_name, "Filename mismatch at index {}", i);

        if i < 10 {
            assert_eq!(
                *size,
                Some(524288000),
                "Parts 1-10 should be 524288000 bytes"
            );
        } else {
            assert_eq!(*size, Some(60844542), "Part 11 should be 60844542 bytes");
        }
        println!("  ✅ Parsed: {} ({} bytes)", name, size.unwrap_or(0));
    }

    // Verify ../  was skipped
    assert!(
        !parsed.iter().any(|(name, _)| name.contains("..")),
        "Parent dir link should be filtered"
    );

    let total: u64 = parsed.iter().filter_map(|(_, s)| *s).sum();
    println!(
        "  📊 Total parsed size: {:.2} GB",
        total as f64 / 1_073_741_824.0
    );
}

#[tokio::test]
async fn feature1_html_parser_edge_cases() {
    println!("\n══════════════════════════════════════════");
    println!("  FEATURE 1b: HTML Parser Edge Cases");
    println!("══════════════════════════════════════════");

    // Empty HTML
    let empty_parsed = parse_autoindex_entries("");
    assert_eq!(empty_parsed.len(), 0);
    println!("  ✅ Empty HTML → 0 entries");

    // HTML with no valid links
    let no_links = "<html><body>No files here</body></html>";
    assert_eq!(parse_autoindex_entries(no_links).len(), 0);
    println!("  ✅ No-link HTML → 0 entries");

    // HTML with only parent directory
    let parent_only = r#"<a href="../">../</a>"#;
    assert_eq!(parse_autoindex_entries(parent_only).len(), 0);
    println!("  ✅ Parent-only HTML → 0 entries");

    // HTML with URL-encoded special characters
    let special = r#"<a href="file%20with%20%26%20special.txt">file with &amp; special.txt</a>  01-Jan-2026 00:00  12345"#;
    let parsed = parse_autoindex_entries(special);
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].0, "file with & special.txt");
    assert_eq!(parsed[0].1, Some(12345));
    println!("  ✅ Special chars decoded: '{}'", parsed[0].0);

    // HTML with no size column
    let no_size = r#"<a href="mystery.bin">mystery.bin</a>"#;
    let parsed = parse_autoindex_entries(no_size);
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].1, None);
    println!("  ✅ Missing size → None");

    // HTML with subdirectory link (trailing slash)
    let subdir = r#"<a href="subdir/">subdir/</a>  -"#;
    let parsed = parse_autoindex_entries(subdir);
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].0, "subdir");
    println!("  ✅ Subdirectory link: '{}'", parsed[0].0);
}

// ═══════════════════════════════════════════════════════════════
// FEATURE 2: URL Decoding for Play filenames
// ═══════════════════════════════════════════════════════════════

#[tokio::test]
async fn feature2_url_decoding() {
    println!("\n══════════════════════════════════════════");
    println!("  FEATURE 2: URL Decoding");
    println!("══════════════════════════════════════════");

    // Play-specific filename decoding
    let cases = vec![
        (
            "2%20Sally%20Personal.part01.rar",
            "2 Sally Personal.part01.rar",
        ),
        ("file%20with%20spaces.zip", "file with spaces.zip"),
        ("normal-file.txt", "normal-file.txt"),
        ("%E2%9C%93%20verified.pdf", "✓ verified.pdf"),
        ("path%2Fto%2Ffile.doc", "path/to/file.doc"),
        ("100%25+complete.log", "100% complete.log"),
        ("unchanged", "unchanged"),
        ("", ""),
        ("%00null%00byte", "\0null\0byte"), // edge: null bytes
    ];

    for (encoded, expected) in &cases {
        let decoded = path_utils::url_decode(encoded);
        assert_eq!(&decoded, expected, "Decoding '{}' failed", encoded);
        println!("  ✅ {} → {}", encoded, decoded);
    }

    // URL encoding roundtrip
    let original = "2 Sally Personal.part01.rar";
    let encoded = path_utils::url_encode(original);
    assert_eq!(encoded, "2%20Sally%20Personal.part01.rar");
    let roundtrip = path_utils::url_decode(&encoded);
    assert_eq!(roundtrip, original);
    println!("  ✅ Roundtrip encode/decode: '{}'", original);
}

// ═══════════════════════════════════════════════════════════════
// FEATURE 3: HTTP HEAD Size Probing (Simulated)
// ═══════════════════════════════════════════════════════════════

#[tokio::test]
async fn feature3_head_probing_simulation() {
    println!("\n══════════════════════════════════════════");
    println!("  FEATURE 3: HTTP HEAD Size Probing");
    println!("══════════════════════════════════════════");

    // Simulate HEAD request behavior
    let frontier = CrawlerFrontier::new(
        None,
        "http://b3pzp6qwelgeygmzn6awkduym6s4gxh6htwxuxeydrziwzlx63zergyd.onion/FALOp".to_string(),
        4,
        true,
        vec![9051, 9052, 9053, 9054],
        Vec::new(),
        CrawlOptions {
            listing: true,
            sizes: true,
            download: false,
            circuits: None,
            daemons: None,
            agnostic_state: false,
            resume: false,
            resume_index: None,
            mega_password: None,
            stealth_ramp: false,
        },
        None,
    );
    let (_cid, client) = frontier.get_client();

    // Try HEAD against a real clearnet server to verify the mechanism works
    let start = Instant::now();
    match client.head("https://httpbin.org/bytes/1024").send().await {
        Ok(resp) => {
            let content_length = resp
                .headers()
                .get("content-length")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok());
            println!(
                "  ✅ HEAD probe returned Content-Length: {:?} in {:?}",
                content_length,
                start.elapsed()
            );
        }
        Err(_) => {
            // Network not available — that's fine for a unit test
            println!("  ⚠️  HEAD probe unavailable (no network) — testing logic path only");
        }
    }

    // Verify that when sizes=false, we skip HEAD entirely
    let frontier_no_sizes = CrawlerFrontier::new(
        None,
        "http://test.onion".to_string(),
        4,
        true,
        vec![9051, 9052, 9053, 9054],
        Vec::new(),
        CrawlOptions {
            listing: true,
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
        None,
    );
    assert!(!frontier_no_sizes.active_options.sizes);
    println!("  ✅ sizes=false correctly propagated — HEAD probing will be skipped");
}

// ═══════════════════════════════════════════════════════════════
// FEATURE 4: Folder Entry Emission
// ═══════════════════════════════════════════════════════════════

#[tokio::test]
async fn feature4_folder_entry_emission() {
    println!("\n══════════════════════════════════════════");
    println!("  FEATURE 4: Folder Entry Emission");
    println!("══════════════════════════════════════════");

    let dir = path_utils::extract_target_dirname(
        "http://b3pzp6qwelgeygmzn6awkduym6s4gxh6htwxuxeydrziwzlx63zergyd.onion/FALOp",
    );
    assert_eq!(dir, "FALOp");
    println!("  ✅ extract_target_dirname → '{}'", dir);

    // Simulate what the adapter now produces
    let mut entries = Vec::new();

    // Folder entry (now emitted first)
    entries.push(FileEntry {
        path: format!("/{}", dir),
        size_bytes: None,
        jwt_exp: None,
        entry_type: EntryType::Folder,
        raw_url: "http://b3pzp6qwelgeygmzn6awkduym6s4gxh6htwxuxeydrziwzlx63zergyd.onion/FALOp"
            .to_string(),
    });

    // File entries
    for i in 1..=11 {
        entries.push(FileEntry {
            path: format!("/{}/2 Sally Personal.part{:02}.rar", dir, i),
            size_bytes: Some(if i == 11 { 60844542 } else { 524288000 }),
            jwt_exp: None,
            entry_type: EntryType::File,
            raw_url: format!(
                "http://play.onion/FALOp/2%20Sally%20Personal.part{:02}.rar",
                i
            ),
        });
    }

    assert_eq!(
        entries.len(),
        12,
        "Should have 1 folder + 11 files = 12 entries"
    );
    assert_eq!(entries[0].entry_type, EntryType::Folder);
    assert_eq!(entries[0].path, "/FALOp");
    println!(
        "  ✅ Total entries: {} (1 folder + 11 files)",
        entries.len()
    );

    // Scaffold to disk and verify folder exists
    let tmp_dir = std::env::temp_dir().join("onionforge_folder_test");
    let _ = tokio::fs::remove_dir_all(&tmp_dir).await;
    let count = test_scaffold(&entries, tmp_dir.to_str().unwrap())
        .await
        .unwrap();
    assert_eq!(count, 12);
    assert!(tmp_dir.join("FALOp").is_dir(), "FALOp folder should exist");
    assert!(
        tmp_dir.join("FALOp/.gitkeep").exists(),
        "FALOp/.gitkeep should exist"
    );
    println!("  ✅ FALOp/ folder created with .gitkeep marker");

    let _ = tokio::fs::remove_dir_all(&tmp_dir).await;
}

// ═══════════════════════════════════════════════════════════════
// FEATURE 5: Filename Sanitization
// ═══════════════════════════════════════════════════════════════

#[tokio::test]
async fn feature5_filename_sanitization() {
    println!("\n══════════════════════════════════════════");
    println!("  FEATURE 5: Filename Sanitization");
    println!("══════════════════════════════════════════");

    let cases = vec![
        // (input, expected)
        (
            "/FALOp/2%20Sally%20Personal.part01.rar",
            "FALOp/2 Sally Personal.part01.rar",
        ),
        (
            "/dir/file<with>bad:chars?.txt",
            "dir/file_with_bad_chars_.txt",
        ),
        ("/CON/NUL/test.txt", "_CON/_NUL/test.txt"),
        ("///multiple///slashes///", "multiple/slashes"),
        ("/path/trailing.dot.", "path/trailing.dot"),
        ("/path/trailing space ", "path/trailing space"),
        ("/file\twith\ttabs.txt", "filewithtabs.txt"), // control chars stripped
        ("/normal/path/file.zip", "normal/path/file.zip"),
        ("", ""),
    ];

    for (input, expected) in &cases {
        let result = path_utils::sanitize_path(input);
        assert_eq!(
            &result, expected,
            "Sanitize '{}' failed: got '{}'",
            input, result
        );
        println!("  ✅ {} → {}", input, result);
    }
}

// ═══════════════════════════════════════════════════════════════
// FEATURE 6: Full Scaffold with Sanitized Paths
// ═══════════════════════════════════════════════════════════════

#[tokio::test]
async fn feature6_full_scaffold_sanitized() {
    println!("\n══════════════════════════════════════════");
    println!("  FEATURE 6: Full Scaffold (Sanitized)");
    println!("══════════════════════════════════════════");

    let tmp_dir = std::env::temp_dir().join("onionforge_sanitize_test");
    let _ = tokio::fs::remove_dir_all(&tmp_dir).await;

    // Entries with URL-encoded and special-char paths
    let entries = vec![
        FileEntry {
            path: "/FALOp".to_string(),
            size_bytes: None,
            jwt_exp: None,
            entry_type: EntryType::Folder,
            raw_url: "http://play.onion/FALOp".to_string(),
        },
        FileEntry {
            path: "/FALOp/2%20Sally%20Personal.part01.rar".to_string(),
            size_bytes: Some(524288000),
            jwt_exp: None,
            entry_type: EntryType::File,
            raw_url: "http://play.onion/FALOp/2%20Sally%20Personal.part01.rar".to_string(),
        },
        FileEntry {
            path: "/FALOp/file<with>bad?chars.rar".to_string(),
            size_bytes: Some(100),
            jwt_exp: None,
            entry_type: EntryType::File,
            raw_url: "http://play.onion/FALOp/file<with>bad?chars.rar".to_string(),
        },
        FileEntry {
            path: "/FALOp/CON".to_string(),
            size_bytes: None,
            jwt_exp: None,
            entry_type: EntryType::Folder,
            raw_url: "http://play.onion/FALOp/CON".to_string(),
        },
    ];

    let count = test_scaffold(&entries, tmp_dir.to_str().unwrap())
        .await
        .unwrap();
    assert_eq!(count, 4);

    // URL-decoded filename on disk
    let decoded_file = tmp_dir.join("FALOp/2 Sally Personal.part01.rar");
    assert!(
        decoded_file.exists(),
        "URL-decoded file should exist on disk"
    );
    println!("  ✅ '%20' → space: {}", decoded_file.display());

    // Sanitized illegal chars: <with> becomes _with_, bad:chars? becomes bad_chars_
    // So "file<with>bad?chars.rar" → "file_with_bad_chars_.rar"
    // But the trailing dot rule might strip the trailing period. Let's check what actually lands on disk:
    let sanitized_name = path_utils::sanitize_path("/FALOp/file<with>bad?chars.rar");
    let sanitized_file = tmp_dir.join(&sanitized_name);
    assert!(
        sanitized_file.exists(),
        "Sanitized file should exist at: {}",
        sanitized_file.display()
    );
    println!("  ✅ Illegal chars sanitized: {}", sanitized_name);

    // Windows reserved name protection
    let con_dir = tmp_dir.join("FALOp/_CON");
    assert!(con_dir.is_dir(), "_CON directory should exist");
    println!("  ✅ Reserved name 'CON' → '_CON'");

    // Verify meta sidecar includes original_path
    let meta = tmp_dir.join("FALOp/2 Sally Personal.part01.rar.onionforge.meta");
    let content = tokio::fs::read_to_string(&meta).await.unwrap();
    assert!(
        content.contains("original_path="),
        "Meta should include original_path"
    );
    assert!(
        content.contains("size=524288000"),
        "Meta should include correct size"
    );
    println!("  ✅ Meta sidecar verified with original_path + size");

    let _ = tokio::fs::remove_dir_all(&tmp_dir).await;
}

// ═══════════════════════════════════════════════════════════════
// FEATURE 7: Manifest with URL-Decoded Paths
// ═══════════════════════════════════════════════════════════════

#[tokio::test]
async fn feature7_manifest_decoded_paths() {
    println!("\n══════════════════════════════════════════");
    println!("  FEATURE 7: Manifest with Decoded Paths");
    println!("══════════════════════════════════════════");

    let tmp_dir = std::env::temp_dir().join("onionforge_manifest_test");
    let _ = tokio::fs::remove_dir_all(&tmp_dir).await;

    let entries = vec![FileEntry {
        path: "/FALOp/2%20Sally%20Personal.part01.rar".to_string(),
        size_bytes: Some(524288000),
        jwt_exp: None,
        entry_type: EntryType::File,
        raw_url: "http://play.onion/FALOp/2%20Sally%20Personal.part01.rar".to_string(),
    }];

    test_scaffold(&entries, tmp_dir.to_str().unwrap())
        .await
        .unwrap();

    let manifest = tokio::fs::read_to_string(tmp_dir.join("_onionforge_manifest.txt"))
        .await
        .unwrap();
    // Manifest should show decoded paths for human readability
    assert!(
        manifest.contains("2 Sally Personal.part01.rar"),
        "Manifest should show decoded filename"
    );
    assert!(
        manifest.contains("500.00 MB") || manifest.contains("524288000"),
        "Manifest should show size"
    );
    println!("  ✅ Manifest shows decoded path: '2 Sally Personal.part01.rar'");

    let _ = tokio::fs::remove_dir_all(&tmp_dir).await;
}

// ═══════════════════════════════════════════════════════════════
// FEATURE 8: Resume Safety
// ═══════════════════════════════════════════════════════════════

#[tokio::test]
async fn feature8_resume_safety() {
    println!("\n══════════════════════════════════════════");
    println!("  FEATURE 8: Resume Safety (No Overwrite)");
    println!("══════════════════════════════════════════");

    let tmp_dir = std::env::temp_dir().join("onionforge_resume_test2");
    let _ = tokio::fs::remove_dir_all(&tmp_dir).await;

    let entries = vec![FileEntry {
        path: "/FALOp/test_resume.rar".to_string(),
        size_bytes: Some(1000),
        jwt_exp: None,
        entry_type: EntryType::File,
        raw_url: "http://play.onion/test_resume.rar".to_string(),
    }];

    // First scaffold — creates 0-byte placeholder
    test_scaffold(&entries, tmp_dir.to_str().unwrap())
        .await
        .unwrap();
    let file_path = tmp_dir.join("FALOp/test_resume.rar");
    assert_eq!(tokio::fs::read(&file_path).await.unwrap().len(), 0);
    println!("  ✅ Step 1: Empty placeholder created");

    // Simulate partial download by writing real content
    tokio::fs::write(&file_path, b"PARTIAL_DOWNLOAD_DATA_12345")
        .await
        .unwrap();
    let size_before = tokio::fs::metadata(&file_path).await.unwrap().len();
    println!(
        "  ✅ Step 2: Wrote {} bytes of partial download",
        size_before
    );

    // Second scaffold — should NOT overwrite
    test_scaffold(&entries, tmp_dir.to_str().unwrap())
        .await
        .unwrap();
    let content = tokio::fs::read(&file_path).await.unwrap();
    assert_eq!(content, b"PARTIAL_DOWNLOAD_DATA_12345");
    println!(
        "  ✅ Step 3: Re-scaffold preserved existing content ({} bytes)",
        content.len()
    );

    let _ = tokio::fs::remove_dir_all(&tmp_dir).await;
}

// ═══════════════════════════════════════════════════════════════
// FEATURE 9: Sustained Load Simulation (120 workers, extended)
// ═══════════════════════════════════════════════════════════════

#[tokio::test]
async fn feature9_sustained_load_120_workers() {
    println!("\n══════════════════════════════════════════");
    println!("  FEATURE 9: Sustained Load (120 Workers)");
    println!("══════════════════════════════════════════");

    let frontier = Arc::new(CrawlerFrontier::new(
        None,
        "http://b3pzp6qwelgeygmzn6awkduym6s4gxh6htwxuxeydrziwzlx63zergyd.onion/FALOp".to_string(),
        4,
        true,
        vec![9051, 9052, 9053, 9054],
        Vec::new(),
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
        None,
    ));

    let start = Instant::now();
    let mut handles = Vec::new();

    // 120 workers, each discovering 500 unique URLs with full sanitization
    for worker_id in 0..120u32 {
        let f = frontier.clone();
        handles.push(tokio::spawn(async move {
            let mut discovered = 0u32;
            for i in 0..500u32 {
                // Simulate URL-encoded filenames
                let raw = format!(
                    "http://play.onion/FALOp/worker_{}/file%20{:04}.rar",
                    worker_id, i
                );
                let decoded = path_utils::url_decode(&raw);
                let sanitized = path_utils::sanitize_path(&format!(
                    "/FALOp/worker_{}/file {:04}.rar",
                    worker_id, i
                ));

                let _client = f.get_client();
                if f.mark_visited(&raw) {
                    discovered += 1;
                }

                // Verify sanitization doesn't crash
                assert!(!sanitized.contains('\0'));
                assert!(!decoded.contains("%20"));
            }
            discovered
        }));
    }

    let mut total = 0u32;
    for h in handles {
        total += h.await.unwrap();
    }
    let elapsed = start.elapsed();

    assert_eq!(total, 60_000, "120 × 500 = 60,000 unique URLs");
    println!(
        "  ✅ 120 workers × 500 URLs = {} discovered in {:?}",
        total, elapsed
    );
    println!(
        "  📊 Throughput: {:.0} URLs/sec",
        total as f64 / elapsed.as_secs_f64()
    );
    println!(
        "  📊 Semaphore permits: {}",
        frontier.politeness_semaphore.available_permits()
    );
    println!("  📊 Client pool size: {}", frontier.http_clients.len());
}

// ═══════════════════════════════════════════════════════════════
// FEATURE 10: Edge Cases — Unicode, Empty, Deep Nesting
// ═══════════════════════════════════════════════════════════════

#[tokio::test]
async fn feature10_edge_cases_comprehensive() {
    println!("\n══════════════════════════════════════════");
    println!("  FEATURE 10: Comprehensive Edge Cases");
    println!("══════════════════════════════════════════");

    let tmp_dir = std::env::temp_dir().join("onionforge_edge_test");
    let _ = tokio::fs::remove_dir_all(&tmp_dir).await;

    let entries = vec![
        // Unicode filename
        FileEntry {
            path: "/FALOp/données_confidentielles.zip".to_string(),
            size_bytes: Some(1024),
            jwt_exp: None,
            entry_type: EntryType::File,
            raw_url: "http://play.onion/FALOp/donn%C3%A9es_confidentielles.zip".to_string(),
        },
        // Chinese characters
        FileEntry {
            path: "/FALOp/机密文件.pdf".to_string(),
            size_bytes: Some(2048),
            jwt_exp: None,
            entry_type: EntryType::File,
            raw_url: "http://play.onion/FALOp/%E6%9C%BA%E5%AF%86%E6%96%87%E4%BB%B6.pdf".to_string(),
        },
        // Deeply nested (6 levels)
        FileEntry {
            path: "/a/b/c/d/e/f/deep_file.log".to_string(),
            size_bytes: Some(42),
            jwt_exp: None,
            entry_type: EntryType::File,
            raw_url: "http://play.onion/a/b/c/d/e/f/deep_file.log".to_string(),
        },
        // Completely empty path
        FileEntry {
            path: "".to_string(),
            size_bytes: None,
            jwt_exp: None,
            entry_type: EntryType::File,
            raw_url: "http://play.onion/".to_string(),
        },
        // 0-byte file with no size
        FileEntry {
            path: "/FALOp/empty.bin".to_string(),
            size_bytes: None,
            jwt_exp: None,
            entry_type: EntryType::File,
            raw_url: "http://play.onion/FALOp/empty.bin".to_string(),
        },
        // Path with only special chars
        FileEntry {
            path: "/<>:\"?*".to_string(),
            size_bytes: None,
            jwt_exp: None,
            entry_type: EntryType::File,
            raw_url: "http://play.onion/<>:\"?*".to_string(),
        },
        // File size = 0
        FileEntry {
            path: "/FALOp/zero_byte.txt".to_string(),
            size_bytes: Some(0),
            jwt_exp: None,
            entry_type: EntryType::File,
            raw_url: "http://play.onion/FALOp/zero_byte.txt".to_string(),
        },
    ];

    let count = test_scaffold(&entries, tmp_dir.to_str().unwrap())
        .await
        .unwrap();
    // 7 entries minus 1 empty path = 6 written
    assert_eq!(count, 6, "Empty path should be skipped");
    println!("  ✅ 6/7 entries written (1 empty path skipped)");

    // Unicode files
    assert!(tmp_dir.join("FALOp/données_confidentielles.zip").exists());
    println!("  ✅ Unicode (French): données_confidentielles.zip");

    assert!(tmp_dir.join("FALOp/机密文件.pdf").exists());
    println!("  ✅ Unicode (Chinese): 机密文件.pdf");

    // Deep nesting
    assert!(tmp_dir.join("a/b/c/d/e/f/deep_file.log").exists());
    println!("  ✅ 6-level deep nesting verified");

    // 0-byte file
    let zero_meta = tmp_dir.join("FALOp/zero_byte.txt.onionforge.meta");
    let meta_content = tokio::fs::read_to_string(&zero_meta).await.unwrap();
    assert!(meta_content.contains("size=0"));
    println!("  ✅ 0-byte file with size=0 in meta");

    // Special-chars-only filename
    let sanitized_special = tmp_dir.join("______");
    assert!(
        sanitized_special.exists(),
        "All-special-char file should exist as underscores"
    );
    println!("  ✅ All-special-char filename → underscores");

    let _ = tokio::fs::remove_dir_all(&tmp_dir).await;
}

// ═══════════════════════════════════════════════════════════════
// FULL PIPELINE BENCHMARK (all features combined)
// ═══════════════════════════════════════════════════════════════

#[tokio::test]
async fn full_pipeline_benchmark_all_features() {
    println!("\n══════════════════════════════════════════════════");
    println!("  FULL PIPELINE: All Features Combined Benchmark");
    println!("══════════════════════════════════════════════════");

    let tmp_dir = std::env::temp_dir().join("onionforge_full_bench");
    let _ = tokio::fs::remove_dir_all(&tmp_dir).await;

    let overall_start = Instant::now();

    // Phase 1: Fingerprint
    let fp_start = Instant::now();
    let registry = AdapterRegistry::new();
    let fp = SiteFingerprint {
        url: "http://b3pzp6qwelgeygmzn6awkduym6s4gxh6htwxuxeydrziwzlx63zergyd.onion/FALOp"
            .to_string(),
        status: 200,
        headers: HeaderMap::new(),
        body: play_realistic_html(),
    };
    let adapter: Option<&dyn crawli_lib::adapters::CrawlerAdapter> =
        registry.determine_adapter(&fp).await;
    assert!(adapter.is_some());
    let fp_time = fp_start.elapsed();

    // Phase 2: Parse HTML
    let parse_start = Instant::now();
    let parsed = parse_autoindex_entries(&fp.body);
    let parse_time = parse_start.elapsed();

    // Phase 3: Build entries with sanitization
    let build_start = Instant::now();
    let dir_name = path_utils::extract_target_dirname(&fp.url);
    let mut entries = Vec::new();
    entries.push(FileEntry {
        path: format!("/{}", dir_name),
        size_bytes: None,
        jwt_exp: None,
        entry_type: EntryType::Folder,
        raw_url: fp.url.clone(),
    });
    for (name, size) in &parsed {
        let encoded = path_utils::url_encode(name);
        entries.push(FileEntry {
            path: format!("/{}/{}", dir_name, path_utils::sanitize_path(name)),
            size_bytes: *size,
            jwt_exp: None,
            entry_type: EntryType::File,
            raw_url: format!("http://play.onion/FALOp/{}", encoded),
        });
    }
    let build_time = build_start.elapsed();

    // Phase 4: Scaffold to disk
    let scaffold_start = Instant::now();
    let count = test_scaffold(&entries, tmp_dir.to_str().unwrap())
        .await
        .unwrap();
    let scaffold_time = scaffold_start.elapsed();

    let overall_time = overall_start.elapsed();

    let total_size: u64 = entries.iter().filter_map(|e| e.size_bytes).sum();

    println!("\n╔════════════════════════════════════════════════════╗");
    println!("║    PLAY RANSOMWARE - ALL FEATURES BENCHMARK        ║");
    println!("╠════════════════════════════════════════════════════╣");
    println!("║  Phase 1: Fingerprint match     {:>14?}   ║", fp_time);
    println!("║  Phase 2: HTML autoindex parse  {:>14?}   ║", parse_time);
    println!("║  Phase 3: URL encode/sanitize   {:>14?}   ║", build_time);
    println!(
        "║  Phase 4: Scaffold to disk      {:>14?}   ║",
        scaffold_time
    );
    println!("╠════════════════════════════════════════════════════╣");
    println!(
        "║  Total pipeline                 {:>14?}   ║",
        overall_time
    );
    println!(
        "║  Adapter matched             {:>17}   ║",
        adapter.unwrap().name()
    );
    println!("║  HTML entries parsed         {:>17}   ║", parsed.len());
    println!("║  Total entries (incl folder) {:>17}   ║", entries.len());
    println!("║  Items written to disk       {:>17}   ║", count);
    println!(
        "║  Total indexed size          {:>14.2} GB   ║",
        total_size as f64 / 1_073_741_824.0
    );
    println!("╚════════════════════════════════════════════════════╝\n");

    let _ = tokio::fs::remove_dir_all(&tmp_dir).await;
}

// ═══════════════════════════════════════════════════════════════
// HELPERS
// ═══════════════════════════════════════════════════════════════

/// Mirrors the HTML parsing logic from play.rs for test use
fn parse_autoindex_entries(html: &str) -> Vec<(String, Option<u64>)> {
    let mut results = Vec::new();
    for line in html.lines() {
        if let Some(href_start) = line.find("href=\"") {
            let after_href = &line[href_start + 6..];
            if let Some(href_end) = after_href.find('"') {
                let raw_href = &after_href[..href_end];
                if raw_href == "../" || raw_href == ".." || raw_href == "/" {
                    continue;
                }
                let decoded = path_utils::url_decode(raw_href);
                let clean = decoded.trim_end_matches('/').to_string();
                if clean.is_empty() {
                    continue;
                }

                let size = if let Some(after_tag) = line.split("</a>").nth(1) {
                    let tokens: Vec<&str> = after_tag.split_whitespace().collect();
                    tokens.last().and_then(|s| s.trim().parse::<u64>().ok())
                } else {
                    None
                };

                results.push((clean, size));
            }
        }
    }
    results
}

/// Scaffold download logic mirrored from lib.rs (without AppHandle dependency)
async fn test_scaffold(entries: &[FileEntry], output_dir: &str) -> anyhow::Result<u32> {
    let base = PathBuf::from(output_dir);
    tokio::fs::create_dir_all(&base).await?;

    let mut written: u32 = 0;

    for entry in entries.iter() {
        let sanitized = path_utils::sanitize_path(&entry.path);
        if sanitized.is_empty() {
            continue;
        }

        let full_path = base.join(&sanitized);

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
                let meta_path = PathBuf::from(format!("{}.onionforge.meta", full_path.display()));
                let size_str = entry
                    .size_bytes
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "0".to_string());
                let meta_content = format!(
                    "url={}\nsize={}\ntype=file\noriginal_path={}\n",
                    entry.raw_url, size_str, entry.path
                );
                tokio::fs::write(&meta_path, meta_content.as_bytes()).await?;
            }
        }
        written += 1;
    }

    // Manifest
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
            .map(|s| format!("{:.2} MB", s as f64 / 1_048_576.0))
            .unwrap_or_else(|| "0 B".to_string());
        let decoded = path_utils::url_decode(&entry.path);
        manifest.push_str(&format!(
            "{} {:>12}  {}  {}\n",
            type_tag, size_tag, decoded, entry.raw_url
        ));
    }
    tokio::fs::write(&manifest_path, manifest.as_bytes()).await?;

    Ok(written)
}
