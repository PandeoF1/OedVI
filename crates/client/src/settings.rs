use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::{Deserialize, Serialize};

/// How the launcher gets the game.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Everything on this PC: builders are local files and the game is
    /// already in `game_dir`. Nothing is downloaded.
    #[default]
    Standalone,
    /// Connected to an OedVI server: it publishes the game files, the
    /// builders and the performance comparison.
    Remote,
}

/// Client settings, shared by the CLI and the launcher
/// (`<config dir>/oedvi/settings.json`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub mode: Mode,
    /// Only used in remote mode.
    pub server_url: String,
    pub token: String,
    /// Clones, caches, saves, logs and local state.
    pub root_dir: PathBuf,
    /// Game files. Defaults to `<root_dir>/game`; a big game is often on
    /// another drive.
    pub game_dir: Option<PathBuf>,
    /// Chunks downloaded at the same time.
    pub download_jobs: usize,
    /// Delete local files the server no longer publishes.
    pub delete_extra: bool,
    /// Install missing tools (git, Rust, Build Tools…) before a build, only
    /// those the builder needs.
    pub auto_install_tools: bool,
    /// Extra environment per builder id, applied when the game runs.
    pub builder_env: BTreeMap<String, BTreeMap<String, String>>,
    /// The branch, tag or commit chosen on this PC for a repository, by
    /// checkout name. It replaces the `ref` of every builder of that clone.
    pub ref_overrides: BTreeMap<String, String>,
    /// Name of this PC in the performance comparison.
    pub machine_name: String,
    /// Send the FPS of each run to the server, to compare with the others.
    pub share_stats: bool,
    /// Save the game's saves before each game start.
    pub backup_before_play: bool,
    /// Automatic backups kept (those made by hand are never removed).
    pub backup_keep: usize,
}

fn default_machine_name() -> String {
    ["COMPUTERNAME", "HOSTNAME"]
        .iter()
        .find_map(|k| std::env::var(k).ok())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "pc".into())
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            mode: Mode::Standalone,
            server_url: String::new(),
            token: String::new(),
            root_dir: dirs::data_local_dir().unwrap_or_else(|| PathBuf::from(".")).join("oedvi"),
            game_dir: None,
            download_jobs: 6,
            delete_extra: true,
            auto_install_tools: true,
            builder_env: BTreeMap::new(),
            ref_overrides: BTreeMap::new(),
            machine_name: default_machine_name(),
            share_stats: true,
            backup_before_play: true,
            backup_keep: 10,
        }
    }
}

impl Settings {
    pub fn default_path() -> PathBuf {
        dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("oedvi").join("settings.json")
    }

    pub fn load(path: &Path) -> anyhow::Result<Self> {
        match std::fs::read(path) {
            Ok(bytes) => {
                let s: Self = serde_json::from_slice(&bytes).with_context(|| format!("parsing {}", path.display()))?;
                Ok(s)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        crate::state::write_json(path, self)
    }

    /// Puts the ref chosen on this PC for the builder's clone, if any.
    pub fn apply_ref_override(&self, def: &mut oedvi_common::BuilderDef) {
        if let Some(r) = self.ref_overrides.get(def.checkout_key()).filter(|r| !r.trim().is_empty()) {
            def.git_ref = r.clone();
        }
    }

    pub fn layout(&self) -> Layout {
        Layout::new(&self.root_dir, self.game_dir.as_deref())
    }
}

/// Where everything lives on disk.
#[derive(Debug, Clone, Serialize)]
pub struct Layout {
    pub root: PathBuf,
    pub game: PathBuf,
    pub repos: PathBuf,
    /// Builder and template files (standalone mode).
    pub builders: PathBuf,
    pub templates: PathBuf,
    pub cache: PathBuf,
    pub data: PathBuf,
    pub state: PathBuf,
    pub logs: PathBuf,
}

impl Layout {
    pub fn new(root: &Path, game: Option<&Path>) -> Self {
        Self {
            root: root.to_path_buf(),
            game: game.map(Path::to_path_buf).unwrap_or_else(|| root.join("game")),
            repos: root.join("repos"),
            builders: root.join("builders"),
            templates: root.join("templates"),
            cache: root.join("cache"),
            data: root.join("data"),
            state: root.join("state"),
            logs: root.join("logs"),
        }
    }

    pub fn create_all(&self) -> std::io::Result<()> {
        for d in [&self.root, &self.game, &self.repos, &self.cache, &self.data, &self.state, &self.logs] {
            std::fs::create_dir_all(d)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_install_is_standalone_with_no_server() {
        let dir = tempfile::tempdir().unwrap();
        let s = Settings::load(&dir.path().join("none.json")).unwrap();
        assert_eq!(s.mode, Mode::Standalone);
        assert!(s.server_url.is_empty() && s.token.is_empty());
    }

    #[test]
    fn settings_files_from_before_modes_still_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, r#"{"server_url":"http://nas:9000","token":"t"}"#).unwrap();
        let s = Settings::load(&path).unwrap();
        assert_eq!((s.mode, s.server_url.as_str(), s.token.as_str()), (Mode::Standalone, "http://nas:9000", "t"));
    }
}
