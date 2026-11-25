use serde::{Deserialize, Serialize};

/// Everything the server publishes about the game files.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Manifest {
    /// Incremented each time the content (paths, hashes, groups) changes.
    pub version: u64,
    /// Unix time, seconds.
    pub generated_at: u64,
    pub chunk_size: u64,
    pub total_size: u64,
    pub groups: Vec<GroupInfo>,
    pub files: Vec<FileEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GroupInfo {
    pub name: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub file_count: u64,
    #[serde(default)]
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FileEntry {
    /// `/`-separated, relative to the game directory.
    pub path: String,
    pub size: u64,
    /// BLAKE3 file hash (see `hash::file_hash`), hex.
    pub hash: String,
    /// BLAKE3 hash of each chunk, hex.
    pub chunks: Vec<String>,
    pub group: String,
    /// Required files are always downloaded and cannot be deselected.
    #[serde(default)]
    pub required: bool,
}

impl Manifest {
    pub fn find(&self, path: &str) -> Option<&FileEntry> {
        self.files.iter().find(|f| f.path == path)
    }
}

/// State of the server's scan of the game directory.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScanStatus {
    pub scanning: bool,
    pub files_total: u64,
    pub files_done: u64,
    pub bytes_total: u64,
    /// Bytes that had to be hashed (files whose size or date changed).
    pub bytes_to_hash: u64,
    pub bytes_hashed: u64,
    pub current: Option<String>,
    pub manifest_version: Option<u64>,
    pub last_scan_at: Option<u64>,
    pub last_scan_secs: Option<f64>,
    pub last_error: Option<String>,
}
