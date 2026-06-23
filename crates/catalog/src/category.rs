/// Derive an item's category from its source manifest and `uniqueName` path.
pub fn derive_category(manifest: &str, unique_name: &str) -> &'static str {
    match manifest {
        "ExportWarframes" => "warframe",
        "ExportSentinels" => "sentinel",
        "ExportWeapons" => "weapon",
        "ExportUpgrades" => "mod",
        "ExportRelicArcane" => {
            if unique_name.contains("Projections") {
                "relic"
            } else {
                "arcane"
            }
        }
        "ExportResources" => {
            if unique_name.contains("/Recipes/") {
                "part"
            } else {
                "resource"
            }
        }
        _ => "other",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prime_part_from_recipe_path() {
        assert_eq!(
            derive_category(
                "ExportResources",
                "/Lotus/Types/Recipes/WarframeRecipes/AshPrimeChassisComponent"
            ),
            "part"
        );
    }

    #[test]
    fn plain_resource() {
        assert_eq!(
            derive_category("ExportResources", "/Lotus/Types/Items/MiscItems/Ferrite"),
            "resource"
        );
    }

    #[test]
    fn manifest_sourced_categories() {
        assert_eq!(
            derive_category("ExportUpgrades", "/Lotus/Upgrades/Mods/X"),
            "mod"
        );
        assert_eq!(
            derive_category("ExportWeapons", "/Lotus/Weapons/X"),
            "weapon"
        );
        assert_eq!(
            derive_category("ExportWarframes", "/Lotus/Powersuits/X"),
            "warframe"
        );
    }
}
