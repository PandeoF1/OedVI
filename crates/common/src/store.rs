//! Builder and template files on disk: `<dir>/<id>.toml`.
//! Replaced and deleted files are kept in `<dir>/.trash/`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::builder::{self, BuilderList};
use crate::paths::is_valid_id;

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Writes next to the file, then renames: a crash never leaves half a file.
fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Builder,
    Template,
}

impl Kind {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "builders" => Some(Kind::Builder),
            "templates" => Some(Kind::Template),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Builder => "builder",
            Kind::Template => "template",
        }
    }
}

pub struct Store {
    builders: PathBuf,
    templates: PathBuf,
}

impl Store {
    pub fn new(builders: PathBuf, templates: PathBuf) -> std::io::Result<Self> {
        std::fs::create_dir_all(&builders)?;
        std::fs::create_dir_all(&templates)?;
        Ok(Self { builders, templates })
    }

    fn dir(&self, kind: Kind) -> &Path {
        match kind {
            Kind::Builder => &self.builders,
            Kind::Template => &self.templates,
        }
    }

    pub fn read_all(&self, kind: Kind) -> std::io::Result<BTreeMap<String, String>> {
        let mut out = BTreeMap::new();
        for entry in std::fs::read_dir(self.dir(kind))? {
            let path = entry?.path();
            if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            let Some(id) = path.file_stem().and_then(|s| s.to_str()) else { continue };
            if !is_valid_id(id) {
                continue;
            }
            let text = std::fs::read_to_string(&path)?;
            out.insert(id.to_string(), text);
        }
        Ok(out)
    }

    pub fn resolved(&self) -> std::io::Result<BuilderList> {
        Ok(builder::resolve_all(&self.read_all(Kind::Template)?, &self.read_all(Kind::Builder)?))
    }

    pub fn read(&self, kind: Kind, id: &str) -> Result<Option<String>, String> {
        if !is_valid_id(id) {
            return Err("invalid id".into());
        }
        let path = self.dir(kind).join(format!("{id}.toml"));
        match std::fs::read_to_string(&path) {
            Ok(t) => Ok(Some(t)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    /// Checks `text` as if it were saved, without saving it. Returns the
    /// resulting builder list (errors of other builders included).
    pub fn check(&self, kind: Kind, id: &str, text: &str) -> Result<BuilderList, String> {
        if !is_valid_id(id) {
            return Err("invalid id: use letters, digits, `-`, `_` and `.`".into());
        }
        text.parse::<toml::Table>().map_err(|e| format!("TOML: {}", e.message()))?;
        let mut templates = self.read_all(Kind::Template).map_err(|e| e.to_string())?;
        let mut builders = self.read_all(Kind::Builder).map_err(|e| e.to_string())?;
        match kind {
            Kind::Template => templates.insert(id.to_string(), text.to_string()),
            Kind::Builder => builders.insert(id.to_string(), text.to_string()),
        };
        let list = builder::resolve_all(&templates, &builders);
        if kind == Kind::Builder
            && let Some(err) = list.errors.iter().find(|e| e.id == id)
        {
            return Err(err.message.clone());
        }
        Ok(list)
    }

    pub fn write(&self, kind: Kind, id: &str, text: &str) -> Result<BuilderList, String> {
        let list = self.check(kind, id, text)?;
        let path = self.dir(kind).join(format!("{id}.toml"));
        self.to_trash(kind, id, false).map_err(|e| e.to_string())?;
        write_atomic(&path, text.as_bytes()).map_err(|e| e.to_string())?;
        Ok(list)
    }

    pub fn delete(&self, kind: Kind, id: &str) -> Result<bool, String> {
        if !is_valid_id(id) {
            return Err("invalid id".into());
        }
        self.to_trash(kind, id, true).map_err(|e| e.to_string())
    }

    /// Keeps a copy of the current file in `.trash/`; moves it there when
    /// `remove` is set.
    fn to_trash(&self, kind: Kind, id: &str, remove: bool) -> std::io::Result<bool> {
        let path = self.dir(kind).join(format!("{id}.toml"));
        if !path.exists() {
            return Ok(false);
        }
        let trash = self.dir(kind).join(".trash");
        std::fs::create_dir_all(&trash)?;
        let dest = trash.join(format!("{id}.{}.toml", now_secs()));
        if remove {
            std::fs::rename(&path, dest)?;
        } else {
            std::fs::copy(&path, dest)?;
        }
        Ok(true)
    }
}
