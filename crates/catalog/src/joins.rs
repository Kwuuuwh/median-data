use std::collections::HashMap;

/// Map each recipe's `resultType` to its positive `primeSellingPrice` (ducat value).
pub fn ducat_map(recipes: &serde_json::Value) -> HashMap<String, i64> {
    let mut map = HashMap::new();
    let Some(obj) = recipes.as_object() else {
        return map;
    };
    for v in obj.values() {
        let Some(arr) = v.as_array() else {
            continue;
        };
        for r in arr {
            let Some(result_type) = r.get("resultType").and_then(|x| x.as_str()) else {
                continue;
            };
            let Some(price) = r.get("primeSellingPrice").and_then(|x| x.as_i64()) else {
                continue;
            };
            if price > 0 {
                map.insert(result_type.to_string(), price);
            }
        }
    }
    map
}

/// Map `uniqueName` → raw texture location, from `ExportManifest.Manifest[]`.
pub fn icon_map(manifest: &serde_json::Value) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let Some(obj) = manifest.as_object() else {
        return map;
    };
    for v in obj.values() {
        let Some(arr) = v.as_array() else {
            continue;
        };
        for m in arr {
            let (Some(unique_name), Some(texture)) = (
                m.get("uniqueName").and_then(|x| x.as_str()),
                m.get("textureLocation").and_then(|x| x.as_str()),
            ) else {
                continue;
            };
            if !unique_name.is_empty() && !texture.is_empty() {
                map.insert(unique_name.to_string(), texture.to_string());
            }
        }
    }
    map
}

/// Map a built component (`resultType`) to its blueprint (`uniqueName`), from `ExportRecipes`.
pub fn component_blueprint_map(recipes: &serde_json::Value) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let Some(obj) = recipes.as_object() else {
        return map;
    };
    for v in obj.values() {
        let Some(arr) = v.as_array() else {
            continue;
        };
        for r in arr {
            let (Some(result_type), Some(unique_name)) = (
                r.get("resultType").and_then(|x| x.as_str()),
                r.get("uniqueName").and_then(|x| x.as_str()),
            ) else {
                continue;
            };
            if !result_type.is_empty() && !unique_name.is_empty() {
                map.entry(result_type.to_string())
                    .or_insert_with(|| unique_name.to_string());
            }
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ducat_keyed_by_result_type_positive_only() {
        let recipes = serde_json::json!({
            "ExportRecipes": [
                { "uniqueName": "/R/Ash", "resultType": "/C/AshPrimeChassisComponent", "primeSellingPrice": 15 },
                { "uniqueName": "/R/None", "resultType": "/C/Plain", "primeSellingPrice": 0 }
            ]
        });
        let map = ducat_map(&recipes);
        assert_eq!(map.get("/C/AshPrimeChassisComponent"), Some(&15));
        assert_eq!(map.get("/C/Plain"), None);
    }

    #[test]
    fn icon_keyed_by_unique_name() {
        let manifest = serde_json::json!({
            "Manifest": [
                { "uniqueName": "/I/X", "textureLocation": "/Lotus/Interface/Icons/x.png!00_h" },
                { "uniqueName": "/I/Y", "textureLocation": "" }
            ]
        });
        let map = icon_map(&manifest);
        assert_eq!(
            map.get("/I/X").map(String::as_str),
            Some("/Lotus/Interface/Icons/x.png!00_h")
        );
        assert_eq!(map.get("/I/Y"), None);
    }

    #[test]
    fn component_blueprint_keyed_by_result_type() {
        let recipes = serde_json::json!({
            "ExportRecipes": [
                { "uniqueName": "/R/AshPrimeChassisBlueprint", "resultType": "/C/AshPrimeChassisComponent" }
            ]
        });
        let map = component_blueprint_map(&recipes);
        assert_eq!(
            map.get("/C/AshPrimeChassisComponent").map(String::as_str),
            Some("/R/AshPrimeChassisBlueprint")
        );
    }
}
