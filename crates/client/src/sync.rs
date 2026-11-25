//! Brings the local game directory in line with the manifest and the
//! selection.
//!
//! Every file is handled chunk by chunk: a local copy (complete, partial or
//! stale) is read, each chunk's hash compared with the manifest, and only the
//! chunks that differ are downloaded (HTTP Range) and written in place. A
//! new file is downloaded to `<name>.oedvi-part` and renamed once complete,
//! so an interrupted download resumes where it stopped.

use std::collections::{BTreeMap, HashMap};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, anyhow, bail};
use futures_util::{StreamExt, TryStreamExt, stream};
use oedvi_common::hash::{self, Digest};
use oedvi_common::paths::{join_safe, to_manifest_path};
use oedvi_common::{FileEntry, Manifest};
use serde::Serialize;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

use crate::api::Api;
use crate::selection::Selection;
use crate::state::{LocalEntry, LocalIndex, StateFiles, mtime_ns};

pub const PART_SUFFIX: &str = ".oedvi-part";
const RETRIES: u32 = 4;
const INDEX_SAVE_EVERY: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FileState {
    /// Present and known to match the manifest.
    Ok,
    /// Selected, absent.
    Missing,
    /// Selected, download started.
    Partial,
    /// Selected, present, known to differ from the manifest.
    Outdated,
    /// Selected, present, never checked against this hash (read on sync).
    Unverified,
    /// Not selected and absent.
    Unselected,
    /// Not selected but present: removed on sync.
    ToRemove,
    /// Present but not in the manifest.
    Extra,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileStatus {
    pub path: String,
    pub size: u64,
    pub local_size: Option<u64>,
    pub group: String,
    pub required: bool,
    pub selected: bool,
    pub state: FileState,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Summary {
    pub manifest_version: u64,
    pub synced_version: Option<u64>,
    pub files_total: u64,
    pub bytes_total: u64,
    pub selected_files: u64,
    pub selected_bytes: u64,
    pub ok_files: u64,
    pub ok_bytes: u64,
    pub missing: u64,
    pub partial: u64,
    pub outdated: u64,
    pub unverified: u64,
    pub to_remove: u64,
    pub extra: u64,
    /// Upper bound of what a sync downloads.
    pub download_max_bytes: u64,
    pub free_space: Option<u64>,
    /// Nothing to download or remove.
    pub up_to_date: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub summary: Summary,
    pub files: Vec<FileStatus>,
}

struct LocalFile {
    size: u64,
    mtime_ns: u64,
}

/// Local files under `game_dir`: final files and `.oedvi-part` files.
fn scan_local(game_dir: &Path) -> anyhow::Result<(HashMap<String, LocalFile>, HashMap<String, LocalFile>)> {
    let mut files = HashMap::new();
    let mut parts = HashMap::new();
    if !game_dir.exists() {
        return Ok((files, parts));
    }
    for entry in walkdir::WalkDir::new(game_dir) {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }
        let Some(rel) = to_manifest_path(game_dir, entry.path()) else { continue };
        let meta = entry.metadata()?;
        let lf = LocalFile { size: meta.len(), mtime_ns: mtime_ns(&meta) };
        match rel.strip_suffix(PART_SUFFIX) {
            Some(base) => parts.insert(base.to_string(), lf),
            None => files.insert(rel, lf),
        };
    }
    Ok((files, parts))
}

/// Compares the local directory with the manifest without reading files.
pub fn local_status(
    manifest: &Manifest,
    selection: &Selection,
    index: &LocalIndex,
    game_dir: &Path,
    delete_extra: bool,
) -> anyhow::Result<Status> {
    let (local, parts) = scan_local(game_dir)?;
    let mut summary = Summary {
        manifest_version: manifest.version,
        synced_version: index.synced_version,
        files_total: manifest.files.len() as u64,
        bytes_total: manifest.total_size,
        free_space: fs4::available_space(game_dir).ok(),
        ..Default::default()
    };
    let mut files = Vec::with_capacity(manifest.files.len());
    let mut in_manifest = std::collections::HashSet::new();
    for f in &manifest.files {
        in_manifest.insert(f.path.as_str());
        let selected = selection.is_selected(f);
        let here = local.get(&f.path);
        let state = match (selected, here) {
            (true, Some(l)) => match index.files.get(&f.path) {
                Some(e) if e.size == l.size && e.mtime_ns == l.mtime_ns => {
                    if e.hash == f.hash {
                        FileState::Ok
                    } else {
                        FileState::Outdated
                    }
                }
                _ if l.size != f.size => FileState::Outdated,
                _ => FileState::Unverified,
            },
            (true, None) if parts.contains_key(&f.path) => FileState::Partial,
            (true, None) => FileState::Missing,
            (false, Some(_)) => FileState::ToRemove,
            (false, None) => FileState::Unselected,
        };
        if selected {
            summary.selected_files += 1;
            summary.selected_bytes += f.size;
        }
        match state {
            FileState::Ok => {
                summary.ok_files += 1;
                summary.ok_bytes += f.size;
            }
            FileState::Missing => {
                summary.missing += 1;
                summary.download_max_bytes += f.size;
            }
            FileState::Partial => {
                summary.partial += 1;
                summary.download_max_bytes += f.size;
            }
            FileState::Outdated => {
                summary.outdated += 1;
                summary.download_max_bytes += f.size;
            }
            FileState::Unverified => {
                summary.unverified += 1;
                summary.download_max_bytes += f.size;
            }
            FileState::ToRemove => summary.to_remove += 1,
            FileState::Unselected | FileState::Extra => {}
        }
        files.push(FileStatus {
            path: f.path.clone(),
            size: f.size,
            local_size: here.map(|l| l.size),
            group: f.group.clone(),
            required: f.required,
            selected,
            state,
        });
    }
    let mut extras: Vec<_> = local.iter().filter(|(p, _)| !in_manifest.contains(p.as_str())).collect();
    extras.sort_by(|a, b| a.0.cmp(b.0));
    for (path, l) in extras {
        summary.extra += 1;
        files.push(FileStatus {
            path: path.clone(),
            size: l.size,
            local_size: Some(l.size),
            group: String::new(),
            required: false,
            selected: false,
            state: FileState::Extra,
        });
    }
    summary.up_to_date = summary.missing + summary.partial + summary.outdated + summary.unverified + summary.to_remove
        == 0
        && (!delete_extra || summary.extra == 0);
    Ok(Status { summary, files })
}

#[derive(Debug, Clone)]
pub struct SyncOptions {
    /// Read every selected file again, even those known to be correct.
    pub verify_all: bool,
    pub delete_extra: bool,
    pub jobs: usize,
    /// Refuse to start when the disk is too small.
    pub check_space: bool,
}

impl Default for SyncOptions {
    fn default() -> Self {
        Self { verify_all: false, delete_extra: true, jobs: 6, check_space: true }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SyncError {
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ProgressSnapshot {
    /// `scan`, `remove`, `sync`, `done`, `cancelled` or `failed`.
    pub phase: String,
    pub files_total: u64,
    pub files_done: u64,
    pub verify_bytes_total: u64,
    pub verified_bytes: u64,
    /// Grows as files are checked and their stale chunks found.
    pub download_bytes_needed: u64,
    pub downloaded_bytes: u64,
    pub removed: u64,
    pub current: Vec<String>,
    pub errors: Vec<SyncError>,
    pub elapsed_secs: f64,
}

/// Progress shared with the UI while a sync runs.
pub struct Progress {
    started: Instant,
    verified: AtomicU64,
    needed: AtomicU64,
    downloaded: AtomicU64,
    inner: Mutex<ProgressSnapshot>,
}

impl Default for Progress {
    fn default() -> Self {
        Self {
            started: Instant::now(),
            verified: AtomicU64::new(0),
            needed: AtomicU64::new(0),
            downloaded: AtomicU64::new(0),
            inner: Mutex::new(ProgressSnapshot { phase: "scan".into(), ..Default::default() }),
        }
    }
}

impl Progress {
    pub fn snapshot(&self) -> ProgressSnapshot {
        let mut s = self.inner.lock().unwrap().clone();
        s.verified_bytes = self.verified.load(Ordering::Relaxed);
        s.download_bytes_needed = self.needed.load(Ordering::Relaxed);
        s.downloaded_bytes = self.downloaded.load(Ordering::Relaxed);
        s.elapsed_secs = self.started.elapsed().as_secs_f64();
        s
    }

    fn update(&self, f: impl FnOnce(&mut ProgressSnapshot)) {
        f(&mut self.inner.lock().unwrap());
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SyncReport {
    pub files_synced: u64,
    pub bytes_downloaded: u64,
    pub removed: u64,
    pub errors: Vec<SyncError>,
    pub cancelled: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    /// Absent: download every chunk to the part file.
    Fresh,
    /// Part file exists: check it, download the rest.
    Resume,
    /// Final file exists: check it, fix it in place.
    InPlace,
}

struct Ctx {
    api: Api,
    game_dir: PathBuf,
    chunk_size: u64,
    jobs: usize,
    permits: Semaphore,
    progress: Arc<Progress>,
    cancel: CancellationToken,
}

#[allow(clippy::too_many_arguments)]
pub async fn sync(
    api: &Api,
    manifest: &Manifest,
    selection: &Selection,
    state: &StateFiles,
    game_dir: &Path,
    opts: &SyncOptions,
    progress: Arc<Progress>,
    cancel: CancellationToken,
) -> anyhow::Result<SyncReport> {
    std::fs::create_dir_all(game_dir)?;
    let index = state.index();
    let status = local_status(manifest, selection, &index, game_dir, opts.delete_extra)?;
    let by_path: HashMap<&str, &FileEntry> = manifest.files.iter().map(|f| (f.path.as_str(), f)).collect();

    let mut work: Vec<(&FileEntry, Mode)> = Vec::new();
    let mut remove: Vec<String> = Vec::new();
    for s in &status.files {
        let mode = match s.state {
            FileState::Missing => Some(Mode::Fresh),
            FileState::Partial => Some(Mode::Resume),
            FileState::Outdated | FileState::Unverified => Some(Mode::InPlace),
            FileState::Ok if opts.verify_all => Some(Mode::InPlace),
            FileState::ToRemove => {
                remove.push(s.path.clone());
                None
            }
            FileState::Extra if opts.delete_extra => {
                remove.push(s.path.clone());
                None
            }
            _ => None,
        };
        if let Some(m) = mode {
            work.push((by_path[s.path.as_str()], m));
        }
    }
    // Part files of files that are no longer wanted.
    let (_, parts) = scan_local(game_dir)?;
    for p in parts.keys() {
        let wanted = by_path.get(p.as_str()).is_some_and(|f| selection.is_selected(f));
        if !wanted {
            remove.push(format!("{p}{PART_SUFFIX}"));
        }
    }

    if opts.check_space {
        let need: u64 = work
            .iter()
            .map(|(f, m)| match m {
                Mode::Fresh => f.size,
                _ => {
                    let local = status.files.iter().find(|s| s.path == f.path).and_then(|s| s.local_size).unwrap_or(0);
                    f.size.saturating_sub(local)
                }
            })
            .sum();
        if let Ok(free) = fs4::available_space(game_dir)
            && need > free
        {
            bail!(
                "not enough space: {:.1} GB needed, {:.1} GB free on the game drive",
                need as f64 / 1e9,
                free as f64 / 1e9
            );
        }
    }

    let mut index = index;
    // Removals first: they free space.
    progress.update(|p| p.phase = "remove".into());
    let mut removed = 0;
    let mut errors = Vec::new();
    for rel in &remove {
        let Some(path) = join_safe(game_dir, rel) else { continue };
        match std::fs::remove_file(&path) {
            Ok(()) => {
                removed += 1;
                index.files.remove(rel);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => errors.push(SyncError { path: rel.clone(), message: format!("cannot delete: {e}") }),
        }
    }
    if removed > 0 {
        remove_empty_dirs(game_dir);
    }
    progress.update(|p| {
        p.removed = removed;
        p.phase = "sync".into();
        p.files_total = work.len() as u64;
        p.verify_bytes_total = work.iter().filter(|(_, m)| *m != Mode::Fresh).map(|(f, _)| f.size).sum();
    });
    for (f, m) in &work {
        if *m == Mode::Fresh {
            progress.needed.fetch_add(f.size, Ordering::Relaxed);
        }
    }

    let ctx = Arc::new(Ctx {
        api: api.clone(),
        game_dir: game_dir.to_path_buf(),
        chunk_size: manifest.chunk_size,
        jobs: opts.jobs.max(1),
        permits: Semaphore::new(opts.jobs.max(1)),
        progress: progress.clone(),
        cancel: cancel.clone(),
    });
    let index = Arc::new(Mutex::new(index));
    let last_save = Mutex::new(Instant::now());
    let synced = AtomicU64::new(0);
    let file_errors: Mutex<Vec<SyncError>> = Mutex::new(Vec::new());

    stream::iter(work)
        .for_each_concurrent(ctx.jobs, |(f, mode)| {
            let ctx = ctx.clone();
            let index = index.clone();
            let (file_errors, last_save, synced) = (&file_errors, &last_save, &synced);
            async move {
                if ctx.cancel.is_cancelled() {
                    return;
                }
                ctx.progress.update(|p| p.current.push(f.path.clone()));
                let result = sync_file(&ctx, f, mode).await;
                ctx.progress.update(|p| {
                    p.current.retain(|c| c != &f.path);
                    p.files_done += 1;
                });
                match result {
                    Ok(entry) => {
                        synced.fetch_add(1, Ordering::Relaxed);
                        let mut idx = index.lock().unwrap();
                        idx.files.insert(f.path.clone(), entry);
                        let mut last = last_save.lock().unwrap();
                        if last.elapsed() > INDEX_SAVE_EVERY {
                            *last = Instant::now();
                            let _ = state.save_index(&idx);
                        }
                    }
                    Err(e) if ctx.cancel.is_cancelled() => {
                        tracing::debug!("{}: {e:#}", f.path);
                    }
                    Err(e) => {
                        let err = SyncError { path: f.path.clone(), message: format!("{e:#}") };
                        ctx.progress.update(|p| p.errors.push(err.clone()));
                        file_errors.lock().unwrap().push(err);
                    }
                }
            }
        })
        .await;

    errors.extend(file_errors.into_inner().unwrap());
    let cancelled = cancel.is_cancelled();
    let mut index =
        Arc::try_unwrap(index).map(|m| m.into_inner().unwrap()).unwrap_or_else(|a| a.lock().unwrap().clone());
    // Entries of files that no longer exist are dropped.
    index.files.retain(|p, _| by_path.contains_key(p.as_str()));
    if errors.is_empty() && !cancelled {
        index.synced_version = Some(manifest.version);
    }
    state.save_index(&index)?;
    progress.update(|p| {
        p.phase = if cancelled {
            "cancelled"
        } else if errors.is_empty() {
            "done"
        } else {
            "failed"
        }
        .into()
    });
    Ok(SyncReport {
        files_synced: synced.into_inner(),
        bytes_downloaded: progress.downloaded.load(Ordering::Relaxed),
        removed,
        errors,
        cancelled,
    })
}

async fn sync_file(ctx: &Ctx, f: &FileEntry, mode: Mode) -> anyhow::Result<LocalEntry> {
    let final_path = join_safe(&ctx.game_dir, &f.path).ok_or_else(|| anyhow!("path refused: {:?}", f.path))?;
    let part_path = PathBuf::from(format!("{}{PART_SUFFIX}", final_path.display()));
    let target = if mode == Mode::InPlace { final_path.clone() } else { part_path.clone() };
    if let Some(dir) = target.parent() {
        std::fs::create_dir_all(dir)?;
    }

    let chunk_size = ctx.chunk_size;
    let expected: Vec<Digest> = f
        .chunks
        .iter()
        .map(|h| hash::parse_digest(h).ok_or_else(|| anyhow!("manifest invalide pour {}", f.path)))
        .collect::<anyhow::Result<_>>()?;
    if expected.len() as u64 != hash::chunk_count(f.size, chunk_size) {
        bail!("manifest invalide pour {} (nombre de chunks)", f.path);
    }

    // Open, size, and find the chunks to download.
    let needed: Vec<u64> = {
        let (target, size, expected, progress, cancel) =
            (target.clone(), f.size, expected.clone(), ctx.progress.clone(), ctx.cancel.clone());
        tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<u64>> {
            let mut file = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(mode == Mode::Fresh)
                .open(&target)
                .with_context(|| format!("ouverture de {}", target.display()))?;
            let had = file.metadata()?.len();
            if had != size {
                file.set_len(size)?;
            }
            let count = hash::chunk_count(size, chunk_size);
            if mode == Mode::Fresh {
                return Ok((0..count).collect());
            }
            let mut needed = Vec::new();
            let mut buf = vec![0u8; chunk_size.min(size.max(1)) as usize];
            for i in 0..count {
                if cancel.is_cancelled() {
                    bail!("cancelled");
                }
                let (start, end) = hash::chunk_range(i, size, chunk_size);
                let len = (end - start) as usize;
                if start >= had {
                    needed.push(i);
                } else {
                    file.seek(SeekFrom::Start(start))?;
                    file.read_exact(&mut buf[..len])?;
                    if hash::hash_chunk(&buf[..len]) != expected[i as usize] {
                        needed.push(i);
                    }
                }
                progress.verified.fetch_add(len as u64, Ordering::Relaxed);
            }
            if mode != Mode::Fresh {
                let bytes: u64 = needed
                    .iter()
                    .map(|&i| {
                        let (s, e) = hash::chunk_range(i, size, chunk_size);
                        e - s
                    })
                    .sum();
                progress.needed.fetch_add(bytes, Ordering::Relaxed);
            }
            Ok(needed)
        })
        .await??
    };

    if !needed.is_empty() {
        let file = Arc::new(OpenOptions::new().write(true).open(&target)?);
        stream::iter(needed)
            .map(|i| {
                let file = file.clone();
                let expected = expected[i as usize];
                async move { download_chunk(ctx, f, i, expected, file).await }
            })
            .buffer_unordered(ctx.jobs)
            .try_collect::<Vec<()>>()
            .await?;
        let file = file.clone();
        tokio::task::spawn_blocking(move || file.sync_all()).await??;
    }

    if target == part_path {
        std::fs::rename(&part_path, &final_path).with_context(|| format!("renommage de {}", part_path.display()))?;
    }
    let meta = std::fs::metadata(&final_path)?;
    Ok(LocalEntry { size: meta.len(), mtime_ns: mtime_ns(&meta), hash: f.hash.clone() })
}

async fn download_chunk(ctx: &Ctx, f: &FileEntry, index: u64, expected: Digest, file: Arc<File>) -> anyhow::Result<()> {
    let (start, end) = hash::chunk_range(index, f.size, ctx.chunk_size);
    let mut last_err = None;
    for attempt in 0..RETRIES {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_secs(1 << attempt)).await;
        }
        let _permit = ctx.permits.acquire().await?;
        let data = tokio::select! {
            _ = ctx.cancel.cancelled() => bail!("cancelled"),
            r = ctx.api.fetch_range(&f.path, start, end) => r,
        };
        let data = match data {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!("{} chunk {index}: {e:#} (essai {})", f.path, attempt + 1);
                last_err = Some(e);
                continue;
            }
        };
        let got = tokio::task::spawn_blocking(move || (hash::hash_chunk(&data), data)).await?;
        if got.0 != expected {
            last_err = Some(anyhow!("the received content does not match the manifest (file changed on the server?)"));
            continue;
        }
        let data = got.1;
        let file = file.clone();
        tokio::task::spawn_blocking(move || write_at(&file, &data, start)).await??;
        ctx.progress.downloaded.fetch_add(end - start, Ordering::Relaxed);
        return Ok(());
    }
    Err(last_err.unwrap_or_else(|| anyhow!("failed")).context(format!("chunk {index}")))
}

fn write_at(file: &File, buf: &[u8], offset: u64) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileExt;
        file.write_all_at(buf, offset)
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::FileExt;
        let mut done = 0;
        while done < buf.len() {
            let n = file.seek_write(&buf[done..], offset + done as u64)?;
            if n == 0 {
                return Err(std::io::ErrorKind::WriteZero.into());
            }
            done += n;
        }
        Ok(())
    }
}

fn remove_empty_dirs(root: &Path) {
    for entry in walkdir::WalkDir::new(root).min_depth(1).contents_first(true).into_iter().flatten() {
        if entry.file_type().is_dir() {
            let _ = std::fs::remove_dir(entry.path()); // fails unless empty
        }
    }
}

/// Group totals for the UI.
pub fn group_sizes(manifest: &Manifest, selection: &Selection) -> BTreeMap<String, (u64, u64)> {
    let mut out: BTreeMap<String, (u64, u64)> = BTreeMap::new();
    for f in &manifest.files {
        let e = out.entry(f.group.clone()).or_default();
        if selection.is_selected(f) {
            e.0 += f.size;
        }
        e.1 += f.size;
    }
    out
}
