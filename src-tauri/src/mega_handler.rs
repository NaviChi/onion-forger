// src-tauri/src/mega_handler.rs
// Phase 52A: Mega.nz Public Folder Listing + AES-128-CTR Decrypted Downloads
//
// Uses the `mega` crate v0.8.0 API:
//   - Client::builder().build() → Client
//   - client.fetch_public_nodes(url) → Result<Nodes>
//   - Nodes.roots() / .iter() / .get_node_by_handle()
//   - Node.name() / .handle() / .size() / .kind() / .children() → &[String]
//   - client.download_node(&node, writer) — AsyncWrite-based
//
// Prevention Rules:
// - PR-MEGA-001: Never persist encryption keys to disk
// - PR-MEGA-002: Fail-fast if key segment missing from URL

use crate::adapters::{EntryType, FileEntry};
use crate::CrawlSessionResult;
use anyhow::{anyhow, Result};
use mega::{Client, Node, NodeKind, Nodes};
use tauri::{AppHandle, Emitter};

// ── Auto-detect ─────────────────────────────────────────────────────

/// Stateless, zero-network-call check for Mega.nz URLs.
pub fn is_mega_link(url: &str) -> bool {
    let lower = url.to_lowercase();
    lower.contains("mega.nz/") || lower.contains("mega.co.nz/")
}

/// Detects password-protected Mega.nz links (format: #P!payload)
pub fn is_mega_protected_link(url: &str) -> bool {
    url.contains("#P!") && is_mega_link(url)
}

// ── URL Parsing ─────────────────────────────────────────────────────

/// Extracts link metadata from a mega.nz URL.
pub fn parse_mega_url(url: &str) -> Result<MegaLinkInfo> {
    let url_trimmed = url.trim();

    // New format: mega.nz/folder/HANDLE#KEY or mega.nz/file/HANDLE#KEY
    if let Some(rest) = url_trimmed
        .split("mega.nz/folder/")
        .nth(1)
        .or_else(|| url_trimmed.split("mega.co.nz/folder/").nth(1))
    {
        let parts: Vec<&str> = rest.splitn(2, '#').collect();
        if parts.len() == 2 && !parts[0].is_empty() && !parts[1].is_empty() {
            return Ok(MegaLinkInfo {
                link_type: MegaLinkType::Folder,
                handle: parts[0].to_string(),
                key: parts[1].to_string(),
                original_url: url_trimmed.to_string(),
            });
        }
    }

    if let Some(rest) = url_trimmed
        .split("mega.nz/file/")
        .nth(1)
        .or_else(|| url_trimmed.split("mega.co.nz/file/").nth(1))
    {
        let parts: Vec<&str> = rest.splitn(2, '#').collect();
        if parts.len() == 2 && !parts[0].is_empty() && !parts[1].is_empty() {
            return Ok(MegaLinkInfo {
                link_type: MegaLinkType::File,
                handle: parts[0].to_string(),
                key: parts[1].to_string(),
                original_url: url_trimmed.to_string(),
            });
        }
    }

    // Legacy format: mega.nz/#F!HANDLE!KEY (folder) or #!HANDLE!KEY (file)
    if let Some(fragment) = url_trimmed.split('#').nth(1) {
        if let Some(rest) = fragment.strip_prefix("F!") {
            let parts: Vec<&str> = rest.splitn(2, '!').collect();
            if parts.len() == 2 {
                return Ok(MegaLinkInfo {
                    link_type: MegaLinkType::Folder,
                    handle: parts[0].to_string(),
                    key: parts[1].to_string(),
                    original_url: url_trimmed.to_string(),
                });
            }
        }
        if let Some(rest) = fragment.strip_prefix('!') {
            let parts: Vec<&str> = rest.splitn(2, '!').collect();
            if parts.len() == 2 {
                return Ok(MegaLinkInfo {
                    link_type: MegaLinkType::File,
                    handle: parts[0].to_string(),
                    key: parts[1].to_string(),
                    original_url: url_trimmed.to_string(),
                });
            }
        }
    }

    Err(anyhow!(
        "Invalid Mega.nz URL — could not extract handle and decryption key. \
         Expected format: https://mega.nz/folder/HANDLE#KEY"
    ))
}

#[derive(Debug, Clone, PartialEq)]
pub enum MegaLinkType {
    Folder,
    File,
    PasswordProtected,
}

#[derive(Debug, Clone)]
pub struct MegaLinkInfo {
    pub link_type: MegaLinkType,
    pub handle: String,
    pub key: String,
    pub original_url: String,
}

// ── Node Tree → FileEntry Conversion ────────────────────────────────

/// Recursively walks NodeKind::Folder children (via handle lookup)
/// and produces canonical FileEntry structs.
fn walk_node_tree(
    node: &Node,
    nodes: &Nodes,
    parent_path: &str,
    original_url: &str,
    entries: &mut Vec<FileEntry>,
) {
    let name = node.name();
    let path = if parent_path == "/" {
        format!("/{}", name)
    } else {
        format!("{}/{}", parent_path, name)
    };

    match node.kind() {
        NodeKind::File => {
            entries.push(FileEntry {
                jwt_exp: None,
                path,
                size_bytes: Some(node.size()),
                entry_type: EntryType::File,
                raw_url: format!("mega://file/{}#{}", node.handle(), original_url),
            });
        }
        NodeKind::Folder => {
            entries.push(FileEntry {
                jwt_exp: None,
                path: path.clone(),
                size_bytes: None,
                entry_type: EntryType::Folder,
                raw_url: format!("mega://folder/{}", node.handle()),
            });
            // children() returns &[String] — child handles, not Node objects
            for child_handle in node.children() {
                if let Some(child_node) = nodes.get_node_by_handle(child_handle) {
                    walk_node_tree(child_node, nodes, &path, original_url, entries);
                }
            }
        }
        _ => {} // Skip Root/Inbox/Trash node kinds
    }
}

// ── Main Crawl Entry Point ──────────────────────────────────────────

pub async fn mega_crawl(
    url: &str,
    output_dir: &str,
    auto_download: bool,
    password: Option<&str>,
    app: AppHandle,
) -> Result<CrawlSessionResult, String> {
    let is_protected = is_mega_protected_link(url);

    if is_protected {
        let _ = app.emit(
            "log",
            "[MEGA] Detected password-protected link (#P! format)".to_string(),
        );
    }

    let link_info = if is_protected {
        // Password-protected links don't have standard handle#key format
        MegaLinkInfo {
            link_type: MegaLinkType::PasswordProtected,
            handle: String::new(),
            key: String::new(),
            original_url: url.trim().to_string(),
        }
    } else {
        parse_mega_url(url).map_err(|e| e.to_string())?
    };

    if !is_protected {
        let _ = app.emit(
            "log",
            format!(
                "[MEGA] Detected {} link: handle={} (key present=true)",
                match link_info.link_type {
                    MegaLinkType::Folder => "folder",
                    MegaLinkType::File => "file",
                    MegaLinkType::PasswordProtected => "protected",
                },
                link_info.handle
            ),
        );
    }
    let _ = app.emit("crawl_log", "Match found: Mega.nz Handler".to_string());

    // Initialize MEGA client (no login required for public links)
    let http_client = reqwest_mega::Client::new();
    let mega = Client::builder()
        .build(http_client)
        .map_err(|e| format!("MEGA client init failed: {e}"))?;
    let _ = app.emit(
        "log",
        "[MEGA] API client initialized (no-auth public mode)".to_string(),
    );

    // Fetch nodes — use protected or public API depending on link type
    let _ = app.emit(
        "log",
        "[MEGA] Fetching node tree (AES-128-CTR decryption)...".to_string(),
    );
    let nodes = if is_protected {
        let pwd =
            password.ok_or_else(|| "Password required for protected Mega.nz link".to_string())?;
        mega.fetch_protected_nodes(&link_info.original_url, pwd)
            .await
            .map_err(|e| format!("MEGA protected node fetch failed: {e}"))?
    } else {
        mega.fetch_public_nodes(&link_info.original_url)
            .await
            .map_err(|e| format!("MEGA node fetch failed: {e}"))?
    };

    let _ = app.emit(
        "log",
        format!("[MEGA] Decrypted {} total nodes", nodes.len()),
    );

    // Convert the Nodes tree into flat FileEntry list
    let mut entries: Vec<FileEntry> = Vec::new();
    for root in nodes.roots() {
        walk_node_tree(root, &nodes, "/", &link_info.original_url, &mut entries);
    }

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
            "[MEGA] Tree expanded: {} files, {} folders",
            file_count, folder_count
        ),
    );

    // Emit to VFS
    let _ = app.emit("crawl_progress", entries.clone());

    let target_key = format!("mega_{}", link_info.handle);

    // Auto-download if enabled
    if auto_download && file_count > 0 {
        let _ = app.emit(
            "log",
            "[MEGA] Auto-download enabled — downloading via MEGA API…".to_string(),
        );
        let output_root = crate::canonical_output_root(output_dir)?;

        let file_nodes: Vec<&Node> = nodes
            .iter()
            .filter(|n| n.kind() == NodeKind::File)
            .collect();

        let total = file_nodes.len();
        let mut completed: usize = 0;
        let mut failed: usize = 0;
        let mut skipped: usize = 0;

        for (idx, node) in file_nodes.iter().enumerate() {
            let entry_path = entries
                .iter()
                .find(|e| e.raw_url.contains(node.handle()) && e.entry_type == EntryType::File)
                .map(|e| e.path.clone())
                .unwrap_or_else(|| format!("/{}", node.name()));

            let file_path = output_root.join(entry_path.trim_start_matches('/'));
            if let Some(parent) = file_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }

            // Smart-skip: if file exists with matching size, skip
            if file_path.exists() {
                if let Ok(meta) = std::fs::metadata(&file_path) {
                    if meta.len() == node.size() {
                        skipped += 1;
                        let _ = app.emit(
                            "log",
                            format!(
                                "[MEGA] [{}/{}] Skipped (exists, {} bytes): {}",
                                idx + 1,
                                total,
                                node.size(),
                                entry_path
                            ),
                        );
                        let _ = app.emit(
                            "mega_download_progress",
                            serde_json::json!({
                                "index": idx + 1, "total": total,
                                "file": entry_path, "size": node.size(),
                                "status": "skipped", "completed": completed,
                                "failed": failed, "skipped": skipped,
                            }),
                        );
                        continue;
                    }
                }
            }

            let _ = app.emit(
                "log",
                format!(
                    "[MEGA] [{}/{}] Downloading: {} ({} bytes)",
                    idx + 1,
                    total,
                    entry_path,
                    node.size()
                ),
            );
            let _ = app.emit(
                "mega_download_progress",
                serde_json::json!({
                    "index": idx + 1, "total": total,
                    "file": entry_path, "size": node.size(),
                    "status": "downloading", "completed": completed,
                    "failed": failed, "skipped": skipped,
                }),
            );

            // Single file creation — use sync File wrapped in AllowStdIo
            // (mega crate's download_node expects futures::io::AsyncWrite)
            match std::fs::File::create(&file_path) {
                Ok(sync_file) => {
                    let writer = futures::io::AllowStdIo::new(sync_file);
                    match mega.download_node(node, writer).await {
                        Ok(_) => {
                            completed += 1;
                            let _ = app.emit("log", format!("[MEGA] ✓ {}", entry_path));
                        }
                        Err(e) => {
                            failed += 1;
                            let _ = app.emit("log", format!("[MEGA] ✗ {} — {}", entry_path, e));
                        }
                    }
                }
                Err(e) => {
                    failed += 1;
                    let _ = app.emit(
                        "log",
                        format!("[MEGA] ✗ Cannot create {} — {}", entry_path, e),
                    );
                }
            }

            let _ = app.emit(
                "mega_download_progress",
                serde_json::json!({
                    "index": idx + 1, "total": total,
                    "file": entry_path, "size": node.size(),
                    "status": if failed > 0 { "error" } else { "done" },
                    "completed": completed, "failed": failed, "skipped": skipped,
                }),
            );
        }

        let _ = app.emit(
            "log",
            format!(
                "[MEGA] Download complete: {} completed, {} failed, {} skipped (of {})",
                completed, failed, skipped, total
            ),
        );
    }

    Ok(CrawlSessionResult {
        target_key,
        discovered_count: entries.len(),
        file_count,
        folder_count,
        best_prior_count: 0,
        raw_this_run_count: entries.len(),
        merged_effective_count: entries.len(),
        crawl_outcome: "mega_complete".to_string(),
        retry_count_used: 0,
        stable_current_listing_path: String::new(),
        stable_current_dirs_listing_path: String::new(),
        stable_best_listing_path: String::new(),
        stable_best_dirs_listing_path: String::new(),
        auto_download_started: auto_download,
        output_dir: output_dir.to_string(),
    })
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_mega_link() {
        assert!(is_mega_link("https://mega.nz/folder/ABC123#key456"));
        assert!(is_mega_link("https://mega.nz/file/XYZ789#key123"));
        assert!(is_mega_link("https://mega.co.nz/folder/OLD#format"));
        assert!(is_mega_link("http://mega.nz/#F!HANDLE!KEY"));
        assert!(!is_mega_link("http://example.onion/files/"));
        assert!(!is_mega_link("magnet:?xt=urn:btih:abc"));
        assert!(!is_mega_link("https://google.com"));
    }

    #[test]
    fn test_parse_mega_url_new_folder() {
        let info = parse_mega_url("https://mega.nz/folder/ABC123#decryptionKey456").unwrap();
        assert_eq!(info.link_type, MegaLinkType::Folder);
        assert_eq!(info.handle, "ABC123");
        assert_eq!(info.key, "decryptionKey456");
    }

    #[test]
    fn test_parse_mega_url_new_file() {
        let info = parse_mega_url("https://mega.nz/file/XYZ789#fileKey123").unwrap();
        assert_eq!(info.link_type, MegaLinkType::File);
        assert_eq!(info.handle, "XYZ789");
        assert_eq!(info.key, "fileKey123");
    }

    #[test]
    fn test_parse_mega_url_legacy_folder() {
        let info = parse_mega_url("https://mega.nz/#F!HANDLE!LEGACYKEY").unwrap();
        assert_eq!(info.link_type, MegaLinkType::Folder);
        assert_eq!(info.handle, "HANDLE");
        assert_eq!(info.key, "LEGACYKEY");
    }

    #[test]
    fn test_parse_mega_url_legacy_file() {
        let info = parse_mega_url("https://mega.nz/#!FILEHANDLE!FILEKEY").unwrap();
        assert_eq!(info.link_type, MegaLinkType::File);
        assert_eq!(info.handle, "FILEHANDLE");
        assert_eq!(info.key, "FILEKEY");
    }

    #[test]
    fn test_parse_mega_url_missing_key_fails() {
        assert!(parse_mega_url("https://mega.nz/folder/ABC123").is_err());
        assert!(parse_mega_url("https://mega.nz/file/").is_err());
        assert!(parse_mega_url("https://google.com").is_err());
    }
}
