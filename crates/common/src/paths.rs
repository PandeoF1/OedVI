//! Validation of the relative paths the server publishes.
//!
//! The client writes files at paths that come from the network, so every path
//! is checked before it is joined to a local directory.

use std::path::{Path, PathBuf};

/// A manifest path: `/`-separated, relative, no `.`/`..`, no drive letters,
/// no characters Windows refuses in file names.
pub fn is_safe_relative(path: &str) -> bool {
    if path.is_empty() || path.starts_with('/') || path.contains('\\') || path.contains('\0') {
        return false;
    }
    path.split('/').all(|part| {
        !part.is_empty()
            && part != "."
            && part != ".."
            && !part.ends_with('.')
            && !part.ends_with(' ')
            && !part.chars().any(|c| matches!(c, ':' | '*' | '?' | '"' | '<' | '>' | '|') || (c as u32) < 32)
    })
}

/// Joins a validated manifest path to `root`.
pub fn join_safe(root: &Path, path: &str) -> Option<PathBuf> {
    if !is_safe_relative(path) {
        return None;
    }
    let mut out = root.to_path_buf();
    for part in path.split('/') {
        out.push(part);
    }
    Some(out)
}

/// `/`-separated relative path of `path` under `root`.
pub fn to_manifest_path(root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).ok()?;
    let parts: Option<Vec<&str>> = rel.components().map(|c| c.as_os_str().to_str()).collect();
    Some(parts?.join("/"))
}

/// Builder and template ids: used as file names on the server and as
/// directory names on the client.
pub fn is_valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
        && !id.starts_with('.')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_escapes() {
        assert!(is_safe_relative("a/b.rpf"));
        assert!(is_safe_relative("movies/intro.bk2"));
        for bad in ["", "/etc/passwd", "../x", "a/../b", "a//b", "C:/x", "a\\b", "a/./b", "con.", "a/b "] {
            assert!(!is_safe_relative(bad), "{bad}");
        }
    }

    #[test]
    fn ids() {
        assert!(is_valid_id("amd-release_2"));
        assert!(!is_valid_id("../x"));
        assert!(!is_valid_id(".hidden"));
        assert!(!is_valid_id("a b"));
    }
}
