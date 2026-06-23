use crate::config::Categories;

/// Derive an item's category from config rules, its source manifest, and `uniqueName`.
pub fn derive_category(categories: &Categories, manifest: &str, unique_name: &str) -> String {
    for m in &categories.manifests {
        if m.name != manifest {
            continue;
        }
        if let Some(fixed) = &m.category {
            return fixed.clone();
        }
        for rule in &m.rules {
            if unique_name.contains(&rule.contains) {
                return rule.category.clone();
            }
        }
        if let Some(default) = &m.default {
            return default.clone();
        }
    }
    categories.default.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ManifestRule, PathRule};

    fn fixture() -> Categories {
        Categories {
            default: "other".into(),
            manifests: vec![
                ManifestRule {
                    name: "ExportWeapons".into(),
                    category: Some("weapon".into()),
                    default: None,
                    rules: vec![],
                },
                ManifestRule {
                    name: "ExportResources".into(),
                    category: None,
                    default: Some("resource".into()),
                    rules: vec![PathRule {
                        contains: "/Recipes/".into(),
                        category: "part".into(),
                    }],
                },
                ManifestRule {
                    name: "ExportRelicArcane".into(),
                    category: None,
                    default: Some("arcane".into()),
                    rules: vec![PathRule {
                        contains: "Projections".into(),
                        category: "relic".into(),
                    }],
                },
            ],
        }
    }

    #[test]
    fn fixed_category() {
        assert_eq!(
            derive_category(&fixture(), "ExportWeapons", "/Lotus/Weapons/X"),
            "weapon"
        );
    }

    #[test]
    fn rule_then_default() {
        let c = fixture();
        assert_eq!(
            derive_category(
                &c,
                "ExportResources",
                "/Lotus/Types/Recipes/WarframeRecipes/AshPrimeChassisComponent"
            ),
            "part"
        );
        assert_eq!(
            derive_category(
                &c,
                "ExportResources",
                "/Lotus/Types/Items/MiscItems/Ferrite"
            ),
            "resource"
        );
        assert_eq!(
            derive_category(&c, "ExportRelicArcane", "/Lotus/Types/Game/Projections/T1A"),
            "relic"
        );
        assert_eq!(
            derive_category(&c, "ExportRelicArcane", "/Lotus/Types/Game/Arcane/X"),
            "arcane"
        );
    }

    #[test]
    fn unknown_manifest_is_global_default() {
        assert_eq!(derive_category(&fixture(), "ExportNope", "/x"), "other");
    }
}
