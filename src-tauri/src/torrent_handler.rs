// src-tauri/src/torrent_handler.rs
// Phase 52A: BitTorrent (.torrent file + magnet link) Support
//
// Uses:
//   - `lava_torrent` crate for .torrent file parsing (bencode decode)
//   - `magnet_url` crate v3.0 API:
//       Magnet::new(uri) → Result<Magnet, MagnetError>
//       magnet.hash() → Option<&str>
//       magnet.display_name() → Option<&str>
//       magnet.trackers() → &[String]
//
// Prevention Rules:
// - PR-TORRENT-001: Never route BitTorrent through Tor
// - PR-TORRENT-002: Reject .torrent files > 10MB

use crate::adapters::{EntryType, FileEntry};
use crate::CrawlSessionResult;
use anyhow::{anyhow, Result};
use lava_torrent::torrent::v1::Torrent;
use magnet_url::Magnet;
use std::path::Path;
use tauri::{AppHandle, Emitter};

/// Maximum .torrent file size (10MB) — PR-TORRENT-002
const MAX_TORRENT_FILE_SIZE: u64 = 10 * 1024 * 1024;

// ── Auto-detect ─────────────────────────────────────────────────────

/// Stateless check for magnet URIs. Zero network calls.
pub fn is_magnet_link(input: &str) -> bool {
    input.trim().to_lowercase().starts_with("magnet:?")
}

/// Stateless check for .torrent file paths. Zero network calls.
pub fn is_torrent_file(input: &str) -> bool {
    let trimmed = input.trim().to_lowercase();
    trimmed.ends_with(".torrent") && !trimmed.starts_with("http")
}

// ── Combined mode detection ─────────────────────────────────────────

/// Returns the detected input mode as a string for the frontend.
pub fn detect_input_mode(input: &str) -> &'static str {
    if crate::mega_handler::is_mega_link(input) {
        "mega"
    } else if is_magnet_link(input) || is_torrent_file(input) {
        "torrent"
    } else {
        "onion"
    }
}

// ── Magnet Link Parsing ─────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct MagnetInfo {
    pub info_hash: String,
    pub display_name: Option<String>,
    pub trackers: Vec<String>,
    pub raw_magnet: String,
}

/// Parse a magnet URI into structured metadata.
/// Uses magnet_url v3.0 accessor methods: hash(), display_name(), trackers()
pub fn parse_magnet(magnet_uri: &str) -> Result<MagnetInfo> {
    let magnet = Magnet::new(magnet_uri).map_err(|e| anyhow!("Invalid magnet URI: {:?}", e))?;

    let info_hash = magnet
        .hash()
        .ok_or_else(|| anyhow!("Magnet link missing hash (xt field)"))?
        .to_string();

    let display_name = magnet.display_name().map(|s| s.to_string());
    let trackers: Vec<String> = magnet.trackers().to_vec();

    Ok(MagnetInfo {
        info_hash,
        display_name,
        trackers,
        raw_magnet: magnet_uri.to_string(),
    })
}

// ── .torrent File Parsing ───────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct TorrentMetadata {
    pub name: String,
    pub info_hash: String,
    pub total_size: u64,
    pub piece_length: u64,
    pub files: Vec<TorrentFileInfo>,
    pub trackers: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct TorrentFileInfo {
    pub path: String,
    pub size: u64,
}

/// Parse a .torrent file from disk into structured metadata.
/// Enforces PR-TORRENT-002: rejects files > 10MB.
pub fn parse_torrent_file(file_path: &str) -> Result<TorrentMetadata> {
    let path = Path::new(file_path);
    if !path.exists() {
        return Err(anyhow!("Torrent file not found: {}", file_path));
    }

    let metadata =
        std::fs::metadata(path).map_err(|e| anyhow!("Cannot read torrent file metadata: {}", e))?;
    if metadata.len() > MAX_TORRENT_FILE_SIZE {
        return Err(anyhow!(
            "Torrent file too large ({} bytes, max {}). Possible attack vector.",
            metadata.len(),
            MAX_TORRENT_FILE_SIZE
        ));
    }

    let torrent = Torrent::read_from_file(path)
        .map_err(|e| anyhow!("Failed to parse .torrent file: {}", e))?;

    let name = torrent.name.clone();
    let info_hash = torrent.info_hash();
    let piece_length = torrent.piece_length as u64;
    let total_size = torrent.length as u64;

    let files = if let Some(ref file_list) = torrent.files {
        file_list
            .iter()
            .map(|f| {
                let path_str = f.path.to_string_lossy().to_string();
                TorrentFileInfo {
                    path: format!("/{}/{}", name, path_str),
                    size: f.length as u64,
                }
            })
            .collect()
    } else {
        vec![TorrentFileInfo {
            path: format!("/{}", name),
            size: total_size,
        }]
    };

    let trackers = if let Some(ref announce_list) = torrent.announce_list {
        announce_list
            .iter()
            .flat_map(|tier| tier.iter().cloned())
            .collect()
    } else if let Some(ref announce) = torrent.announce {
        vec![announce.clone()]
    } else {
        vec![]
    };

    Ok(TorrentMetadata {
        name,
        info_hash,
        total_size,
        piece_length,
        files,
        trackers,
    })
}

// ── FileEntry Conversion ────────────────────────────────────────────

pub fn torrent_files_to_entries(meta: &TorrentMetadata) -> Vec<FileEntry> {
    let mut entries = Vec::new();
    let mut seen_dirs = std::collections::HashSet::new();

    for file in &meta.files {
        let parts: Vec<&str> = file.path.trim_start_matches('/').split('/').collect();
        let mut dir_path = String::new();
        for part in &parts[..parts.len().saturating_sub(1)] {
            dir_path = format!("{}/{}", dir_path, part);
            if seen_dirs.insert(dir_path.clone()) {
                entries.push(FileEntry {
                    jwt_exp: None,
                    path: dir_path.clone(),
                    size_bytes: None,
                    entry_type: EntryType::Folder,
                    raw_url: format!("torrent://dir/{}", dir_path),
                });
            }
        }

        entries.push(FileEntry {
            jwt_exp: None,
            path: file.path.clone(),
            size_bytes: Some(file.size),
            entry_type: EntryType::File,
            raw_url: format!("torrent://file/{}#{}", meta.info_hash, file.path),
        });
    }

    entries
}

// ── Main Crawl Entry Point ──────────────────────────────────────────

pub async fn torrent_crawl(
    input: &str,
    output_dir: &str,
    auto_download: bool,
    app: AppHandle,
) -> Result<CrawlSessionResult, String> {
    let _ = app.emit("crawl_log", "Match found: BitTorrent Handler".to_string());

    let meta = if is_torrent_file(input) {
        let _ = app.emit("log", format!("[TORRENT] Parsing .torrent file: {}", input));
        parse_torrent_file(input).map_err(|e| e.to_string())?
    } else if is_magnet_link(input) {
        let magnet_info = parse_magnet(input).map_err(|e| e.to_string())?;
        let _ = app.emit(
            "log",
            format!(
                "[TORRENT] Parsed magnet link: hash={} name={:?} trackers={}",
                magnet_info.info_hash,
                magnet_info.display_name,
                magnet_info.trackers.len()
            ),
        );

        let display_name = magnet_info
            .display_name
            .unwrap_or_else(|| magnet_info.info_hash.clone());
        TorrentMetadata {
            name: display_name.clone(),
            info_hash: magnet_info.info_hash,
            total_size: 0,
            piece_length: 0,
            files: vec![TorrentFileInfo {
                path: format!("/{}", display_name),
                size: 0,
            }],
            trackers: magnet_info.trackers,
        }
    } else {
        return Err("Input is neither a valid .torrent file path nor a magnet link".to_string());
    };

    let entries = torrent_files_to_entries(&meta);
    let file_count = entries
        .iter()
        .filter(|e| e.entry_type == EntryType::File)
        .count();
    let folder_count = entries
        .iter()
        .filter(|e| e.entry_type == EntryType::Folder)
        .count();

    let _ = app.emit(
        "log",
        format!(
            "[TORRENT] {} — {} files, {} folders, total {} bytes, trackers={}",
            meta.name,
            file_count,
            folder_count,
            meta.total_size,
            meta.trackers.len()
        ),
    );

    let _ = app.emit("crawl_progress", entries.clone());

    let target_key = format!("torrent_{}", meta.info_hash);

    if auto_download {
        let _ = app.emit(
            "log",
            "[TORRENT] Auto-download: Starting librqbit download engine (clearnet)".to_string(),
        );

        match torrent_download(input, output_dir, &app).await {
            Ok(()) => {
                let _ = app.emit("log", "[TORRENT] ✓ Download complete".to_string());
            }
            Err(e) => {
                let _ = app.emit("log", format!("[TORRENT] ✗ Download failed: {}", e));
            }
        }
    }

    Ok(CrawlSessionResult {
        target_key,
        discovered_count: entries.len(),
        file_count,
        folder_count,
        best_prior_count: 0,
        raw_this_run_count: entries.len(),
        merged_effective_count: entries.len(),
        crawl_outcome: if auto_download {
            "torrent_downloaded".to_string()
        } else {
            "torrent_listed".to_string()
        },
        retry_count_used: 0,
        stable_current_listing_path: String::new(),
        stable_current_dirs_listing_path: String::new(),
        stable_best_listing_path: String::new(),
        stable_best_dirs_listing_path: String::new(),
        auto_download_started: auto_download,
        output_dir: output_dir.to_string(),
    })
}

// ── librqbit Download Engine ────────────────────────────────────────

/// Actual BitTorrent piece-mode download via librqbit.
/// PR-TORRENT-001: Never routes through Tor — uses clearnet directly.
async fn torrent_download(input: &str, output_dir: &str, app: &AppHandle) -> Result<(), String> {
    use librqbit::{AddTorrent, Session, SessionOptions, SessionPersistenceConfig};

    let output_root = crate::canonical_output_root(output_dir)?;
    let download_dir = output_root.to_string_lossy().to_string();

    let _ = app.emit("log", format!("[TORRENT] Download dir: {}", download_dir));

    // Create a librqbit session with persistence for resume support
    let opts = SessionOptions {
        fastresume: true,
        persistence: Some(SessionPersistenceConfig::Json {
            folder: Some(output_root.join(".crawli_torrent_state").into()),
        }),
        disable_dht: false,
        disable_dht_persistence: false,
        enable_upnp_port_forwarding: false,
        ..Default::default()
    };

    let session = Session::new_with_opts(download_dir.into(), opts)
        .await
        .map_err(|e| format!("librqbit session init failed: {e}"))?;

    let _ = app.emit(
        "log",
        "[TORRENT] librqbit session initialized (fastresume=on, upload=disabled)".to_string(),
    );

    // Add the torrent — supports both magnet URIs and file:/// URLs
    let add_torrent = if is_magnet_link(input) {
        AddTorrent::from_url(input)
    } else if is_torrent_file(input) {
        // For file paths, read the bytes and pass them
        let bytes = std::fs::read(input).map_err(|e| format!("Cannot read .torrent file: {e}"))?;
        AddTorrent::from_bytes(bytes)
    } else {
        return Err("Invalid torrent input".to_string());
    };

    let _ = app.emit("log", "[TORRENT] Adding torrent to session…".to_string());

    let handle = session
        .add_torrent(add_torrent, None)
        .await
        .map_err(|e| format!("Failed to add torrent: {e}"))?
        .into_handle()
        .ok_or_else(|| "Torrent added but listed-only (no handle)".to_string())?;

    let _ = app.emit(
        "log",
        "[TORRENT] Torrent added — downloading pieces…".to_string(),
    );

    // Progress polling loop — every 500ms
    let app_progress = app.clone();
    let poll_handle = handle.clone();
    let progress_task = tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;

            let stats = poll_handle.stats();
            let progress_pct =
                stats.progress_bytes as f64 / stats.total_bytes.max(1) as f64 * 100.0;

            // LiveStats fields: snapshot, download_speed (Speed), upload_speed (Speed)
            // Speed implements Display for human-readable formatting
            let speed_str = match &stats.live {
                Some(live) => format!("{}", live.download_speed),
                None => "0 B/s".to_string(),
            };

            let _ = app_progress.emit(
                "torrent_download_progress",
                serde_json::json!({
                    "downloaded_bytes": stats.progress_bytes,
                    "total_bytes": stats.total_bytes,
                    "progress_pct": format!("{:.1}", progress_pct),
                    "download_speed": speed_str,
                    "status": if stats.finished { "complete" } else { "downloading" },
                }),
            );

            let _ = app_progress.emit(
                "log",
                format!(
                    "[TORRENT] Progress: {:.1}% ({}/{}), speed={}",
                    progress_pct, stats.progress_bytes, stats.total_bytes, speed_str,
                ),
            );

            if stats.finished {
                break;
            }
        }
    });

    // Wait for download to complete
    handle
        .wait_until_completed()
        .await
        .map_err(|e| format!("Torrent download failed: {e}"))?;

    // Cancel progress polling (it should exit on its own, but be safe)
    progress_task.abort();

    let _ = app.emit(
        "torrent_download_progress",
        serde_json::json!({
            "status": "complete",
            "downloaded_bytes": 0,
            "total_bytes": 0,
            "progress_pct": "100.0",
            "download_speed": "0 B/s",
        }),
    );

    Ok(())
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_magnet_link() {
        assert!(is_magnet_link("magnet:?xt=urn:btih:abc123"));
        assert!(is_magnet_link("MAGNET:?xt=urn:btih:ABC123"));
        assert!(is_magnet_link("  magnet:?xt=urn:btih:abc  "));
        assert!(!is_magnet_link("https://mega.nz/folder/ABC#KEY"));
        assert!(!is_magnet_link("http://example.onion/files/"));
        assert!(!is_magnet_link("/path/to/file.torrent"));
    }

    #[test]
    fn test_is_torrent_file() {
        assert!(is_torrent_file("/path/to/ubuntu.torrent"));
        assert!(is_torrent_file("~/Downloads/movie.TORRENT"));
        assert!(is_torrent_file("  /tmp/test.torrent  "));
        assert!(!is_torrent_file("http://example.com/file.torrent"));
        assert!(!is_torrent_file("magnet:?xt=urn:btih:abc"));
    }

    #[test]
    fn test_detect_input_mode() {
        assert_eq!(detect_input_mode("https://mega.nz/folder/ABC#KEY"), "mega");
        assert_eq!(detect_input_mode("magnet:?xt=urn:btih:abc"), "torrent");
        assert_eq!(detect_input_mode("/tmp/test.torrent"), "torrent");
        assert_eq!(detect_input_mode("http://example.onion/files/"), "onion");
        assert_eq!(detect_input_mode("https://google.com"), "onion");
    }

    #[test]
    fn test_parse_magnet_basic() {
        let info = parse_magnet(
            "magnet:?xt=urn:btih:da39a3ee5e6b4b0d3255bfef95601890afd80709&dn=TestFile&tr=udp://tracker.example.com:6969"
        ).unwrap();
        assert_eq!(info.info_hash, "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(info.display_name, Some("TestFile".to_string()));
        assert!(!info.trackers.is_empty());
    }

    #[test]
    fn test_torrent_files_to_entries() {
        let meta = TorrentMetadata {
            name: "TestTorrent".to_string(),
            info_hash: "abc123".to_string(),
            total_size: 1_000_000,
            piece_length: 262_144,
            files: vec![
                TorrentFileInfo {
                    path: "/TestTorrent/dir1/file1.txt".to_string(),
                    size: 500_000,
                },
                TorrentFileInfo {
                    path: "/TestTorrent/dir1/file2.txt".to_string(),
                    size: 300_000,
                },
                TorrentFileInfo {
                    path: "/TestTorrent/readme.md".to_string(),
                    size: 200_000,
                },
            ],
            trackers: vec!["udp://tracker.example.com:6969".to_string()],
        };

        let entries = torrent_files_to_entries(&meta);
        let dirs: Vec<_> = entries
            .iter()
            .filter(|e| e.entry_type == EntryType::Folder)
            .collect();
        let files: Vec<_> = entries
            .iter()
            .filter(|e| e.entry_type == EntryType::File)
            .collect();

        assert_eq!(dirs.len(), 2, "Should create 2 directories");
        assert_eq!(files.len(), 3, "Should have 3 file entries");
        assert_eq!(files[0].size_bytes, Some(500_000));
        assert!(files[0].raw_url.starts_with("torrent://file/abc123#"));
    }
}
