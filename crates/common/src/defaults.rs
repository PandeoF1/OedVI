//! Templates built into OedVI. They exist without any file: a template file
//! with the same id (on this PC, or on the server when connected) replaces
//! the built-in one.

/// `(id, text)` of the built-in templates.
pub const TEMPLATES: &[(&str, &str)] = &[
    ("victory", include_str!("../../../examples/server/templates/victory.toml")),
    ("victory-dexnom", include_str!("../../../examples/server/templates/victory-dexnom.toml")),
];

pub fn template(id: &str) -> Option<&'static str> {
    TEMPLATES.iter().find(|(i, _)| *i == id).map(|(_, t)| *t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_template_file_is_built_in() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/server/templates");
        let files = std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "toml"))
            .count();
        assert_eq!(files, TEMPLATES.len(), "add the new template to defaults.rs");
        for (id, text) in TEMPLATES {
            assert!(text.parse::<toml::Table>().is_ok(), "{id} is not valid TOML");
        }
    }
}
