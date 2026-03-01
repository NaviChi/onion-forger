/// Path Utilities for OnionForge
/// Handles URL decoding, filename sanitization, and path normalization

/// Decode URL-encoded strings: %20 → space, %2F → /, etc.
/// Pure Rust implementation — no external crate needed.
pub fn url_decode(input: &str) -> String {
    let mut result = Vec::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (
                hex_val(bytes[i + 1]),
                hex_val(bytes[i + 2]),
            ) {
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
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9'
            | b'-' | b'_' | b'.' | b'~'
            | b'/' | b':' | b'@' | b'!' | b'$' | b'&'
            | b'\'' | b'(' | b')' | b'*' | b',' | b';' | b'=' => {
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

    // Strip leading slash for relative path construction
    let trimmed = decoded.trim_start_matches('/');

    // Process each path component individually
    let components: Vec<String> = trimmed
        .split('/')
        .filter(|c| !c.is_empty())
        .map(|component| sanitize_component(component))
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
    let reserved = ["CON", "PRN", "AUX", "NUL",
        "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9",
        "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9"];
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
        assert_eq!(sanitize_path("/FALOp/2%20Sally%20Personal.part01.rar"), "FALOp/2 Sally Personal.part01.rar");
        assert_eq!(sanitize_path("/dir/file<with>bad:chars?.txt"), "dir/file_with_bad_chars_.txt");
        assert_eq!(sanitize_path("///multiple///slashes///"), "multiple/slashes");
        assert_eq!(sanitize_path(""), "");
        assert_eq!(sanitize_path("/CON/test.txt"), "_CON/test.txt");
    }

    #[test]
    fn test_extract_target_dirname() {
        assert_eq!(
            extract_target_dirname("http://b3pzp6qwelgeygmzn6awkduym6s4gxh6htwxuxeydrziwzlx63zergyd.onion/FALOp"),
            "FALOp"
        );
        assert_eq!(
            extract_target_dirname("http://incblog6qu4y4mm4zvw5nrmue6qbwtgjsxpw6b7ixzssu36tsajldoad.onion/blog/disclosures/698d5c538f1d14b7436dd63b"),
            "698d5c538f1d14b7436dd63b"
        );
        assert_eq!(
            extract_target_dirname("http://m3wwhkus4dxbnxbtihexlyd2cv63qrvex6jiebc4vqe22kg2z3udebid.onion/sdeb.org/"),
            "sdeb.org"
        );
    }
}
