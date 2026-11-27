//! Standalone mode: builders and templates live in folders of this PC.
//!
//! The builders shipped with the launcher are copied there once; after that
//! they are ordinary files the user can edit (in the editor tab or by hand). A
//! shipped file the user deleted is not brought back, and one they edited is
//! never overwritten. Templates are not copied: they are built in (see
//! `oedvi_common::defaults`) and a file here replaces one.

use std::path::Path;

use oedvi_common::store::Store;

use crate::settings::Layout;

/// `(id, text)` of the builders shipped with the launcher.
pub const BUILDERS: &[(&str, &str)] = &[
    ("amd-diag", include_str!("../../../examples/server/builders/amd-diag.toml")),
    ("amd-diag-trace", include_str!("../../../examples/server/builders/amd-diag-trace.toml")),
    ("amd-diag-warp", include_str!("../../../examples/server/builders/amd-diag-warp.toml")),
    ("amd-release", include_str!("../../../examples/server/builders/amd-release.toml")),
    ("amd-series-s", include_str!("../../../examples/server/builders/amd-series-s.toml")),
    ("dexnom-release", include_str!("../../../examples/server/builders/dexnom-release.toml")),
    ("dexnom-rt", include_str!("../../../examples/server/builders/dexnom-rt.toml")),
    ("dexnom-warp", include_str!("../../../examples/server/builders/dexnom-warp.toml")),
    ("dexnom-workers", include_str!("../../../examples/server/builders/dexnom-workers.toml")),
    ("xtx7900", include_str!("../../../examples/server/builders/xtx7900.toml")),
    ("nvidia-dlss", include_str!("../../../examples/server/builders/nvidia-dlss.toml")),
    ("nvidia", include_str!("../../../examples/server/builders/nvidia.toml")),
    ("coolfox", include_str!("../../../examples/server/builders/coolfox.toml")),
    ("kelvaroq", include_str!("../../../examples/server/builders/kelvaroq.toml")),
    ("upstream", include_str!("../../../examples/server/builders/upstream.toml")),
    ("upstream-diag", include_str!("../../../examples/server/builders/upstream-diag.toml")),
];

/// Marker file listing what was already copied.
const SEEDED: &str = ".seeded";

fn seed_dir(dir: &Path, files: &[(&str, &str)]) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let marker = dir.join(SEEDED);
    let done = std::fs::read_to_string(&marker).unwrap_or_default();
    let mut done: Vec<String> = done.lines().map(String::from).collect();
    let mut changed = false;
    for (id, text) in files {
        if done.iter().any(|d| d == id) {
            continue;
        }
        let path = dir.join(format!("{id}.toml"));
        if !path.exists() {
            std::fs::write(&path, text)?;
        }
        done.push((*id).to_string());
        changed = true;
    }
    if changed {
        std::fs::write(marker, done.join("\n"))?;
    }
    Ok(())
}

/// The local builder and template folders, with the shipped files in them.
pub fn store(layout: &Layout) -> std::io::Result<Store> {
    seed_dir(&layout.builders, BUILDERS)?;
    Store::new(layout.builders.clone(), layout.templates.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use oedvi_common::store::Kind;

    #[test]
    fn shipped_files_are_all_embedded() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/server");
        let on_disk = std::fs::read_dir(dir.join("builders"))
            .unwrap()
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "toml"))
            .count();
        assert_eq!(on_disk, BUILDERS.len(), "add the new builder file to local.rs");
    }

    #[test]
    fn seeding_keeps_edits_and_deletions() {
        let tmp = tempfile::tempdir().unwrap();
        let layout = Layout::new(tmp.path(), None);
        let s = store(&layout).unwrap();
        let list = s.resolved().unwrap();
        assert!(list.errors.is_empty(), "{:?}", list.errors);
        assert_eq!(list.builders.len(), BUILDERS.len());
        // An edit and a deletion survive the next start.
        let id = BUILDERS[0].0;
        std::fs::write(layout.builders.join(format!("{id}.toml")), "# mine\n").unwrap();
        let other = BUILDERS[1].0;
        assert!(s.delete(Kind::Builder, other).unwrap());
        let s = store(&layout).unwrap();
        assert_eq!(s.read(Kind::Builder, id).unwrap().as_deref(), Some("# mine\n"));
        assert_eq!(s.read(Kind::Builder, other).unwrap(), None);
    }
}
