use std::collections::HashMap;
use std::io::Read;

/// A WFM trade match for a DE `uniqueName`: the trade slug and the English display name.
#[derive(Debug, Clone)]
pub struct WfmEntry {
    /// WFM `slug` / `url_name`.
    pub url_name: String,
    /// English name from WFM `i18n.en.name`, if present.
    pub en_name: Option<String>,
}

/// One member part of a trade set, as named on WFM.
#[derive(Debug, Clone)]
pub struct WfmMember {
    /// WFM slug of the member part.
    pub slug: String,
    /// DE `uniqueName` of the member (WFM `gameRef`), possibly empty.
    pub game_ref: String,
    /// English name from WFM, if present.
    pub en_name: Option<String>,
}

/// A WFM trade set: its slug base, the assembled item it points at, and its member parts.
#[derive(Debug, Clone)]
pub struct WfmSet {
    /// Slug without the trailing `_set` (e.g. `ash_prime`).
    pub base: String,
    /// Full set slug (e.g. `ash_prime_set`).
    pub slug: String,
    /// DE `uniqueName` of the assembled item (WFM `gameRef`), possibly empty.
    pub game_ref: String,
    /// English set name from WFM, if present.
    pub en_name: Option<String>,
    /// Member parts of the set.
    pub members: Vec<WfmMember>,
}

/// WFM data split into regular items (joinable by `gameRef`) and trade sets.
#[derive(Debug, Clone, Default)]
pub struct WfmBridge {
    /// Non-set items keyed by `gameRef` (= DE `uniqueName`).
    pub by_game_ref: HashMap<String, WfmEntry>,
    /// Trade sets.
    pub sets: Vec<WfmSet>,
}

/// Fetch WFM v2 `/items` from `items_url` and split it into the gameRef bridge and trade sets.
pub fn fetch_bridge(agent: &ureq::Agent, items_url: &str) -> anyhow::Result<WfmBridge> {
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

/// A flat view of one WFM item used while splitting the response.
struct RawWfm {
    slug: String,
    game_ref: String,
    en_name: Option<String>,
    is_set: bool,
}

/// Split a WFM `/items` response into the gameRef bridge (non-set items) and trade sets.
pub fn bridge_from_items(value: &serde_json::Value) -> WfmBridge {
    let Some(data) = value.get("data").and_then(|d| d.as_array()) else {
        return WfmBridge::default();
    };

    let mut raw: Vec<RawWfm> = Vec::new();
    for item in data {
        let Some(slug) = item.get("slug").and_then(|x| x.as_str()) else {
            continue;
        };
        let game_ref = item
            .get("gameRef")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let en_name = item
            .get("i18n")
            .and_then(|i| i.get("en"))
            .and_then(|e| e.get("name"))
            .and_then(|n| n.as_str())
            .map(String::from);
        let is_set = item
            .get("tags")
            .and_then(|t| t.as_array())
            .is_some_and(|tags| tags.iter().any(|t| t.as_str() == Some("set")));
        raw.push(RawWfm {
            slug: slug.to_string(),
            game_ref,
            en_name,
            is_set,
        });
    }

    let mut by_game_ref = HashMap::new();
    for r in raw.iter().filter(|r| !r.is_set && !r.game_ref.is_empty()) {
        by_game_ref.insert(
            r.game_ref.clone(),
            WfmEntry {
                url_name: r.slug.clone(),
                en_name: r.en_name.clone(),
            },
        );
    }

    let mut sets = Vec::new();
    for s in raw.iter().filter(|r| r.is_set) {
        let Some(base) = s.slug.strip_suffix("_set") else {
            continue;
        };
        let prefix = format!("{base}_");
        let members = raw
            .iter()
            .filter(|m| !m.is_set && m.slug.starts_with(&prefix))
            .map(|m| WfmMember {
                slug: m.slug.clone(),
                game_ref: m.game_ref.clone(),
                en_name: m.en_name.clone(),
            })
            .collect();
        sets.push(WfmSet {
            base: base.to_string(),
            slug: s.slug.clone(),
            game_ref: s.game_ref.clone(),
            en_name: s.en_name.clone(),
            members,
        });
    }

    WfmBridge { by_game_ref, sets }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_non_set_items_and_sets_with_members() {
        let value = serde_json::json!({
            "data": [
                { "slug": "ash_prime_set", "gameRef": "/Lotus/Powersuits/Ninja/AshPrime", "tags": ["set","prime","warframe"], "i18n": { "en": { "name": "Ash Prime Set" } } },
                { "slug": "ash_prime_blueprint", "gameRef": "/Lotus/Types/Recipes/WarframeRecipes/AshPrimeBlueprint", "tags": ["blueprint","prime","warframe"], "i18n": { "en": { "name": "Ash Prime Blueprint" } } },
                { "slug": "ash_prime_chassis_blueprint", "gameRef": "/Lotus/Types/Recipes/WarframeRecipes/AshPrimeChassisBlueprint", "tags": ["component","prime","warframe","blueprint"], "i18n": { "en": { "name": "Ash Prime Chassis" } } },
                { "slug": "braton_prime_barrel", "gameRef": "/Lotus/X/BratonPrimeBarrel", "tags": ["component","prime"], "i18n": { "en": { "name": "Braton Prime Barrel" } } }
            ]
        });
        let bridge = bridge_from_items(&value);

        assert!(
            bridge
                .by_game_ref
                .contains_key("/Lotus/Types/Recipes/WarframeRecipes/AshPrimeBlueprint")
        );
        assert!(
            !bridge
                .by_game_ref
                .contains_key("/Lotus/Powersuits/Ninja/AshPrime"),
            "set items must be excluded from the gameRef bridge"
        );

        assert_eq!(bridge.sets.len(), 1);
        let set = &bridge.sets[0];
        assert_eq!(set.base, "ash_prime");
        assert_eq!(set.slug, "ash_prime_set");
        let slugs: Vec<&str> = set.members.iter().map(|m| m.slug.as_str()).collect();
        assert!(slugs.contains(&"ash_prime_blueprint"));
        assert!(slugs.contains(&"ash_prime_chassis_blueprint"));
        assert!(!slugs.contains(&"braton_prime_barrel"));
        assert_eq!(set.members.len(), 2);
    }
}
