/// Mega + Torrent Integration Test
/// Phase 52C: Validates auto-detection, URL parsing, mode switching,
/// and FileEntry production without requiring live network or Tauri AppHandle.
use crawli_lib::mega_handler;
use crawli_lib::torrent_handler;

// ── Mega.nz Detection Tests ────────────────────────────────────────

#[test]
fn test_mega_detection_new_folder() {
    assert!(mega_handler::is_mega_link(
        "https://mega.nz/folder/ABC123#key456"
    ));
    assert!(mega_handler::is_mega_link(
        "https://mega.nz/file/XYZ789#key123"
    ));
}

#[test]
fn test_mega_detection_legacy() {
    assert!(mega_handler::is_mega_link("http://mega.nz/#F!HANDLE!KEY"));
    assert!(mega_handler::is_mega_link("http://mega.co.nz/#!FILE!KEY"));
}

#[test]
fn test_mega_detection_rejects_nonmega() {
    assert!(!mega_handler::is_mega_link("http://example.onion/files/"));
    assert!(!mega_handler::is_mega_link("magnet:?xt=urn:btih:abc"));
    assert!(!mega_handler::is_mega_link("https://google.com"));
    assert!(!mega_handler::is_mega_link(
        "https://megaupload.nz/folder/test"
    ));
}

// ── Mega.nz URL Parsing Tests ──────────────────────────────────────

#[test]
fn test_mega_parse_new_folder_format() {
    let info =
        mega_handler::parse_mega_url("https://mega.nz/folder/TestHandle#TestKey123").unwrap();
    assert_eq!(info.link_type, mega_handler::MegaLinkType::Folder);
    assert_eq!(info.handle, "TestHandle");
    assert_eq!(info.key, "TestKey123");
}

#[test]
fn test_mega_parse_new_file_format() {
    let info = mega_handler::parse_mega_url("https://mega.nz/file/FileHandle#FileKey").unwrap();
    assert_eq!(info.link_type, mega_handler::MegaLinkType::File);
    assert_eq!(info.handle, "FileHandle");
    assert_eq!(info.key, "FileKey");
}

#[test]
fn test_mega_parse_legacy_folder_format() {
    let info = mega_handler::parse_mega_url("https://mega.nz/#F!LegacyFolder!LegacyKey").unwrap();
    assert_eq!(info.link_type, mega_handler::MegaLinkType::Folder);
    assert_eq!(info.handle, "LegacyFolder");
    assert_eq!(info.key, "LegacyKey");
}

#[test]
fn test_mega_parse_legacy_file_format() {
    let info = mega_handler::parse_mega_url("https://mega.nz/#!LegacyFile!LegacyKey").unwrap();
    assert_eq!(info.link_type, mega_handler::MegaLinkType::File);
    assert_eq!(info.handle, "LegacyFile");
    assert_eq!(info.key, "LegacyKey");
}

#[test]
fn test_mega_parse_co_nz_domain() {
    let info = mega_handler::parse_mega_url("https://mega.co.nz/folder/CoHandle#CoKey").unwrap();
    assert_eq!(info.link_type, mega_handler::MegaLinkType::Folder);
    assert_eq!(info.handle, "CoHandle");
    assert_eq!(info.key, "CoKey");
}

#[test]
fn test_mega_parse_missing_key_fails() {
    assert!(mega_handler::parse_mega_url("https://mega.nz/folder/NoKey").is_err());
    assert!(mega_handler::parse_mega_url("https://mega.nz/file/").is_err());
}

#[test]
fn test_mega_parse_garbage_fails() {
    assert!(mega_handler::parse_mega_url("https://google.com").is_err());
    assert!(mega_handler::parse_mega_url("not_a_url").is_err());
    assert!(mega_handler::parse_mega_url("").is_err());
}

// ── Torrent Detection Tests ────────────────────────────────────────

#[test]
fn test_torrent_detect_magnet() {
    assert!(torrent_handler::is_magnet_link(
        "magnet:?xt=urn:btih:abc123def456"
    ));
    assert!(torrent_handler::is_magnet_link("MAGNET:?xt=urn:btih:ABC"));
    assert!(torrent_handler::is_magnet_link(
        "  magnet:?xt=urn:btih:test  "
    ));
}

#[test]
fn test_torrent_detect_file() {
    assert!(torrent_handler::is_torrent_file("/path/to/ubuntu.torrent"));
    assert!(torrent_handler::is_torrent_file(
        "~/Downloads/movie.TORRENT"
    ));
    assert!(torrent_handler::is_torrent_file("  /tmp/test.torrent  "));
}

#[test]
fn test_torrent_detect_rejects_http_torrent() {
    // HTTP URLs ending in .torrent should NOT match is_torrent_file
    // because they need http handling, not local file parsing
    assert!(!torrent_handler::is_torrent_file(
        "http://example.com/file.torrent"
    ));
    assert!(!torrent_handler::is_torrent_file(
        "https://tracker.org/dl.torrent"
    ));
}

#[test]
fn test_torrent_detect_rejects_non_torrent() {
    assert!(!torrent_handler::is_magnet_link(
        "https://mega.nz/folder/ABC#KEY"
    ));
    assert!(!torrent_handler::is_magnet_link(
        "http://example.onion/files/"
    ));
    assert!(!torrent_handler::is_torrent_file("magnet:?xt=urn:btih:abc"));
    assert!(!torrent_handler::is_torrent_file("/path/to/file.zip"));
}

// ── Combined Mode Detection Tests ──────────────────────────────────

#[test]
fn test_detect_input_mode_mega() {
    assert_eq!(
        torrent_handler::detect_input_mode("https://mega.nz/folder/ABC#KEY"),
        "mega"
    );
    assert_eq!(
        torrent_handler::detect_input_mode("https://mega.co.nz/file/X#Y"),
        "mega"
    );
    assert_eq!(
        torrent_handler::detect_input_mode("http://mega.nz/#F!H!K"),
        "mega"
    );
}

#[test]
fn test_detect_input_mode_torrent() {
    assert_eq!(
        torrent_handler::detect_input_mode("magnet:?xt=urn:btih:abc"),
        "torrent"
    );
    assert_eq!(
        torrent_handler::detect_input_mode("/tmp/test.torrent"),
        "torrent"
    );
}

#[test]
fn test_detect_input_mode_onion_fallback() {
    assert_eq!(
        torrent_handler::detect_input_mode("http://example.onion/files/"),
        "onion"
    );
    assert_eq!(
        torrent_handler::detect_input_mode("https://google.com"),
        "onion"
    );
    assert_eq!(torrent_handler::detect_input_mode(""), "onion");
}

// ── Magnet URI Parsing Tests ───────────────────────────────────────

#[test]
fn test_parse_magnet_full() {
    let info = torrent_handler::parse_magnet(
        "magnet:?xt=urn:btih:da39a3ee5e6b4b0d3255bfef95601890afd80709&dn=TestFile&tr=udp://tracker.example.com:6969"
    ).unwrap();
    assert_eq!(info.info_hash, "da39a3ee5e6b4b0d3255bfef95601890afd80709");
    assert_eq!(info.display_name, Some("TestFile".to_string()));
    assert!(!info.trackers.is_empty());
    assert!(info.trackers[0].contains("tracker.example.com"));
}

#[test]
fn test_parse_magnet_no_name() {
    let info = torrent_handler::parse_magnet(
        "magnet:?xt=urn:btih:da39a3ee5e6b4b0d3255bfef95601890afd80709",
    )
    .unwrap();
    assert_eq!(info.info_hash, "da39a3ee5e6b4b0d3255bfef95601890afd80709");
    // display_name may or may not be Some depending on crate behavior
    assert!(info.trackers.is_empty());
}

#[test]
fn test_parse_magnet_invalid_fails() {
    assert!(torrent_handler::parse_magnet("not_a_magnet").is_err());
    assert!(torrent_handler::parse_magnet("https://google.com").is_err());
}

// ── FileEntry Generation Tests ─────────────────────────────────────

#[test]
fn test_torrent_files_to_entries_structure() {
    use crawli_lib::adapters::EntryType;

    let meta = torrent_handler::TorrentMetadata {
        name: "TestBundle".to_string(),
        info_hash: "abc123def456".to_string(),
        total_size: 2_000_000,
        piece_length: 262_144,
        files: vec![
            torrent_handler::TorrentFileInfo {
                path: "/TestBundle/docs/readme.md".to_string(),
                size: 100_000,
            },
            torrent_handler::TorrentFileInfo {
                path: "/TestBundle/docs/changelog.md".to_string(),
                size: 50_000,
            },
            torrent_handler::TorrentFileInfo {
                path: "/TestBundle/src/main.rs".to_string(),
                size: 200_000,
            },
            torrent_handler::TorrentFileInfo {
                path: "/TestBundle/LICENSE".to_string(),
                size: 10_000,
            },
        ],
        trackers: vec!["udp://tracker.example.com:6969".to_string()],
    };

    let entries = torrent_handler::torrent_files_to_entries(&meta);
    let dirs: Vec<_> = entries
        .iter()
        .filter(|e| e.entry_type == EntryType::Folder)
        .collect();
    let files: Vec<_> = entries
        .iter()
        .filter(|e| e.entry_type == EntryType::File)
        .collect();

    // Should create 3 directories: TestBundle, TestBundle/docs, TestBundle/src
    assert_eq!(dirs.len(), 3, "3 unique parent dirs");
    assert_eq!(files.len(), 4, "4 file entries");

    // Verify file metadata
    assert_eq!(files[0].size_bytes, Some(100_000));
    assert!(files[0].raw_url.starts_with("torrent://file/abc123def456#"));
    assert_eq!(files[3].path, "/TestBundle/LICENSE");
}

#[test]
fn test_torrent_files_to_entries_single_file() {
    use crawli_lib::adapters::EntryType;

    let meta = torrent_handler::TorrentMetadata {
        name: "SingleFile".to_string(),
        info_hash: "single123".to_string(),
        total_size: 500_000,
        piece_length: 262_144,
        files: vec![torrent_handler::TorrentFileInfo {
            path: "/SingleFile".to_string(),
            size: 500_000,
        }],
        trackers: vec![],
    };

    let entries = torrent_handler::torrent_files_to_entries(&meta);
    let files: Vec<_> = entries
        .iter()
        .filter(|e| e.entry_type == EntryType::File)
        .collect();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].size_bytes, Some(500_000));
}

// ── Torrent File Size Guard Test ───────────────────────────────────

#[test]
fn test_torrent_file_not_found_fails() {
    let result = torrent_handler::parse_torrent_file("/nonexistent/path/fake.torrent");
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not found"));
}

// ── Mode Priority Tests (Mega > Torrent > Onion) ──────────────────

#[test]
fn test_detect_mode_priority_mega_over_onion() {
    // Even if URL has .onion in it, mega.nz takes priority
    assert_eq!(
        torrent_handler::detect_input_mode("https://mega.nz/folder/onion#key"),
        "mega"
    );
}

#[test]
fn test_detect_mode_whitespace_handling() {
    assert_eq!(
        torrent_handler::detect_input_mode("  https://mega.nz/folder/A#B  "),
        "mega"
    );
    assert_eq!(
        torrent_handler::detect_input_mode("  magnet:?xt=urn:btih:abc  "),
        "torrent"
    );
    assert_eq!(
        torrent_handler::detect_input_mode("  http://test.onion  "),
        "onion"
    );
}
