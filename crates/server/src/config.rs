use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use globset::{GlobSet, GlobSetBuilder};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default = "default_bind")]
    pub bind: String,
    /// The game files to publish.
    pub game_dir: PathBuf,
    /// Index, manifest, and the default home of builders and templates.
    #[serde(default = "default_data_dir")]
    pub data_dir: PathBuf,
    #[serde(default)]
    pub builders_dir: Option<PathBuf>,
    #[serde(default)]
    pub templates_dir: Option<PathBuf>,
    /// Read access. Overridden by a non-empty `OEDVI_TOKEN`. Empty: no authentication.
    #[serde(default)]
    pub token: String,
    /// Write access (builders, rescans). Overridden by a non-empty `OEDVI_ADMIN_TOKEN`.
    /// Empty: `token` gives write access too.
    #[serde(default)]
    pub admin_token: String,
    #[serde(default = "default_chunk_mib")]
    pub chunk_size_mib: u64,
    /// Files hashed at the same time. 1 for hard disks, 2-4 for SSDs.
    #[serde(default = "default_hash_jobs")]
    pub hash_jobs: usize,
    /// Rescan when the game directory changes.
    #[serde(default = "default_true")]
    pub watch: bool,
    /// Quiet time after the last change before rescanning.
    #[serde(default = "default_debounce")]
    pub watch_debounce_secs: u64,
    /// Periodic rescan, in minutes; 0 turns it off. Useful where file system
    /// events do not arrive (network shares).
    #[serde(default)]
    pub rescan_interval_mins: u64,
    /// Globs of files left out of the manifest.
    #[serde(default = "default_ignore")]
    pub ignore: Vec<String>,
    #[serde(default = "default_group")]
    pub default_group: String,
    /// First group whose patterns match a file gets it.
    #[serde(default)]
    pub groups: Vec<GroupRule>,
    /// Launcher installers published for self-update (see `launcher.rs`).
    /// Defaults to `<data_dir>/launcher`.
    #[serde(default)]
    pub launcher_dir: Option<PathBuf>,
    /// URL clients reach this server at, for download links in the update
    /// manifest. Defaults to the `Host` header of the request.
    #[serde(default)]
    pub public_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroupRule {
    pub name: String,
    #[serde(default)]
    pub label: Option<String>,
    pub patterns: Vec<String>,
    #[serde(default)]
    pub required: bool,
}

fn default_bind() -> String {
    "0.0.0.0:7878".into()
}
fn default_data_dir() -> PathBuf {
    PathBuf::from("./oedvi-data")
}
fn default_chunk_mib() -> u64 {
    32
}
fn default_hash_jobs() -> usize {
    1
}
fn default_true() -> bool {
    true
}
fn default_debounce() -> u64 {
    15
}
fn default_group() -> String {
    "other".into()
}
fn default_ignore() -> Vec<String> {
    ["**/Thumbs.db", "**/desktop.ini", "**/.DS_Store", "**/*.tmp", "**/*.oedvi-part", "**/.git/**"]
        .into_iter()
        .map(String::from)
        .collect()
}

impl Config {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let mut cfg: Config = toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        // An empty variable (compose passes one when `.env` lacks it) must
        // not wipe a token or path set in the file.
        let env = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
        if let Some(t) = env("OEDVI_TOKEN") {
            cfg.token = t;
        }
        if let Some(t) = env("OEDVI_ADMIN_TOKEN") {
            cfg.admin_token = t;
        }
        if let Some(d) = env("OEDVI_GAME_DIR") {
            cfg.game_dir = d.into();
        }
        // Relative paths are relative to the config file.
        let base = path.parent().unwrap_or(Path::new("."));
        for p in [&mut cfg.game_dir, &mut cfg.data_dir] {
            if p.is_relative() {
                *p = base.join(&*p);
            }
        }
        for p in [&mut cfg.builders_dir, &mut cfg.templates_dir, &mut cfg.launcher_dir].into_iter().flatten() {
            if p.is_relative() {
                *p = base.join(&*p);
            }
        }
        cfg.validate()?;
        Ok(cfg)
    }

    fn validate(&self) -> anyhow::Result<()> {
        if !(1..=1024).contains(&self.chunk_size_mib) {
            bail!("chunk_size_mib must be between 1 and 1024");
        }
        if self.hash_jobs == 0 {
            bail!("hash_jobs must be at least 1");
        }
        if !self.game_dir.is_dir() {
            bail!("game_dir {} is not a directory", self.game_dir.display());
        }
        self.ignore_set()?;
        self.group_matchers()?;
        let game = self.game_dir.canonicalize()?;
        std::fs::create_dir_all(&self.data_dir)?;
        let data = self.data_dir.canonicalize()?;
        if data.starts_with(&game) {
            bail!("data_dir must not be inside game_dir");
        }
        Ok(())
    }

    pub fn chunk_size(&self) -> u64 {
        self.chunk_size_mib * 1024 * 1024
    }

    pub fn builders_dir(&self) -> PathBuf {
        self.builders_dir.clone().unwrap_or_else(|| self.data_dir.join("builders"))
    }

    pub fn launcher_dir(&self) -> PathBuf {
        self.launcher_dir.clone().unwrap_or_else(|| self.data_dir.join("launcher"))
    }

    pub fn templates_dir(&self) -> PathBuf {
        self.templates_dir.clone().unwrap_or_else(|| self.data_dir.join("templates"))
    }

    pub fn ignore_set(&self) -> anyhow::Result<GlobSet> {
        build_set(&self.ignore)
    }

    pub fn group_matchers(&self) -> anyhow::Result<Vec<(GroupRule, GlobSet)>> {
        self.groups.iter().map(|g| Ok((g.clone(), build_set(&g.patterns)?))).collect()
    }
}

fn build_set(patterns: &[String]) -> anyhow::Result<GlobSet> {
    let mut b = GlobSetBuilder::new();
    for p in patterns {
        let glob = globset::GlobBuilder::new(p)
            .literal_separator(true)
            .case_insensitive(true)
            .build()
            .with_context(|| format!("invalid pattern {p:?}"))?;
        b.add(glob);
    }
    Ok(b.build()?)
}
