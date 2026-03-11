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

pub fn resolve_path_within_root(
    output_root: &Path,
    raw_path: &str,
    is_directory: bool,
) -> std::io::Result<Option<PathBuf>> {
    let sanitized = sanitize_path(raw_path);
    if sanitized.is_empty() {
        return Ok(None);
    }

    let joined = output_root.join(&sanitized);
    if is_directory {
        std::fs::create_dir_all(&joined)?;
        let canonical = std::fs::canonicalize(&joined)?;
        if !canonical.starts_with(output_root) {
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
    if !canonical_parent.starts_with(output_root) {
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
    let requested = PathBuf::from(requested_path);
    let candidate = if requested.is_absolute() {
        requested
    } else {
        let sanitized = sanitize_path(requested_path);
        if sanitized.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Requested download path is empty after sanitization",
            ));
        }
        output_root.join(sanitized)
    };

    let parent = candidate.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Requested download target has no parent directory",
        )
    })?;
    std::fs::create_dir_all(parent)?;
    let canonical_parent = std::fs::canonicalize(parent)?;
    if !canonical_parent.starts_with(output_root) {
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
    fn test_normalize_windows_device_path_for_display() {
        assert_eq!(
            normalize_windows_device_path(r"\\?\X:\Exports\Case1"),
            r"X:\Exports\Case1"
        );
        assert_eq!(
            normalize_windows_device_path(r"\\?\UNC\server\share\Exports"),
            r"\\server\share\Exports"
        );
        assert_eq!(
            normalize_windows_device_path(r"X:\Exports\Case1"),
            r"X:\Exports\Case1"
        );
    }
}
