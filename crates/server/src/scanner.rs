//! Scans the game directory and keeps the manifest up to date.
//!
//! Hashing 250 GB takes a long while, so the result of every file is kept in
//! `index.json` with the file's size and modification time. A rescan only
//! hashes files whose size or date changed; the index is saved during long
//! scans so an interrupted scan resumes where it stopped. The previous
//! manifest is served while a scan runs.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::Context;
use bytes::Bytes;
use oedvi_common::hash::{self, FileDigest};
use oedvi_common::paths::{is_safe_relative, to_manifest_path};
use oedvi_common::{FileEntry, GroupInfo, Manifest, ScanStatus};
use serde::{Deserialize, Serialize};
use tokio::sync::Notify;

use crate::config::Config;
use crate::telemetry;

const INDEX_SAVE_EVERY: Duration = Duration::from_secs(30);
const PROGRESS_EVERY: Duration = Duration::from_secs(10);
/// Files this big get their own line when hashed.
const BIG_FILE: u64 = 1 << 30;

#[derive(Debug, Default, Serialize, Deserialize)]
struct Index {
    chunk_size: u64,
    version: u64,
    /// Hash of the published content, to tell whether a scan changed it.
    digest: String,
    files: BTreeMap<String, IndexEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct IndexEntry {
    size: u64,
    mtime_ns: u64,
    hash: String,
    chunks: Vec<String>,
}

/// The published manifest: parsed for the API, serialized once for clients.
pub struct Published {
    pub manifest: Manifest,
    pub json: Bytes,
}

pub struct Scanner {
    cfg: Arc<Config>,
    status: Mutex<ScanStatus>,
    published: RwLock<Option<Arc<Published>>>,
    wake: Notify,
    bytes_hashed: AtomicU64,
    shutdown: AtomicBool,
}

struct Found {
    rel: String,
    abs: PathBuf,
    size: u64,
    mtime_ns: u64,
}

impl Scanner {
    pub fn new(cfg: Arc<Config>) -> Arc<Self> {
        let scanner = Arc::new(Self {
            cfg,
            status: Mutex::new(ScanStatus::default()),
            published: RwLock::new(None),
            wake: Notify::new(),
            bytes_hashed: AtomicU64::new(0),
            shutdown: AtomicBool::new(false),
        });
        match scanner.load_previous() {
            Ok(Some(p)) => {
                tracing::info!(
                    version = p.manifest.version,
                    files = p.manifest.files.len(),
                    "serving the previous manifest while scanning"
                );
                scanner.status.lock().unwrap().manifest_version = Some(p.manifest.version);
                *scanner.published.write().unwrap() = Some(Arc::new(p));
            }
            Ok(None) => {}
            Err(e) => tracing::warn!("previous manifest unreadable, waiting for the scan: {e:#}"),
        }
        scanner
    }

    pub fn published(&self) -> Option<Arc<Published>> {
        self.published.read().unwrap().clone()
    }

    pub fn status(&self) -> ScanStatus {
        let mut s = self.status.lock().unwrap().clone();
        s.bytes_hashed = self.bytes_hashed.load(Ordering::Relaxed);
        s
    }

    pub fn request_rescan(&self) {
        self.wake.notify_one();
    }

    pub fn stop(&self) {
        self.shutdown.store(true, Ordering::Relaxed);
        self.wake.notify_one();
    }

    /// Runs scans for as long as the server lives: one at start, then one
    /// per request (requests made during a scan are merged into one).
    pub async fn run(self: Arc<Self>) {
        self.wake.notify_one();
        loop {
            self.wake.notified().await;
            if self.shutdown.load(Ordering::Relaxed) {
                return;
            }
            let this = self.clone();
            let started = Instant::now();
            let result = tokio::task::spawn_blocking(move || this.scan_once()).await;
            let mut status = self.status.lock().unwrap();
            status.scanning = false;
            status.current = None;
            status.last_scan_at = Some(now_secs());
            status.last_scan_secs = Some(started.elapsed().as_secs_f64());
            match result {
                Ok(Ok(version)) => {
                    status.manifest_version = Some(version);
                    status.last_error = None;
                }
                Ok(Err(e)) => {
                    tracing::error!("scan failed: {e:#}");
                    status.last_error = Some(format!("{e:#}"));
                }
                Err(e) => {
                    tracing::error!("scan panicked: {e}");
                    status.last_error = Some(format!("scan panicked: {e}"));
                }
            }
        }
    }

    fn index_path(&self) -> PathBuf {
        self.cfg.data_dir.join("index.json")
    }

    fn manifest_path(&self) -> PathBuf {
        self.cfg.data_dir.join("manifest.json")
    }

    fn load_previous(&self) -> anyhow::Result<Option<Published>> {
        let path = self.manifest_path();
        if !path.exists() {
            return Ok(None);
        }
        let json = std::fs::read(&path)?;
        let manifest: Manifest = serde_json::from_slice(&json)?;
        Ok(Some(Published { manifest, json: json.into() }))
    }

    fn load_index(&self) -> Index {
        let path = self.index_path();
        let index = std::fs::read(&path)
            .ok()
            .and_then(|b| {
                serde_json::from_slice::<Index>(&b)
                    .map_err(|e| tracing::warn!("index unreadable, rehashing everything: {e}"))
                    .ok()
            })
            .unwrap_or_default();
        if index.chunk_size != 0 && index.chunk_size != self.cfg.chunk_size() {
            tracing::warn!("chunk size changed, rehashing everything");
            return Index { version: index.version, digest: index.digest, ..Default::default() };
        }
        index
    }

    fn save_index(&self, index: &Index) -> anyhow::Result<()> {
        write_atomic(&self.index_path(), &serde_json::to_vec(index)?)
    }

    fn scan_once(&self) -> anyhow::Result<u64> {
        let started = Instant::now();
        {
            let mut s = self.status.lock().unwrap();
            *s = ScanStatus { scanning: true, ..s.clone() };
            s.last_error = None;
        }
        self.bytes_hashed.store(0, Ordering::Relaxed);

        tracing::info!("scan: listing {}", self.cfg.game_dir.display());
        let found = self.walk()?;
        let mut index = self.load_index();
        index.chunk_size = self.cfg.chunk_size();
        let present: std::collections::HashSet<&str> = found.iter().map(|f| f.rel.as_str()).collect();

        let to_hash: Vec<&Found> = found
            .iter()
            .filter(|f| match index.files.get(&f.rel) {
                Some(e) => e.size != f.size || e.mtime_ns != f.mtime_ns,
                None => true,
            })
            .collect();
        {
            let mut s = self.status.lock().unwrap();
            s.files_total = found.len() as u64;
            s.files_done = (found.len() - to_hash.len()) as u64;
            s.bytes_total = found.iter().map(|f| f.size).sum();
            s.bytes_to_hash = to_hash.iter().map(|f| f.size).sum();
        }
        let new_files = to_hash.iter().filter(|f| !index.files.contains_key(&f.rel)).count();
        let removed: Vec<&String> = index.files.keys().filter(|k| !present.contains(k.as_str())).collect();
        let (bytes_total, bytes_to_hash) = {
            let s = self.status.lock().unwrap();
            (s.bytes_total, s.bytes_to_hash)
        };
        tracing::info!(
            "scan started: {} files ({}); {} unchanged, {} new, {} modified, {} removed; {} to hash",
            found.len(),
            telemetry::bytes(bytes_total),
            found.len() - to_hash.len(),
            new_files,
            to_hash.len() - new_files,
            removed.len(),
            telemetry::bytes(bytes_to_hash),
        );
        // A handful of changes is worth naming; a first scan is not.
        let name_them = to_hash.len() + removed.len() <= 10;
        for f in &to_hash {
            let mark = if index.files.contains_key(&f.rel) { "~" } else { "+" };
            if name_them {
                tracing::info!("  {mark} {} ({})", f.rel, telemetry::bytes(f.size));
            } else {
                tracing::debug!("  {mark} {} ({})", f.rel, telemetry::bytes(f.size));
            }
        }
        for rel in &removed {
            if name_them {
                tracing::info!("  - {rel}");
            } else {
                tracing::debug!("  - {rel}");
            }
        }

        // Hash with `hash_jobs` workers pulling from a shared queue.
        let queue = Mutex::new(to_hash.into_iter());
        let index = Mutex::new(index);
        let last_save = Mutex::new(Instant::now());
        let unstable = AtomicBool::new(false);
        let failure: Mutex<Option<anyhow::Error>> = Mutex::new(None);
        let finished = AtomicBool::new(false);
        std::thread::scope(|scope| {
            let workers: Vec<_> = (0..self.cfg.hash_jobs)
                .map(|_| {
                    scope.spawn(|| {
                        loop {
                            if self.shutdown.load(Ordering::Relaxed) || failure.lock().unwrap().is_some() {
                                return;
                            }
                            let Some(f) = queue.lock().unwrap().next() else { return };
                            self.status.lock().unwrap().current = Some(f.rel.clone());
                            match self.hash_one(f) {
                                Ok(Some(entry)) => {
                                    index.lock().unwrap().files.insert(f.rel.clone(), entry);
                                }
                                Ok(None) => {
                                    unstable.store(true, Ordering::Relaxed);
                                }
                                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                                    unstable.store(true, Ordering::Relaxed);
                                }
                                Err(e) => {
                                    *failure.lock().unwrap() =
                                        Some(anyhow::Error::new(e).context(format!("hashing {}", f.rel)));
                                    return;
                                }
                            }
                            self.status.lock().unwrap().files_done += 1;
                            let mut last = last_save.lock().unwrap();
                            if last.elapsed() > INDEX_SAVE_EVERY {
                                *last = Instant::now();
                                let idx = index.lock().unwrap();
                                if let Err(e) = self.save_index(&idx) {
                                    tracing::warn!("saving the index: {e:#}");
                                }
                            }
                        }
                    })
                })
                .collect();
            if bytes_to_hash > 0 {
                scope.spawn(|| self.report_progress(&finished, bytes_to_hash));
            }
            let mut panicked = None;
            for w in workers {
                if let Err(p) = w.join() {
                    panicked.get_or_insert(p);
                }
            }
            finished.store(true, Ordering::Relaxed);
            if let Some(p) = panicked {
                std::panic::resume_unwind(p);
            }
        });
        let mut index = index.into_inner().unwrap();
        if let Some(e) = failure.into_inner().unwrap() {
            self.save_index(&index).ok();
            return Err(e);
        }
        if self.shutdown.load(Ordering::Relaxed) {
            self.save_index(&index).ok();
            anyhow::bail!("stopped");
        }

        // Files that changed while hashed keep their previous entry, if any,
        // and are rescanned shortly. Files that are gone leave the index.
        index.files.retain(|k, _| present.contains(k.as_str()));

        let mut manifest = self.build_manifest(&index)?;
        let digest = content_digest(&manifest);
        let changed = digest != index.digest;
        if changed {
            index.version += 1;
            index.digest = digest;
        }
        manifest.version = index.version;
        let json = serde_json::to_vec(&manifest)?;
        write_atomic(&self.manifest_path(), &json)?;
        self.save_index(&index)?;
        tracing::info!(
            "manifest published: version {} ({}), {} files, {}, scan took {}",
            manifest.version,
            if changed { "content changed, clients will update" } else { "content unchanged" },
            manifest.files.len(),
            telemetry::bytes(manifest.total_size),
            telemetry::duration(started.elapsed().as_secs()),
        );
        let groups: Vec<String> = manifest
            .groups
            .iter()
            .map(|g| {
                format!(
                    "{}{} {} files {}",
                    g.name,
                    if g.required { "*" } else { "" },
                    g.file_count,
                    telemetry::bytes(g.size)
                )
            })
            .collect();
        tracing::info!("groups: {}", groups.join(" | "));
        *self.published.write().unwrap() = Some(Arc::new(Published { manifest, json: json.into() }));

        if unstable.into_inner() {
            tracing::info!("some files changed during the scan; rescanning");
            self.wake.notify_one();
        }
        Ok(index.version)
    }

    /// Prints how far the hashing is every few seconds, until `finished`.
    fn report_progress(&self, finished: &AtomicBool, bytes_total: u64) {
        let started = Instant::now();
        let mut last = (Instant::now(), 0u64);
        while !finished.load(Ordering::Relaxed) && !self.shutdown.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(500));
            if last.0.elapsed() < PROGRESS_EVERY {
                continue;
            }
            let done = self.bytes_hashed.load(Ordering::Relaxed).min(bytes_total);
            let recent = (done - last.1) as f64 / last.0.elapsed().as_secs_f64();
            let average = done as f64 / started.elapsed().as_secs_f64();
            let eta = if average > 0.0 {
                telemetry::duration(((bytes_total - done) as f64 / average) as u64)
            } else {
                "?".into()
            };
            let (files_done, files_total, current) = {
                let st = self.status.lock().unwrap();
                (st.files_done, st.files_total, st.current.clone().unwrap_or_default())
            };
            tracing::info!(
                "hashing {:.0}%: {} / {} at {}/s, files {}/{}, about {} left, now {}",
                done as f64 * 100.0 / bytes_total as f64,
                telemetry::bytes(done),
                telemetry::bytes(bytes_total),
                telemetry::bytes(recent as u64),
                files_done,
                files_total,
                eta,
                current,
            );
            last = (Instant::now(), done);
        }
    }

    /// Lists the files to publish.
    fn walk(&self) -> anyhow::Result<Vec<Found>> {
        let root = &self.cfg.game_dir;
        let ignore = self.cfg.ignore_set()?;
        let mut out = Vec::new();
        for entry in walkdir::WalkDir::new(root).follow_links(true) {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    tracing::warn!("skipping: {e}");
                    continue;
                }
            };
            if !entry.file_type().is_file() {
                continue;
            }
            let Some(rel) = to_manifest_path(root, entry.path()) else {
                tracing::warn!("skipping {} (name is not UTF-8)", entry.path().display());
                continue;
            };
            if ignore.is_match(&rel) {
                continue;
            }
            if !is_safe_relative(&rel) {
                tracing::warn!("skipping {rel:?} (name not usable on Windows)");
                continue;
            }
            let meta = entry.metadata().with_context(|| format!("reading {}", entry.path().display()))?;
            out.push(Found { rel, abs: entry.path().to_path_buf(), size: meta.len(), mtime_ns: mtime_ns(&meta) });
        }
        out.sort_by(|a, b| a.rel.cmp(&b.rel));
        Ok(out)
    }

    /// `Ok(None)`: the file changed while it was read.
    fn hash_one(&self, f: &Found) -> std::io::Result<Option<IndexEntry>> {
        let started = Instant::now();
        let digest: FileDigest = hash::hash_file(
            &f.abs,
            self.cfg.chunk_size(),
            |n| {
                self.bytes_hashed.fetch_add(n, Ordering::Relaxed);
            },
            || !self.shutdown.load(Ordering::Relaxed),
        )?;
        let meta = std::fs::metadata(&f.abs)?;
        if meta.len() != digest.size || mtime_ns(&meta) != f.mtime_ns || digest.size != f.size {
            tracing::info!("{} changed while hashed", f.rel);
            return Ok(None);
        }
        let secs = started.elapsed().as_secs_f64().max(0.001);
        let line = format!(
            "hashed {} ({}, {}/s)",
            f.rel,
            telemetry::bytes(f.size),
            telemetry::bytes((f.size as f64 / secs) as u64)
        );
        if f.size >= BIG_FILE {
            tracing::info!("{line}");
        } else {
            tracing::debug!("{line}");
        }
        Ok(Some(IndexEntry {
            size: f.size,
            mtime_ns: f.mtime_ns,
            hash: digest.hash_hex(),
            chunks: digest.chunks_hex(),
        }))
    }

    fn build_manifest(&self, index: &Index) -> anyhow::Result<Manifest> {
        let rules = self.cfg.group_matchers()?;
        let mut groups: BTreeMap<String, GroupInfo> = BTreeMap::new();
        for (rule, _) in &rules {
            groups.entry(rule.name.clone()).or_insert(GroupInfo {
                name: rule.name.clone(),
                label: rule.label.clone(),
                required: rule.required,
                file_count: 0,
                size: 0,
            });
        }
        let mut files = Vec::with_capacity(index.files.len());
        for (path, e) in &index.files {
            let (group, required) = rules
                .iter()
                .find(|(_, set)| set.is_match(path))
                .map(|(r, _)| (r.name.clone(), r.required))
                .unwrap_or_else(|| (self.cfg.default_group.clone(), false));
            let g = groups.entry(group.clone()).or_insert(GroupInfo {
                name: group.clone(),
                label: None,
                required: false,
                file_count: 0,
                size: 0,
            });
            g.file_count += 1;
            g.size += e.size;
            files.push(FileEntry {
                path: path.clone(),
                size: e.size,
                hash: e.hash.clone(),
                chunks: e.chunks.clone(),
                group,
                required,
            });
        }
        // Configured order first, then the default group.
        let mut ordered: Vec<GroupInfo> = Vec::new();
        for (rule, _) in &rules {
            if let Some(g) = groups.remove(&rule.name) {
                ordered.push(g);
            }
        }
        ordered.extend(groups.into_values());
        Ok(Manifest {
            version: index.version,
            generated_at: now_secs(),
            chunk_size: index.chunk_size,
            total_size: files.iter().map(|f| f.size).sum(),
            groups: ordered,
            files,
        })
    }
}

fn content_digest(m: &Manifest) -> String {
    let mut h = blake3::Hasher::new();
    h.update(&m.chunk_size.to_le_bytes());
    for f in &m.files {
        for part in [f.path.as_str(), f.hash.as_str(), f.group.as_str(), if f.required { "r" } else { "-" }] {
            h.update(part.as_bytes());
            h.update(&[0]);
        }
    }
    for g in &m.groups {
        h.update(g.name.as_bytes());
        h.update(g.label.as_deref().unwrap_or("").as_bytes());
        h.update(&[g.required as u8, 0]);
    }
    h.finalize().to_hex().to_string()
}

fn mtime_ns(meta: &std::fs::Metadata) -> u64 {
    meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_nanos() as u64).unwrap_or(0)
}

pub fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

pub fn write_atomic(path: &Path, data: &[u8]) -> anyhow::Result<()> {
    let tmp = path.with_extension("tmp-write");
    std::fs::write(&tmp, data).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))?;
    Ok(())
}
