//! Which game files to keep locally.
//!
//! A selection is a set of rules rather than a list of files, so that files
//! the server adds later follow the folder or group they land in:
//!
//! 1. required files are always selected;
//! 2. otherwise the longest path rule covering the file decides (a rule on
//!    `movies` covers `movies/intro.bk2`);
//! 3. otherwise the file's group decides;
//! 4. otherwise `default_selected`.

use std::collections::BTreeMap;

use oedvi_common::{FileEntry, Manifest};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Selection {
    pub default_selected: bool,
    pub groups: BTreeMap<String, bool>,
    pub paths: BTreeMap<String, bool>,
}

impl Default for Selection {
    fn default() -> Self {
        Self { default_selected: true, groups: BTreeMap::new(), paths: BTreeMap::new() }
    }
}

fn covers(prefix: &str, path: &str) -> bool {
    path == prefix || (path.starts_with(prefix) && path.as_bytes().get(prefix.len()) == Some(&b'/'))
}

impl Selection {
    pub fn is_selected(&self, f: &FileEntry) -> bool {
        if f.required {
            return true;
        }
        let by_path =
            self.paths.iter().filter(|(p, _)| covers(p, &f.path)).max_by_key(|(p, _)| p.len()).map(|(_, v)| *v);
        by_path.or_else(|| self.groups.get(&f.group).copied()).unwrap_or(self.default_selected)
    }

    /// Selects or deselects a folder or file: rules below it are dropped.
    pub fn set_path(&mut self, prefix: &str, selected: bool) {
        let prefix = prefix.trim_matches('/');
        if prefix.is_empty() {
            self.set_all(selected);
            return;
        }
        self.paths.retain(|p, _| !covers(prefix, p));
        self.paths.insert(prefix.to_string(), selected);
    }

    /// Selects or deselects a group: path rules on its files are dropped so
    /// the group choice shows.
    pub fn set_group(&mut self, manifest: &Manifest, group: &str, selected: bool) {
        self.groups.insert(group.to_string(), selected);
        let files: Vec<&str> = manifest.files.iter().filter(|f| f.group == group).map(|f| f.path.as_str()).collect();
        self.paths.retain(|p, _| !files.iter().any(|f| covers(p, f) || covers(f, p)));
    }

    pub fn set_all(&mut self, selected: bool) {
        self.default_selected = selected;
        self.groups.clear();
        self.paths.clear();
    }

    /// Drops rules that change nothing.
    pub fn simplify(&mut self, manifest: &Manifest) {
        let keys: Vec<String> = self.paths.keys().cloned().collect();
        for k in keys {
            let v = self.paths[&k];
            self.paths.remove(&k);
            let changes = manifest.files.iter().any(|f| covers(&k, &f.path) && !f.required && self.is_selected(f) != v);
            if changes {
                self.paths.insert(k, v);
            }
        }
    }

    pub fn selected_size(&self, manifest: &Manifest) -> (u64, u64) {
        manifest.files.iter().filter(|f| self.is_selected(f)).fold((0, 0), |(n, s), f| (n + 1, s + f.size))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, group: &str, required: bool) -> FileEntry {
        FileEntry { path: path.into(), size: 10, hash: String::new(), chunks: vec![], group: group.into(), required }
    }

    fn manifest() -> Manifest {
        Manifest {
            version: 1,
            generated_at: 0,
            chunk_size: 1,
            total_size: 0,
            groups: vec![],
            files: vec![
                file("game.exe", "core", true),
                file("movies/a.bk2", "videos", false),
                file("movies/extra/b.bk2", "videos", false),
                file("audio/fr/x", "fr", false),
                file("audio/en/x", "en", false),
            ],
        }
    }

    #[test]
    fn rules() {
        let m = manifest();
        let mut s = Selection::default();
        assert!(m.files.iter().all(|f| s.is_selected(f)));

        s.set_group(&m, "videos", false);
        assert!(!s.is_selected(&m.files[1]));
        s.set_path("movies/extra", true);
        assert!(!s.is_selected(&m.files[1]));
        assert!(s.is_selected(&m.files[2]));

        // `movies` alone must not cover `moviesX`.
        assert!(!covers("movies", "moviesX/a"));

        s.set_all(false);
        assert!(s.is_selected(&m.files[0]), "required files stay selected");
        assert!(!s.is_selected(&m.files[3]));
        s.set_path("audio", true);
        s.set_path("audio/en", false);
        assert!(s.is_selected(&m.files[3]));
        assert!(!s.is_selected(&m.files[4]));
        s.set_path("audio", false);
        assert_eq!(s.paths.len(), 1, "rules below a folder are replaced");

        s.set_group(&m, "videos", true);
        assert!(s.is_selected(&m.files[2]));
        s.simplify(&m);
    }
}
