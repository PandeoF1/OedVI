//! Backups of the game's saves (the `{data_dir}` folder the builders give to
//! the game). Each backup is a folder `<root>/backups/<id>/` holding a copy of
//! the files and a `backup.json` listing them with their BLAKE3 hash, so a
//! backup can be verified before it is restored.

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::state::{now_secs, write_json};

const META: &str = "backup.json";
const FILES: &str = "files";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BackupFile {
    /// Path relative to the data folder, with `/` separators.
    pub path: String,
    pub size: u64,
    pub hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Backup {
    pub id: String,
    pub created_at: u64,
    /// Name given by the player (or the reason, for an automatic backup).
    pub label: String,
    /// Made by the launcher (before a game start or a restore), not by hand.
    pub auto: bool,
    pub files: Vec<BackupFile>,
    pub bytes: u64,
}

fn hash_file(path: &Path) -> anyhow::Result<String> {
    let mut h = blake3::Hasher::new();
    let f = std::fs::File::open(path).with_context(|| format!("reading {}", path.display()))?;
    h.update_reader(f)?;
    Ok(h.finalize().to_hex().to_string())
}

fn rel(root: &Path, p: &Path) -> String {
    p.strip_prefix(root).unwrap_or(p).components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/")
}

/// An id is a folder name made by us: refuse anything that could leave
/// the backups folder.
fn check_id(id: &str) -> anyhow::Result<()> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        bail!("invalid backup id {id:?}");
    }
    Ok(())
}

fn read_meta(dir: &Path) -> anyhow::Result<Backup> {
    let p = dir.join(META);
    serde_json::from_slice(&std::fs::read(&p).with_context(|| format!("reading {}", p.display()))?)
        .with_context(|| format!("parsing {}", p.display()))
}

/// Copies `data` into a new backup. `None` when there is nothing to save.
pub fn create(data: &Path, backups: &Path, label: &str, auto: bool) -> anyhow::Result<Option<Backup>> {
    if !data.is_dir() {
        return Ok(None);
    }
    let mut files = Vec::new();
    for entry in WalkDir::new(data).follow_links(false) {
        let entry = entry?;
        if entry.file_type().is_file() {
            files.push(entry.into_path());
        }
    }
    if files.is_empty() {
        return Ok(None);
    }
    let created_at = now_secs();
    let mut id = format!("{created_at}");
    let mut n = 1;
    while backups.join(&id).exists() {
        id = format!("{created_at}-{n}");
        n += 1;
    }
    let dir = backups.join(&id);
    let tmp = backups.join(format!(".{id}.tmp"));
    let _ = std::fs::remove_dir_all(&tmp);
    let copy = || -> anyhow::Result<Vec<BackupFile>> {
        let mut out = Vec::new();
        for src in &files {
            let r = rel(data, src);
            let dst = tmp.join(FILES).join(&r);
            std::fs::create_dir_all(dst.parent().unwrap())?;
            std::fs::copy(src, &dst).with_context(|| format!("copying {}", src.display()))?;
            // Hash what was written, not what was read: that is what a restore gets.
            out.push(BackupFile { path: r, size: std::fs::metadata(&dst)?.len(), hash: hash_file(&dst)? });
        }
        Ok(out)
    };
    let list = match copy() {
        Ok(l) => l,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(e);
        }
    };
    let bytes = list.iter().map(|f| f.size).sum();
    let backup = Backup { id, created_at, label: label.trim().chars().take(80).collect(), auto, files: list, bytes };
    write_json(&tmp.join(META), &backup)?;
    std::fs::rename(&tmp, &dir)?;
    Ok(Some(backup))
}

/// Every backup, newest first. Folders that are not valid backups are skipped.
pub fn list(backups: &Path) -> Vec<Backup> {
    let mut v: Vec<Backup> = std::fs::read_dir(backups)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| read_meta(&e.path()).ok())
        .collect();
    v.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(b.id.cmp(&a.id)));
    v
}

/// Checks the copy against its list. Returns the problems found.
pub fn verify(backups: &Path, id: &str) -> anyhow::Result<Vec<String>> {
    check_id(id)?;
    let dir = backups.join(id);
    let meta = read_meta(&dir)?;
    let mut problems = Vec::new();
    for f in &meta.files {
        let p = dir.join(FILES).join(&f.path);
        match hash_file(&p) {
            Ok(h) if h == f.hash => {}
            Ok(_) => problems.push(format!("{}: content differs", f.path)),
            Err(_) => problems.push(format!("{}: missing", f.path)),
        }
    }
    Ok(problems)
}

/// Puts a backup back in `data`: the folder is first saved as an automatic
/// backup, then replaced. The backup is verified before anything is touched.
pub fn restore(data: &Path, backups: &Path, id: &str) -> anyhow::Result<()> {
    check_id(id)?;
    let problems = verify(backups, id)?;
    if !problems.is_empty() {
        bail!("this backup is damaged, nothing was changed ({})", problems.join("; "));
    }
    create(data, backups, "before restore", true).context("saving the current files before the restore")?;
    let meta = read_meta(&backups.join(id))?;
    let src = backups.join(id).join(FILES);
    if data.exists() {
        std::fs::remove_dir_all(data).with_context(|| format!("clearing {}", data.display()))?;
    }
    for f in &meta.files {
        let dst = data.join(&f.path);
        std::fs::create_dir_all(dst.parent().unwrap())?;
        std::fs::copy(src.join(&f.path), &dst).with_context(|| format!("restoring {}", f.path))?;
    }
    Ok(())
}

pub fn delete(backups: &Path, id: &str) -> anyhow::Result<()> {
    check_id(id)?;
    let dir = backups.join(id);
    if !dir.join(META).is_file() {
        bail!("no such backup");
    }
    std::fs::remove_dir_all(&dir).with_context(|| format!("deleting {}", dir.display()))
}

/// Keeps the `keep` newest automatic backups; those made by hand are never removed.
pub fn prune(backups: &Path, keep: usize) {
    for b in list(backups).into_iter().filter(|b| b.auto).skip(keep) {
        let _ = delete(backups, &b.id);
    }
}

pub fn dir_of(root: &Path) -> PathBuf {
    root.join("backups")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let t = tempfile::tempdir().unwrap();
        let data = t.path().join("data");
        std::fs::create_dir_all(data.join("7952987A/sub")).unwrap();
        std::fs::write(data.join("7952987A/save.bin"), b"one").unwrap();
        std::fs::write(data.join("7952987A/sub/x.bin"), b"two").unwrap();
        let b = t.path().join("backups");
        (t, data, b)
    }

    #[test]
    fn nothing_to_save_makes_no_backup() {
        let t = tempfile::tempdir().unwrap();
        assert!(create(&t.path().join("none"), &t.path().join("b"), "x", false).unwrap().is_none());
    }

    #[test]
    fn backup_then_restore_brings_the_files_back() {
        let (_t, data, b) = setup();
        let made = create(&data, &b, "  before the heist ", false).unwrap().unwrap();
        assert_eq!((made.files.len(), made.bytes, made.label.as_str()), (2, 6, "before the heist"));
        std::fs::write(data.join("7952987A/save.bin"), b"corrupted").unwrap();
        std::fs::remove_file(data.join("7952987A/sub/x.bin")).unwrap();
        std::fs::write(data.join("new.bin"), b"later").unwrap();
        restore(&data, &b, &made.id).unwrap();
        assert_eq!(std::fs::read(data.join("7952987A/save.bin")).unwrap(), b"one");
        assert_eq!(std::fs::read(data.join("7952987A/sub/x.bin")).unwrap(), b"two");
        assert!(!data.join("new.bin").exists());
        // The state before the restore is kept as an automatic backup.
        let all = list(&b);
        assert_eq!(all.len(), 2);
        assert!(all.iter().any(|x| x.auto && x.label == "before restore"));
    }

    #[test]
    fn a_damaged_backup_is_refused_and_changes_nothing() {
        let (_t, data, b) = setup();
        let made = create(&data, &b, "", false).unwrap().unwrap();
        std::fs::write(b.join(&made.id).join(FILES).join("7952987A/save.bin"), b"bad").unwrap();
        assert_eq!(verify(&b, &made.id).unwrap().len(), 1);
        assert!(restore(&data, &b, &made.id).is_err());
        assert_eq!(std::fs::read(data.join("7952987A/save.bin")).unwrap(), b"one");
        assert_eq!(list(&b).len(), 1);
    }

    #[test]
    fn ids_cannot_escape_the_backups_folder() {
        let (_t, _data, b) = setup();
        assert!(delete(&b, "../data").is_err());
        assert!(verify(&b, "").is_err());
    }

    #[test]
    fn prune_keeps_manual_backups_and_the_newest_automatic_ones() {
        let (_t, data, b) = setup();
        create(&data, &b, "mine", false).unwrap();
        for _ in 0..4 {
            create(&data, &b, "auto", true).unwrap();
        }
        prune(&b, 2);
        let all = list(&b);
        assert_eq!(all.iter().filter(|x| x.auto).count(), 2);
        assert_eq!(all.iter().filter(|x| !x.auto).count(), 1);
    }
}
