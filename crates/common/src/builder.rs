//! Builder definitions: how to fetch, build and run one variant of the game.
//!
//! Builders are TOML files on the server (`builders/<id>.toml`). To keep fifty
//! of them short, a builder can `extends = "<template>"` a file of
//! `templates/` (or another builder): tables are merged key by key, the
//! child's values winning, arrays replaced as a whole.
//!
//! Strings in `build`, `run`, `env`, `vars` and `run_workdir` can use
//! placeholders: `{game_dir}`, `{repo_dir}`, any `vars` entry, and so on.
//! An argument that is exactly `"{name...}"` expands to the value split on
//! whitespace, zero or more arguments. `{{` and `}}` stand for braces.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::paths::is_valid_id;

/// Placeholders the client always provides.
pub const BUILTIN_VARS: &[&str] =
    &["game_dir", "cache_dir", "data_dir", "root_dir", "repo_dir", "work_dir", "builder_id", "exe_suffix", "os"];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BuilderDef {
    /// File name without `.toml`; set by the server.
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    /// `rust`, `python`, `node`, `dotnet`, `cmake` or `custom`: decides which
    /// tools the launcher checks for before building.
    #[serde(default = "default_stack")]
    pub stack: String,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Lower first in the launcher's list.
    #[serde(default)]
    pub order: i32,
    pub repo: String,
    #[serde(rename = "ref", default = "default_ref")]
    pub git_ref: String,
    /// Builders with the same `checkout` share one clone (and its build
    /// output). They must name the same `repo` and `ref`. Defaults to the id.
    #[serde(default)]
    pub checkout: Option<String>,
    /// Directory inside the repository where commands run.
    #[serde(default)]
    pub workdir: Option<String>,
    #[serde(default)]
    pub vars: BTreeMap<String, String>,
    /// Environment of both the build and the run.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub build_env: BTreeMap<String, String>,
    #[serde(default)]
    pub run_env: BTreeMap<String, String>,
    /// Commands run in order; each is an argument list.
    #[serde(default)]
    pub build: Vec<Vec<String>>,
    pub run: Vec<String>,
    #[serde(default)]
    pub run_workdir: Option<String>,
    /// Regular expressions finding the FPS in the game's output (the number
    /// is the first capture group). Empty: the usual `fps` formats.
    #[serde(default)]
    pub fps_patterns: Vec<String>,
    /// More numbers to read in the game's output: name → regex whose first
    /// capture group is the value. They add to the usual ones (`fps`,
    /// `frame_ms`, `gpu`, `cpu`, `ram`, `draws`); the same name replaces it.
    #[serde(default)]
    pub metrics: BTreeMap<String, String>,
    /// Phases of the game, found in its output: name → regex. A line that
    /// matches starts that phase (the last match wins), and the charts show
    /// it as a band.
    #[serde(default)]
    pub phases: BTreeMap<String, String>,
    /// Extra programs that must be on PATH, beyond those of the stack.
    #[serde(default)]
    pub requires: Vec<String>,
}

fn default_stack() -> String {
    "custom".into()
}

fn default_ref() -> String {
    "main".into()
}

impl BuilderDef {
    pub fn checkout_key(&self) -> &str {
        self.checkout.as_deref().unwrap_or(&self.id)
    }

    fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("`name` is empty".into());
        }
        if self.repo.trim().is_empty() {
            return Err("`repo` is empty".into());
        }
        if self.git_ref.trim().is_empty() || self.git_ref.starts_with('-') {
            return Err("`ref` is invalid".into());
        }
        if self.run.is_empty() {
            return Err("`run` is empty".into());
        }
        if self.build.iter().any(|step| step.is_empty()) {
            return Err("a `build` step is empty".into());
        }
        if let Some(c) = &self.checkout
            && !is_valid_id(c)
        {
            return Err(format!("`checkout` {c:?} is not a valid name"));
        }
        if let Some(w) = &self.workdir
            && !crate::paths::is_safe_relative(w)
        {
            return Err(format!("`workdir` {w:?} must be a relative path inside the repository"));
        }
        for p in &self.fps_patterns {
            let re = regex::Regex::new(p).map_err(|e| format!("`fps_patterns` {p:?}: {e}"))?;
            if re.captures_len() < 2 {
                return Err(format!("`fps_patterns` {p:?} needs a capture group around the number"));
            }
        }
        for (name, p) in &self.metrics {
            let ok_name = name.len() <= 32
                && name.starts_with(|c: char| c.is_ascii_lowercase())
                && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
            if !ok_name {
                return Err(format!("`metrics` name {name:?}: lowercase letters, digits and _ only"));
            }
            let re = regex::Regex::new(p).map_err(|e| format!("`metrics.{name}` {p:?}: {e}"))?;
            if re.captures_len() < 2 {
                return Err(format!("`metrics.{name}` needs a capture group around the number"));
            }
        }
        for (name, p) in &self.phases {
            if name.trim().is_empty() || name.len() > 60 {
                return Err(format!("`phases` name {name:?} is empty or too long"));
            }
            regex::Regex::new(p).map_err(|e| format!("`phases.{name}` {p:?}: {e}"))?;
        }
        // Every placeholder must be known.
        let known: BTreeSet<&str> = BUILTIN_VARS.iter().copied().chain(self.vars.keys().map(String::as_str)).collect();
        let strings = self
            .build
            .iter()
            .flatten()
            .chain(&self.run)
            .chain(self.vars.values())
            .chain(self.env.values())
            .chain(self.build_env.values())
            .chain(self.run_env.values())
            .chain(self.run_workdir.iter());
        for s in strings {
            for name in referenced_vars(s)? {
                if !known.contains(name.as_str()) {
                    return Err(format!("unknown placeholder {{{name}}} in {s:?}"));
                }
            }
        }
        resolve_vars(&BUILTIN_VARS.iter().map(|v| (v.to_string(), String::new())).collect(), &self.vars)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BuilderLoadError {
    pub id: String,
    pub message: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BuilderList {
    pub builders: Vec<BuilderDef>,
    pub errors: Vec<BuilderLoadError>,
}

/// Resolves `extends` chains and validates every builder.
/// `templates` and `builders` map ids to TOML text.
pub fn resolve_all(templates: &BTreeMap<String, String>, builders: &BTreeMap<String, String>) -> BuilderList {
    let mut list = BuilderList::default();
    for (id, text) in builders {
        match resolve_one(id, text, templates, builders) {
            Ok(def) => list.builders.push(def),
            Err(message) => list.errors.push(BuilderLoadError { id: id.clone(), message }),
        }
    }

    // Builders sharing a checkout must agree on what is checked out.
    let mut by_checkout: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, b) in list.builders.iter().enumerate() {
        by_checkout.entry(b.checkout_key().to_string()).or_default().push(i);
    }
    let mut bad = BTreeSet::new();
    for (key, idx) in &by_checkout {
        let first = &list.builders[idx[0]];
        for &i in &idx[1..] {
            let b = &list.builders[i];
            if b.repo != first.repo || b.git_ref != first.git_ref {
                bad.insert(i);
                list.errors.push(BuilderLoadError {
                    id: b.id.clone(),
                    message: format!("shares checkout {key:?} with {:?} but names another repo or ref", first.id),
                });
            }
        }
    }
    let mut i = 0;
    list.builders.retain(|_| {
        let keep = !bad.contains(&i);
        i += 1;
        keep
    });
    list.builders.sort_by(|a, b| a.order.cmp(&b.order).then_with(|| a.name.cmp(&b.name)));
    list
}

/// Resolves and validates one builder without the cross-builder checks.
pub fn resolve_one(
    id: &str,
    text: &str,
    templates: &BTreeMap<String, String>,
    builders: &BTreeMap<String, String>,
) -> Result<BuilderDef, String> {
    if !is_valid_id(id) {
        return Err(format!("{id:?} is not a valid builder id"));
    }
    let table = merged_table(id, text, templates, builders, &mut Vec::new())?;
    let mut def: BuilderDef =
        toml::Value::Table(table).try_into().map_err(|e: toml::de::Error| e.message().to_string())?;
    def.id = id.to_string();
    def.validate()?;
    Ok(def)
}

fn merged_table(
    id: &str,
    text: &str,
    templates: &BTreeMap<String, String>,
    builders: &BTreeMap<String, String>,
    stack: &mut Vec<String>,
) -> Result<toml::Table, String> {
    if stack.iter().any(|s| s == id) {
        stack.push(id.to_string());
        return Err(format!("`extends` loop: {}", stack.join(" -> ")));
    }
    if stack.len() > 16 {
        return Err("`extends` chain is longer than 16".into());
    }
    stack.push(id.to_string());
    let mut table: toml::Table = text.parse().map_err(|e: toml::de::Error| {
        if stack.len() > 1 { format!("in {id}: {}", e.message()) } else { e.message().to_string() }
    })?;
    let result = match table.remove("extends") {
        None => Ok(table),
        Some(toml::Value::String(parent)) => {
            let parent_text = templates
                .get(&parent)
                .or_else(|| builders.get(&parent))
                .ok_or_else(|| format!("`extends` names {parent:?}, which is neither a template nor a builder"))?;
            let mut base = merged_table(&parent, parent_text, templates, builders, stack)?;
            merge(&mut base, table);
            Ok(base)
        }
        Some(_) => Err("`extends` must be a string".into()),
    };
    stack.pop();
    result
}

/// Deep-merges `over` into `base`: tables merge, everything else is replaced.
fn merge(base: &mut toml::Table, over: toml::Table) {
    for (k, v) in over {
        match (base.get_mut(&k), v) {
            (Some(toml::Value::Table(b)), toml::Value::Table(o)) => merge(b, o),
            (_, v) => {
                base.insert(k, v);
            }
        }
    }
}

/// Names of the placeholders used in `s`.
pub fn referenced_vars(s: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    scan(s, |name, _spread| {
        out.push(name.to_string());
        Ok(String::new())
    })?;
    Ok(out)
}

/// Replaces placeholders in `s`. A spread `{x...}` is only allowed as a whole
/// argument (`substitute_argv`); here it is replaced by the plain value.
pub fn substitute(s: &str, vars: &BTreeMap<String, String>) -> Result<String, String> {
    scan(s, |name, _| vars.get(name).cloned().ok_or_else(|| format!("unknown placeholder {{{name}}}")))
}

pub fn substitute_argv(argv: &[String], vars: &BTreeMap<String, String>) -> Result<Vec<String>, String> {
    let mut out = Vec::with_capacity(argv.len());
    for arg in argv {
        if let Some(name) = arg.strip_prefix('{').and_then(|a| a.strip_suffix("...}"))
            && !name.contains(['{', '}'])
        {
            let value = vars.get(name).ok_or_else(|| format!("unknown placeholder {{{name}}}"))?;
            out.extend(value.split_whitespace().map(str::to_string));
            continue;
        }
        out.push(substitute(arg, vars)?);
    }
    Ok(out)
}

/// Resolves `vars`, which may refer to builtins and to each other.
pub fn resolve_vars(
    builtins: &BTreeMap<String, String>,
    vars: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>, String> {
    fn get(
        name: &str,
        builtins: &BTreeMap<String, String>,
        vars: &BTreeMap<String, String>,
        done: &mut BTreeMap<String, String>,
        stack: &mut Vec<String>,
    ) -> Result<String, String> {
        if let Some(v) = done.get(name) {
            return Ok(v.clone());
        }
        let Some(raw) = vars.get(name) else {
            return builtins.get(name).cloned().ok_or_else(|| format!("unknown placeholder {{{name}}}"));
        };
        if stack.iter().any(|s| s == name) {
            return Err(format!("variables refer to each other in a loop: {} -> {name}", stack.join(" -> ")));
        }
        stack.push(name.to_string());
        let value = scan(raw, |inner, _| get(inner, builtins, vars, done, stack))?;
        stack.pop();
        done.insert(name.to_string(), value.clone());
        Ok(value)
    }

    let mut done = BTreeMap::new();
    for name in vars.keys() {
        get(name, builtins, vars, &mut done, &mut Vec::new())?;
    }
    let mut all = builtins.clone();
    all.extend(done);
    Ok(all)
}

fn scan(s: &str, mut f: impl FnMut(&str, bool) -> Result<String, String>) -> Result<String, String> {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(pos) = rest.find(['{', '}']) {
        out.push_str(&rest[..pos]);
        let tail = &rest[pos..];
        if let Some(after) = tail.strip_prefix("{{") {
            out.push('{');
            rest = after;
        } else if let Some(after) = tail.strip_prefix("}}") {
            out.push('}');
            rest = after;
        } else if tail.starts_with('}') {
            return Err(format!("unmatched `}}` in {s:?} (write `}}}}` for a brace)"));
        } else {
            let end = tail.find('}').ok_or_else(|| format!("unclosed `{{` in {s:?}"))?;
            let inner = &tail[1..end];
            let (name, spread) = match inner.strip_suffix("...") {
                Some(n) => (n, true),
                None => (inner, false),
            };
            if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                return Err(format!("invalid placeholder {{{inner}}} in {s:?}"));
            }
            out.push_str(&f(name, spread)?);
            rest = &tail[end + 1..];
        }
    }
    out.push_str(rest);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    const TEMPLATE: &str = r#"
stack = "rust"
repo = "https://example.com/r.git"
tags = ["victory"]
build = [["cargo", "build", "--profile", "{profile}", "{cargo_flags...}"]]
run = ["{repo_dir}/target/{profile_dir}/victory{exe_suffix}", "run", "{game_dir}/game.exe"]
[vars]
profile = "release"
profile_dir = "{profile}"
cargo_flags = ""
[env]
VICTORY_GAME_DIR = "{game_dir}"
"#;

    #[test]
    fn extends_merges_tables() {
        let templates = map(&[("victory", TEMPLATE)]);
        let builders = map(&[(
            "diag",
            r#"
extends = "victory"
name = "Diag"
checkout = "victory"
[vars]
profile = "diag"
cargo_flags = "--features diagnostics"
[env]
VICTORY_HAIR = "1"
"#,
        )]);
        let list = resolve_all(&templates, &builders);
        assert!(list.errors.is_empty(), "{:?}", list.errors);
        let b = &list.builders[0];
        assert_eq!(b.id, "diag");
        assert_eq!(b.stack, "rust");
        assert_eq!(b.vars["profile"], "diag");
        assert_eq!(b.vars["profile_dir"], "{profile}");
        assert_eq!(b.env.len(), 2);

        let builtins = map(&[("game_dir", "G"), ("repo_dir", "R"), ("exe_suffix", ".exe")]);
        let vars = resolve_vars(&builtins, &b.vars).unwrap();
        assert_eq!(
            substitute_argv(&b.build[0], &vars).unwrap(),
            ["cargo", "build", "--profile", "diag", "--features", "diagnostics"]
        );
        assert_eq!(substitute_argv(&b.run, &vars).unwrap()[0], "R/target/diag/victory.exe");
    }

    #[test]
    fn empty_spread_disappears() {
        let vars = map(&[("x", "")]);
        assert_eq!(substitute_argv(&["a".into(), "{x...}".into()], &vars).unwrap(), ["a"]);
    }

    #[test]
    fn errors_are_reported() {
        let templates = map(&[("victory", TEMPLATE)]);
        let builders = map(&[
            ("typo", "extends = \"victory\"\nname = \"T\"\nrnu = []"),
            ("unknown", "extends = \"victory\"\nname = \"U\"\nrun = [\"{nope}\"]"),
            ("missing", "extends = \"nothing\"\nname = \"M\""),
            ("loop", "extends = \"loop\"\nname = \"L\""),
            ("ok", "extends = \"victory\"\nname = \"OK\""),
        ]);
        let list = resolve_all(&templates, &builders);
        assert_eq!(list.builders.len(), 1);
        let ids: Vec<_> = list.errors.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["loop", "missing", "typo", "unknown"]);
    }

    #[test]
    fn shared_checkout_must_agree() {
        let templates = map(&[("victory", TEMPLATE)]);
        let builders = map(&[
            ("a", "extends = \"victory\"\nname = \"A\"\ncheckout = \"v\""),
            ("b", "extends = \"victory\"\nname = \"B\"\ncheckout = \"v\"\nref = \"dev\""),
        ]);
        let list = resolve_all(&templates, &builders);
        assert_eq!(list.builders.len(), 1);
        assert_eq!(list.errors[0].id, "b");
    }

    #[test]
    fn braces() {
        assert_eq!(substitute("a{{b}}", &BTreeMap::new()).unwrap(), "a{b}");
        assert!(substitute("a}", &BTreeMap::new()).is_err());
        assert!(substitute("{a", &BTreeMap::new()).is_err());
    }

    /// The builders and templates we ship must resolve.
    #[test]
    fn shipped_examples_resolve() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/server");
        let read = |sub: &str| -> BTreeMap<String, String> {
            std::fs::read_dir(dir.join(sub))
                .unwrap()
                .flatten()
                .filter(|e| e.path().extension().is_some_and(|x| x == "toml"))
                .map(|e| {
                    (
                        e.path().file_stem().unwrap().to_string_lossy().into_owned(),
                        std::fs::read_to_string(e.path()).unwrap(),
                    )
                })
                .collect()
        };
        let list = resolve_all(&read("templates"), &read("builders"));
        assert!(list.errors.is_empty(), "{:?}", list.errors);
        assert!(list.builders.len() >= 10);
        let dex = list.builders.iter().find(|b| b.id == "dexnom-release").unwrap();
        assert_eq!(dex.phases.len(), 4);
    }
}
