use std::collections::{HashMap, HashSet};

use crate::write::NameRow;

/// Reverse index from a normalized English display name to a catalog `uniqueName`.
pub struct NameIndex {
    by_name: HashMap<String, String>,
    collisions: usize,
    colliding: HashSet<String>,
}

impl NameIndex {
    /// Build the index from English name rows (first writer wins; later clashes are counted).
    pub fn build(names: &[NameRow]) -> Self {
        let mut by_name: HashMap<String, String> = HashMap::new();
        let mut collisions = 0usize;
        let mut colliding: HashSet<String> = HashSet::new();
        for row in names.iter().filter(|r| r.lang == "en") {
            let key = normalize(&row.name);
            if key.is_empty() {
                continue;
            }
            match by_name.get(&key) {
                Some(existing) if existing != &row.unique_name => {
                    collisions += 1;
                    colliding.insert(key);
                }
                Some(_) => {}
                None => {
                    by_name.insert(key, row.unique_name.clone());
                }
            }
        }
        Self {
            by_name,
            collisions,
            colliding,
        }
    }

    /// Number of distinct indexed names.
    pub fn name_count(&self) -> usize {
        self.by_name.len()
    }

    /// Count of names seen mapping to more than one `uniqueName`.
    pub fn collisions(&self) -> usize {
        self.collisions
    }

    /// Resolve a source display name, trying the exact name then a few conservative suffix trims.
    pub fn resolve(&self, display_name: &str) -> Option<&str> {
        let norm = normalize(display_name);
        if let Some(u) = self.by_name.get(&norm) {
            return Some(u.as_str());
        }
        for suffix in [" blueprint", " relic", " cache"] {
            if let Some(stripped) = norm.strip_suffix(suffix) {
                if let Some(u) = self.by_name.get(stripped) {
                    return Some(u.as_str());
                }
            }
        }
        None
    }

    /// Whether a display name maps to a key seen with more that one `uniqueName`.
    pub fn is_colliding(&self, display_name: &str) -> bool {
        let norm = normalize(display_name);
        if self.colliding.contains(&norm) {
            return true;
        }
        for suffix in [" blueprint", " relic", " cache"] {
            if let Some(stripped) = norm.strip_suffix(suffix) {
                if self.colliding.contains(stripped) {
                    return true;
                }
            }
        }
        false
    }
}

/// Normalize a display name for matching: collapse internal whitespace and lowercase.
fn normalize(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(unique: &str, lang: &str, n: &str) -> NameRow {
        NameRow {
            unique_name: unique.into(),
            lang: lang.into(),
            source: "DE",
            name: n.into(),
        }
    }

    #[test]
    fn resolves_exact_case_and_whitespace_insensitive() {
        let idx = NameIndex::build(&[name("/A", "en", "Akstiletto Prime Barrel")]);
        assert_eq!(idx.resolve("Akstiletto  Prime  Barrel"), Some("/A"));
        assert_eq!(idx.resolve("akstiletto prime barrel"), Some("/A"));
        assert_eq!(idx.resolve("Unknown Thing"), None);
    }

    #[test]
    fn resolves_via_blueprint_suffix_trim() {
        let idx = NameIndex::build(&[name("/N", "en", "Nikana Prime")]);
        assert_eq!(idx.resolve("Nikana Prime Blueprint"), Some("/N"));
    }

    #[test]
    fn ignores_non_english_and_counts_collisions() {
        let rows = vec![
            name("/A", "en", "Forma Blueprint"),
            name("/B", "en", "Forma Blueprint"),
            name("/C", "ru", "Чертёж: Форма"),
        ];
        let idx = NameIndex::build(&rows);
        assert_eq!(idx.collisions(), 1);
        assert_eq!(idx.resolve("Forma Blueprint"), Some("/A"));
        assert_eq!(idx.resolve("Чертёж: Форма"), None);
    }

    #[test]
    fn flags_colliding_names_only() {
        let rows = vec![
            name("/A", "en", "Forma Blueprint"),
            name("/B", "en", "Forma Blueprint"),
            name("/C", "en", "Nikana Prime"),
        ];
        let idx = NameIndex::build(&rows);
        assert!(idx.is_colliding("Forma Blueprint"));
        assert!(idx.is_colliding("forma  blueprint"));
        assert!(!idx.is_colliding("Nikana Prime"));
        assert!(!idx.is_colliding("Unknown Thing"));
    }
}
