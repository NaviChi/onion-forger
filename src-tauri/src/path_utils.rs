/// Path Utilities for OnionForge
/// Handles URL decoding, filename sanitization, and path normalization
use std::path::{Path, PathBuf};

/// Decode URL-encoded strings: %20 → space, %2F → /, etc.
/// Pure Rust implementation — no external crate needed.
pub fn url_decode(input: &str) -> String {
    let mut result = Vec::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                result.push(hi * 16 + lo);
                i += 3;
                continue;
            }
        }
        // Also decode '+' as space (common in query strings)
        if bytes[i] == b'+' {
            result.push(b' ');
        } else {
            result.push(bytes[i]);
        }
        i += 1;
    }

    String::from_utf8(result).unwrap_or_else(|_| input.to_string())
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// URL-encode a string for use in raw_url fields.
/// Only encodes characters that are unsafe in URLs.
pub fn url_encode(input: &str) -> String {
    let mut result = String::with_capacity(input.len() * 3);
    for b in input.bytes() {
        match b {
            // Safe characters that don't need encoding
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'_'
            | b'.'
            | b'~'
            | b'/'
            | b':'
            | b'@'
            | b'!'
            | b'$'
            | b'&'
            | b'\''
            | b'('
            | b')'
            | b'*'
            | b','
            | b';'
            | b'=' => {
                result.push(b as char);
            }
            b' ' => result.push_str("%20"),
            _ => {
                result.push_str(&format!("%{:02X}", b));
            }
        }
    }
    result
}

/// Sanitize a filename/path component for safe filesystem use.
/// - Strips characters illegal on Windows/macOS/Linux
/// - Decodes URL-encoded sequences first (%20 → space, etc.)
/// - Preserves spaces, dots, dashes, underscores
/// - Handles edge cases: empty strings, leading/trailing dots, reserved names
pub fn sanitize_path(raw_path: &str) -> String {
    // First decode any URL-encoded sequences
    let decoded = url_decode(raw_path);
    // Normalize Windows separators into forward slashes
    let normalized = decoded.replace('\\', "/");

    // Strip leading slash for relative path construction
    let trimmed = normalized.trim_start_matches('/');

    // Process each path component individually
    let components: Vec<String> = trimmed
        .split('/')
        .filter(|component| !component.is_empty() && *component != "." && *component != "..")
        .map(sanitize_component)
        .filter(|component| !component.is_empty() && component != "." && component != "..")
        .collect();

    // Rejoin with forward slashes
    components.join("/")
}

/// Sanitize a single path component (filename or directory name)
fn sanitize_component(name: &str) -> String {
    let mut clean = String::with_capacity(name.len());

    for ch in name.chars() {
        match ch {
            // Illegal on Windows
            '<' | '>' | ':' | '"' | '|' | '?' | '*' => clean.push('_'),
            // Control characters
            c if c.is_control() => continue,
            // Null byte
            '\0' => continue,
            // Everything else is fine (spaces, dots, dashes, unicode, etc.)
            _ => clean.push(ch),
        }
    }

    // Handle Windows reserved names
    let upper = clean.to_uppercase();
    let reserved = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    if reserved.contains(&upper.as_str()) {
        clean = format!("_{}", clean);
    }

    // Strip trailing dots and spaces (Windows issue)
    clean = clean.trim_end_matches(['.', ' ']).to_string();

    // If somehow empty after sanitization, give it a safe name
    if clean.is_empty() {
        clean = "_unnamed".to_string();
    }

    clean
}

/// Parses a human-readable size string (e.g. "912.0 KiB", "1.5 M", "1024") into raw bytes.
pub fn parse_size(size_str: &str) -> Option<u64> {
    let raw = size_str.trim().to_uppercase();
    if raw.is_empty() || raw == "-" || raw == "--" {
        return None;
    }

    // Direct byte parsing (including 0)
    if let Ok(bytes) = raw.parse::<u64>() {
        return Some(bytes);
    }

    // Human readable parsing
    let mut num_str = String::new();
    let mut multiplier: u64 = 1;

    for c in raw.chars() {
        if c.is_ascii_digit() || c == '.' {
            num_str.push(c);
        } else if c == 'K' {
            multiplier = 1024;
            break;
        } else if c == 'M' {
            multiplier = 1024 * 1024;
            break;
        } else if c == 'G' {
            multiplier = 1024 * 1024 * 1024;
            break;
        } else if c == 'T' {
            multiplier = 1024 * 1024 * 1024 * 1024;
            break;
        }
    }

    if let Ok(num) = num_str.parse::<f64>() {
        return Some((num * multiplier as f64) as u64);
    }

    None
}

/// Extracts a structural, URL-agnostic logical path footprint from dynamic frontend requests.
/// This prevents cross-domain or UUID-rotation amnesia in the crawler.
pub fn extract_agnostic_path(url: &str) -> String {
    // 1. DragonForce / Next.js SPA routing (extracts `path` query param)
    if url.contains("?path=") || url.contains("&path=") {
        if let Ok(parsed) = reqwest::Url::parse(url) {
            for (k, v) in parsed.query_pairs() {
                if k == "path" {
                    let sanitized = sanitize_path(&v);
                    return if sanitized.is_empty() {
                        "_root".to_string()
                    } else {
                        sanitized
                    };
                }
            }
        }
    }

    // 2. Qilin / QData CMS internal router mapping
    // Matches patterns like `/site/data?uuid=xyz/Finance/Q3/file.txt` -> `/Finance/Q3/file.txt`
    if let Some(uuid_idx) = url.find("uuid=") {
        let after_uuid = &url[uuid_idx + 5..];
        // The UUID string ends at the first slash or ampersand
        if let Some(slash_idx) = after_uuid.find('/') {
            let amp_idx = after_uuid.find('&').unwrap_or(usize::MAX);
            if slash_idx < amp_idx {
                let logical_path = &after_uuid[slash_idx..];
                let sanitized = sanitize_path(logical_path);
                return if sanitized.is_empty() {
                    "_root".to_string()
                } else {
                    sanitized
                };
            }
        }
    }

    // 3. Fallback: Standard Traversal (Play, LockBit, Autoindex)
    if let Ok(parsed) = reqwest::Url::parse(url) {
        let sanitized = sanitize_path(parsed.path());
        return if sanitized.is_empty() {
            "_root".to_string()
        } else {
            sanitized
        };
    }

    url.to_string()
}

pub fn canonicalize_output_root(output_dir: &str) -> std::io::Result<PathBuf> {
    if output_dir.trim().is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Output directory cannot be empty",
        ));
    }

    let raw = PathBuf::from(output_dir);
    std::fs::create_dir_all(&raw)?;
    let canonical = std::fs::canonicalize(&raw)?;
    if !canonical.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Output root is not a directory",
        ));
    }
    Ok(ensure_long_path(canonical))
}

/// On Windows, prepend `\\?\` to bypass the 260-character MAX_PATH limit.
/// On other platforms, returns the path unchanged.
/// This is critical for deeply nested Qilin paths like
/// `HR/Active Employees/Gonzalez, Jander A 20 Term 1.31.2018/...`
pub fn ensure_long_path(path: PathBuf) -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        let s = path.to_string_lossy();
        if !s.starts_with("\\\\?\\") {
            return PathBuf::from(format!("\\\\?\\{}", s));
        }
    }
    path
}

/// Removes the Windows extended-length path prefix from display strings so logs
/// and UI error messages show normal operator-facing paths.
pub fn normalize_windows_device_path(raw: &str) -> String {
    if let Some(rest) = raw.strip_prefix("\\\\?\\UNC\\") {
        return format!("\\\\{}", rest);
    }
    if let Some(rest) = raw.strip_prefix("\\\\?\\") {
        return rest.to_string();
    }
    raw.to_string()
}

pub fn display_path(path: &Path) -> String {
    normalize_windows_device_path(&path.to_string_lossy())
}

/// Phase 139: Normalize a path for reliable `starts_with` comparisons.
///
/// On Windows, `std::fs::canonicalize()` returns `\\?\C:\...` while user-constructed
/// paths may or may not have the prefix. This mismatch causes `PathBuf::starts_with()`
/// to return incorrect results. This function strips the extended-length prefix so
/// both sides of `starts_with()` use the same format.
///
/// Example: `\\?\C:\output\HR\file.pdf` → `C:\output\HR\file.pdf`
fn normalize_for_starts_with(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{}", rest));
    }
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        return PathBuf::from(rest);
    }
    path.to_path_buf()
}

pub fn resolve_path_within_root(
    output_root: &Path,
    raw_path: &str,
    is_directory: bool,
) -> std::io::Result<Option<PathBuf>> {
    let sanitized = sanitize_path(raw_path);
    if sanitized.is_empty() {
        return Ok(None);
    }

    // Phase 139: Always join with output_root — sanitize_path strips leading
    // slashes and `..` so the result is always a clean relative path.
    let joined = output_root.join(&sanitized);
    // Normalize the output_root for consistent starts_with checks.
    // On Windows, canonicalize may add/remove \\?\ prefix inconsistently.
    let normalized_root = normalize_for_starts_with(output_root);
    if is_directory {
        std::fs::create_dir_all(&joined)?;
        let canonical = std::fs::canonicalize(&joined)?;
        let normalized_canonical = normalize_for_starts_with(&canonical);
        if !normalized_canonical.starts_with(&normalized_root) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "Resolved directory escaped output root",
            ));
        }
        return Ok(Some(canonical));
    }

    let parent = joined.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Resolved file path has no parent directory",
        )
    })?;
    std::fs::create_dir_all(parent)?;
    let canonical_parent = std::fs::canonicalize(parent)?;
    let normalized_parent = normalize_for_starts_with(&canonical_parent);
    if !normalized_parent.starts_with(&normalized_root) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "Resolved file parent escaped output root",
        ));
    }

    let file_name = joined.file_name().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Resolved file path has no file name",
        )
    })?;
    Ok(Some(canonical_parent.join(file_name)))
}

pub fn resolve_download_target_within_root(
    output_root: &Path,
    requested_path: &str,
) -> std::io::Result<PathBuf> {
    // Phase 139 FIX: ALWAYS sanitize first. On Windows, paths like
    // "/HR/Reports/file.pdf" are treated as absolute by PathBuf::is_absolute()
    // because they root to the current drive (C:\HR\...).  Adapter paths always
    // start with "/" and are logically relative to the output root.  By running
    // sanitize_path() first — which strips leading slashes and `..` — we
    // guarantee the path is joined correctly with output_root.
    let sanitized = sanitize_path(requested_path);
    if sanitized.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Requested download path is empty after sanitization",
        ));
    }
    let candidate = output_root.join(&sanitized);

    let parent = candidate.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Requested download target has no parent directory",
        )
    })?;
    std::fs::create_dir_all(parent)?;
    let canonical_parent = std::fs::canonicalize(parent)?;
    let normalized_root = normalize_for_starts_with(output_root);
    let normalized_parent = normalize_for_starts_with(&canonical_parent);
    if !normalized_parent.starts_with(&normalized_root) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "Download target escaped output root",
        ));
    }

    let file_name = candidate.file_name().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Requested download target has no file name",
        )
    })?;
    Ok(canonical_parent.join(file_name))
}

/// Extract a clean directory name from a URL for the output folder.
/// e.g. "http://b3pzp6...onion/FALOp" → "FALOp"
/// e.g. "http://inc...onion/blog/disclosures/698d" → "698d"
/// Falls back to the onion hash if no path segments exist.
pub fn extract_target_dirname(target_url: &str) -> String {
    // Strip protocol
    let without_proto = target_url
        .trim_start_matches("http://")
        .trim_start_matches("https://");

    // Split host and path
    let parts: Vec<&str> = without_proto.splitn(2, '/').collect();

    if parts.len() > 1 && !parts[1].is_empty() {
        // Use the last non-empty path segment
        let path = parts[1].trim_end_matches('/');
        let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        if let Some(last) = segments.last() {
            return sanitize_component(&url_decode(last));
        }
    }

    // Fallback: use the onion host (first 16 chars)
    let host = parts[0].trim_end_matches(".onion");
    let safe = if host.len() > 16 { &host[..16] } else { host };
    sanitize_component(safe)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_url_decode_basic() {
        assert_eq!(url_decode("hello%20world"), "hello world");
        assert_eq!(url_decode("file%2Fname.txt"), "file/name.txt");
        assert_eq!(url_decode("no+encoding+needed"), "no encoding needed");
        assert_eq!(url_decode("already-clean"), "already-clean");
        assert_eq!(url_decode("%E2%9C%93"), "✓"); // UTF-8 checkmark
    }

    #[test]
    fn test_url_encode_basic() {
        assert_eq!(url_encode("hello world"), "hello%20world");
        assert_eq!(url_encode("file/name.txt"), "file/name.txt");
        assert_eq!(url_encode("safe-chars_ok.zip"), "safe-chars_ok.zip");
    }

    #[test]
    fn test_sanitize_path() {
        assert_eq!(
            sanitize_path("/FALOp/2%20Sally%20Personal.part01.rar"),
            "FALOp/2 Sally Personal.part01.rar"
        );
        assert_eq!(
            sanitize_path("/dir/file<with>bad:chars?.txt"),
            "dir/file_with_bad_chars_.txt"
        );
        assert_eq!(
            sanitize_path("///multiple///slashes///"),
            "multiple/slashes"
        );
        assert_eq!(sanitize_path(""), "");
        assert_eq!(sanitize_path("/CON/test.txt"), "_CON/test.txt");
        assert_eq!(
            sanitize_path("..\\..\\Windows\\System32\\drivers\\etc\\hosts"),
            "Windows/System32/drivers/etc/hosts"
        );
        assert_eq!(sanitize_path("././safe/./file.txt"), "safe/file.txt");
    }

    #[test]
    fn test_extract_target_dirname() {
        assert_eq!(
            extract_target_dirname(
                "http://b3pzp6qwelgeygmzn6awkduym6s4gxh6htwxuxeydrziwzlx63zergyd.onion/FALOp"
            ),
            "FALOp"
        );
        assert_eq!(
            extract_target_dirname("http://incblog6qu4y4mm4zvw5nrmue6qbwtgjsxpw6b7ixzssu36tsajldoad.onion/blog/disclosures/698d5c538f1d14b7436dd63b"),
            "698d5c538f1d14b7436dd63b"
        );
        assert_eq!(
            extract_target_dirname(
                "http://m3wwhkus4dxbnxbtihexlyd2cv63qrvex6jiebc4vqe22kg2z3udebid.onion/sdeb.org/"
            ),
            "sdeb.org"
        );
    }

    #[test]
    fn test_resolve_download_target_within_root_blocks_escape() {
        let temp = std::env::temp_dir().join("crawli_path_utils_test");
        let _ = std::fs::remove_dir_all(&temp);
        std::fs::create_dir_all(&temp).unwrap();
        let root = std::fs::canonicalize(&temp).unwrap();

        let ok = resolve_download_target_within_root(&root, "safe/file.bin").unwrap();
        assert!(ok.starts_with(&root));

        let escape = root.parent().unwrap().join("outside.bin");
        let err = resolve_download_target_within_root(&root, escape.to_string_lossy().as_ref());
        assert!(err.is_err());

        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn test_phase_139_leading_slash_preserves_folder_structure() {
        let temp = std::env::temp_dir().join("crawli_phase139_test");
        let _ = std::fs::remove_dir_all(&temp);
        std::fs::create_dir_all(&temp).unwrap();
        let root = std::fs::canonicalize(&temp).unwrap();

        // Adapter paths typically start with "/" — these should be joined
        // with the output root, NOT treated as absolute drive-root paths.
        let result = resolve_download_target_within_root(&root, "/HR/Reports/file.pdf").unwrap();
        let result_norm = normalize_for_starts_with(&result);
        let root_norm = normalize_for_starts_with(&root);
        assert!(
            result_norm.starts_with(&root_norm),
            "Path {:?} should be under root {:?}",
            result_norm,
            root_norm
        );
        // The parent chain should include the HR/Reports directories
        let parent = result.parent().unwrap();
        assert!(parent.ends_with("HR/Reports") || parent.to_string_lossy().contains("HR"));

        // Nested adapter paths
        let nested = resolve_download_target_within_root(
            &root,
            "/Finance/Q3 2025/Invoices/payment.xlsx",
        )
        .unwrap();
        let nested_norm = normalize_for_starts_with(&nested);
        assert!(nested_norm.starts_with(&root_norm));
        assert!(nested.file_name().unwrap() == "payment.xlsx");

        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn test_phase_139_resolve_path_within_root_with_leading_slash() {
        let temp = std::env::temp_dir().join("crawli_phase139_resolve_test");
        let _ = std::fs::remove_dir_all(&temp);
        std::fs::create_dir_all(&temp).unwrap();
        let root = std::fs::canonicalize(&temp).unwrap();

        // File path with leading slash
        let result = resolve_path_within_root(&root, "/Documents/report.pdf", false)
            .unwrap()
            .expect("Should resolve to a path");
        let result_norm = normalize_for_starts_with(&result);
        let root_norm = normalize_for_starts_with(&root);
        assert!(result_norm.starts_with(&root_norm));
        assert!(result.file_name().unwrap() == "report.pdf");

        // Directory path with leading slash
        let dir_result = resolve_path_within_root(&root, "/Archive/2025/Q1", true)
            .unwrap()
            .expect("Should resolve to a dir path");
        let dir_norm = normalize_for_starts_with(&dir_result);
        assert!(dir_norm.starts_with(&root_norm));

        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn test_phase_139_normalize_for_starts_with() {
        use std::path::PathBuf;

        let with_prefix = PathBuf::from(r"\\?\C:\Users\output\HR\file.pdf");
        let normalized = normalize_for_starts_with(&with_prefix);
        assert_eq!(normalized, PathBuf::from(r"C:\Users\output\HR\file.pdf"));

        let without_prefix = PathBuf::from(r"C:\Users\output\HR\file.pdf");
        let normalized2 = normalize_for_starts_with(&without_prefix);
        assert_eq!(normalized2, without_prefix);

        let unc = PathBuf::from(r"\\?\UNC\server\share\folder");
        let unc_normalized = normalize_for_starts_with(&unc);
        assert_eq!(unc_normalized, PathBuf::from(r"\\server\share\folder"));

        // Unix-style path (no-op)
        let unix = PathBuf::from("/home/user/output/file.pdf");
        assert_eq!(normalize_for_starts_with(&unix), unix);
    }
}
