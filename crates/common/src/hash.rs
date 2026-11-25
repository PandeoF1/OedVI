//! Chunked BLAKE3 hashing.
//!
//! A file is cut into fixed-size chunks. Each chunk is hashed on its own and
//! the file hash is the hash of the chunk hashes. This lets the client check
//! and download a damaged or updated file chunk by chunk (HTTP Range) instead
//! of fetching a whole multi-gigabyte archive again, and the file is read only
//! once to produce both.

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

/// 32 MiB: about 8,000 chunks for 250 GB, which keeps the manifest small.
pub const DEFAULT_CHUNK_SIZE: u64 = 32 * 1024 * 1024;

/// Below this size, multithreading costs more than it saves.
const RAYON_THRESHOLD: usize = 1024 * 1024;

pub type Digest = [u8; 32];

pub fn hash_chunk(data: &[u8]) -> Digest {
    let mut hasher = blake3::Hasher::new();
    if data.len() >= RAYON_THRESHOLD {
        hasher.update_rayon(data);
    } else {
        hasher.update(data);
    }
    *hasher.finalize().as_bytes()
}

/// Hash of a whole file from its size and chunk hashes.
pub fn file_hash(size: u64, chunks: &[Digest]) -> Digest {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"oedvi-file-v1");
    hasher.update(&size.to_le_bytes());
    for chunk in chunks {
        hasher.update(chunk);
    }
    *hasher.finalize().as_bytes()
}

pub fn chunk_count(size: u64, chunk_size: u64) -> u64 {
    size.div_ceil(chunk_size)
}

/// Byte range `[start, end)` of chunk `index`.
pub fn chunk_range(index: u64, size: u64, chunk_size: u64) -> (u64, u64) {
    let start = index * chunk_size;
    (start, (start + chunk_size).min(size))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDigest {
    pub size: u64,
    pub hash: Digest,
    pub chunks: Vec<Digest>,
}

impl FileDigest {
    pub fn hash_hex(&self) -> String {
        hex::encode(self.hash)
    }

    pub fn chunks_hex(&self) -> Vec<String> {
        self.chunks.iter().map(hex::encode).collect()
    }
}

/// Hashes a file chunk by chunk. `progress` receives the bytes read since the
/// previous call; returning `false` from `keep_going` aborts with
/// `ErrorKind::Interrupted`.
pub fn hash_file(
    path: &Path,
    chunk_size: u64,
    mut progress: impl FnMut(u64),
    keep_going: impl Fn() -> bool,
) -> io::Result<FileDigest> {
    let mut file = File::open(path)?;
    let size = file.metadata()?.len();
    let mut buf = vec![0u8; chunk_size.min(size.max(1)) as usize];
    let mut chunks = Vec::with_capacity(chunk_count(size, chunk_size) as usize);
    let mut remaining = size;
    while remaining > 0 {
        if !keep_going() {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "hashing cancelled"));
        }
        let len = remaining.min(chunk_size) as usize;
        file.read_exact(&mut buf[..len])?;
        chunks.push(hash_chunk(&buf[..len]));
        remaining -= len as u64;
        progress(len as u64);
    }
    Ok(FileDigest { size, hash: file_hash(size, &chunks), chunks })
}

pub fn parse_digest(hex_str: &str) -> Option<Digest> {
    let mut out = [0u8; 32];
    hex::decode_to_slice(hex_str, &mut out).ok()?;
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn chunked_hash_is_stable_and_detects_changes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f.bin");
        let data: Vec<u8> = (0..10_000u32).flat_map(|i| i.to_le_bytes()).collect();
        File::create(&path).unwrap().write_all(&data).unwrap();

        let a = hash_file(&path, 4096, |_| {}, || true).unwrap();
        assert_eq!(a.size, 40_000);
        assert_eq!(a.chunks.len(), 10);
        let b = hash_file(&path, 4096, |_| {}, || true).unwrap();
        assert_eq!(a, b);

        let mut changed = data.clone();
        changed[5000] ^= 1;
        File::create(&path).unwrap().write_all(&changed).unwrap();
        let c = hash_file(&path, 4096, |_| {}, || true).unwrap();
        assert_ne!(a.hash, c.hash);
        let differing: Vec<_> = (0..10).filter(|&i| a.chunks[i] != c.chunks[i]).collect();
        assert_eq!(differing, vec![1]);
    }

    #[test]
    fn empty_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty");
        File::create(&path).unwrap();
        let d = hash_file(&path, 4096, |_| {}, || true).unwrap();
        assert!(d.chunks.is_empty());
        assert_eq!(d.hash, file_hash(0, &[]));
    }

    #[test]
    fn ranges() {
        assert_eq!(chunk_count(10, 4), 3);
        assert_eq!(chunk_range(2, 10, 4), (8, 10));
        assert_eq!(chunk_count(0, 4), 0);
    }
}
