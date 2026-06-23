use std::collections::HashMap;
use std::io::Read;

/// A WFM catalog match for a DE `uniqueName`: the trade slug and the English display name.
#[derive(Debug, Clone)]
pub struct WfmEntry {
    /// WFM `slug` / `url_name` — what to search for on warframe.market.
    pub url_name: String,
    /// English name from WFM `i18n.en.name`, if present.
    pub en_name: Option<String>,
}

/// Fetch WFM v2 `/items` from `items_url` and build the `gameRef` → `WfmEntry` bridge.
pub fn fetch_bridge(
    agent: &ureq::Agent,
    items_url: &str,
) -> anyhow::Result<HashMap<String, WfmEntry>> {
    let mut res = agent
        .get(items_url)
        .header("User-Agent", wf_fetch::USER_AGENT)
        .call()
        .map_err(|e| anyhow::anyhow!("GET {items_url}: {e}"))?;
    let mut buf = Vec::new();
    res.body_mut().as_reader().read_to_end(&mut buf)?;
    let value: serde_json::Value = serde_json::from_slice(&buf)?;
    Ok(bridge_from_items(&value))
}

/// Parse a WFM `/items` response into the `gameRef` → `WfmEntry` bridge. Items with an empty or
/// missing `gameRef` are skipped (no DE join key).
pub fn bridge_from_items(value: &serde_json::Value) -> HashMap<String, WfmEntry> {
    let mut map = HashMap::new();
    let Some(data) = value.get("data").and_then(|d| d.as_array()) else {
        return map;
    };
    for item in data {
        let Some(game_ref) = item.get("gameRef").and_then(|x| x.as_str()) else {
            continue;
        };
        if game_ref.is_empty() {
            continue;
        }
        let Some(slug) = item.get("slug").and_then(|x| x.as_str()) else {
            continue;
        };
        let en_name = item
            .get("i18n")
            .and_then(|i| i.get("en"))
            .and_then(|e| e.get("name"))
            .and_then(|n| n.as_str())
            .map(String::from);
        map.insert(
            game_ref.to_string(),
            WfmEntry {
                url_name: slug.to_string(),
                en_name,
            },
        );
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_game_ref_to_slug_and_en_name() {
        let value = serde_json::json!({
            "data": [
                {
                    "slug": "ash_prime_chassis",
                    "gameRef": "/Lotus/Types/Recipes/WarframeRecipes/AshPrimeChassisComponent",
                    "i18n": { "en": { "name": "Ash Prime Chassis" }, "ru": { "name": "Шасси Аш Прайм" } }
                },
                { "slug": "skipped", "gameRef": "" }
            ]
        });
        let map = bridge_from_items(&value);
        let entry = map
            .get("/Lotus/Types/Recipes/WarframeRecipes/AshPrimeChassisComponent")
            .expect("ash prime chassis bridged");
        assert_eq!(entry.url_name, "ash_prime_chassis");
        assert_eq!(entry.en_name.as_deref(), Some("Ash Prime Chassis"));
        assert!(!map.contains_key(""));
    }
}
