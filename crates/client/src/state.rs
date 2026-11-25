//! Local state kept in `<root>/state/`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::selection::Selection;
use crate::settings::Layout;

/// What is known about local game files: a file whose size and date still
/// match its entry has the recorded hash, without reading it again.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct LocalIndex {
    pub files: BTreeMap<String, LocalEntry>,
    /// Manifest version the last complete sync reached.
    #[serde(default)]
    pub synced_version: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LocalEntry {
    pub size: u64,
    pub mtime_ns: u64,
    pub hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildRecord {
    pub commit: String,
    pub ok: bool,
    pub finished_at: u64,
    pub secs: f64,
}

/// Build results per checkout and build recipe.
pub type BuildRecords = BTreeMap<String, BTreeMap<String, BuildRecord>>;

pub struct StateFiles {
    dir: PathBuf,
}

impl StateFiles {
    pub fn new(layout: &Layout) -> Self {
        Self { dir: layout.state.clone() }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    pub fn index(&self) -> LocalIndex {
        read_json(&self.path("index.json")).unwrap_or_default()
    }
    pub fn save_index(&self, v: &LocalIndex) -> anyhow::Result<()> {
        write_json(&self.path("index.json"), v)
    }

    pub fn selection(&self) -> Selection {
        read_json(&self.path("selection.json")).unwrap_or_default()
    }
    pub fn save_selection(&self, v: &Selection) -> anyhow::Result<()> {
        write_json(&self.path("selection.json"), v)
    }

    pub fn builds(&self) -> BuildRecords {
        read_json(&self.path("builds.json")).unwrap_or_default()
    }
    pub fn save_builds(&self, v: &BuildRecords) -> anyhow::Result<()> {
        write_json(&self.path("builds.json"), v)
    }

    /// Last manifest seen, for offline use.
    pub fn manifest(&self) -> Option<oedvi_common::Manifest> {
        read_json(&self.path("manifest.json"))
    }
    pub fn save_manifest(&self, v: &oedvi_common::Manifest) -> anyhow::Result<()> {
        write_json(&self.path("manifest.json"), v)
    }

    /// Runs recorded on this machine, oldest first (`sessions.jsonl`).
    pub fn sessions(&self) -> Vec<oedvi_common::stats::Session> {
        let Ok(text) = std::fs::read_to_string(self.path("sessions.jsonl")) else { return Vec::new() };
        text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect()
    }
    pub fn add_session(&self, s: &oedvi_common::stats::Session) -> anyhow::Result<()> {
        use std::io::Write;
        std::fs::create_dir_all(&self.dir)?;
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(self.path("sessions.jsonl"))?;
        writeln!(f, "{}", serde_json::to_string(s)?)?;
        Ok(())
    }

    pub fn builders(&self) -> Option<oedvi_common::BuilderList> {
        read_json(&self.path("builders.json"))
    }
    pub fn save_builders(&self, v: &oedvi_common::BuilderList) -> anyhow::Result<()> {
        write_json(&self.path("builders.json"), v)
    }
}

pub fn read_json<T: DeserializeOwned>(path: &Path) -> Option<T> {
    let bytes = std::fs::read(path).ok()?;
    match serde_json::from_slice(&bytes) {
        Ok(v) => Some(v),
        Err(e) => {
            tracing::warn!("{} unreadable: {e}", path.display());
            None
        }
    }
}

pub fn write_json<T: Serialize>(path: &Path, value: &T) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(value)?).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))?;
    Ok(())
}

pub fn mtime_ns(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}
