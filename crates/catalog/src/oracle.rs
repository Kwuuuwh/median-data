use std::collections::HashMap;

use wf_fetch::DropTables;

/// Normalized comparison key: (relic display, refinement token, reward display), all lowercased.
type Key = (String, String, String);

/// Map a WFCD `state` to our stored refinement token.
fn state_to_refinement(state: &str) -> Option<&'static str> {
    match state {
        "Intact" => Some("intact"),
        "Exceptional" => Some("exceptional"),
        "Flawless" => Some("flawless"),
        "Radiant" => Some("radiant"),
        _ => None,
    }
}

/// Collapse whitespace and lowercase a display name for matching.
fn normalize_name(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// WFCD relic lines keyed for comparison, plus a count of entries skipped (no name / unknown state).
fn wfcd_map(value: &serde_json::Value) -> (HashMap<Key, f64>, usize) {
    let mut map = HashMap::new();
    let mut skipped = 0usize;
    let Some(relics) = value.get("relics").and_then(|r| r.as_array()) else {
        return (map, skipped);
    };
    for relic in relics {
        let (Some(tier), Some(relic_name), Some(state)) = (
            relic.get("tier").and_then(|x| x.as_str()),
            relic.get("relicName").and_then(|x| x.as_str()),
            relic.get("state").and_then(|x| x.as_str()),
        ) else {
            skipped += 1;
            continue;
        };
        let Some(refine) = state_to_refinement(state) else {
            skipped += 1;
            continue;
        };
        let relic_key = normalize_name(&format!("{tier} {relic_name} Relic"));
        let Some(rewards) = relic.get("rewards").and_then(|r| r.as_array()) else {
            continue;
        };
        for rw in rewards {
            let (Some(item_name), Some(chance)) = (
                rw.get("itemName").and_then(|x| x.as_str()),
                rw.get("chance").and_then(|x| x.as_f64()),
            ) else {
                continue;
            };
            let key = (relic_key.clone(), refine.to_string(), normalize_name(item_name));
            map.insert(key, chance / 100.0);
        }
    }
    (map, skipped)
}

/// Our parsed relic lines keyed for comparison.
fn ours_map(dt: &DropTables) -> HashMap<Key, f64> {
    let mut map = HashMap::new();
    for r in &dt.relics {
        let key = (
            normalize_name(&r.relic_name),
            r.refinement.as_str().to_string(),
            normalize_name(&r.reward_name),
        );
        map.insert(key, r.chance);
    }
    map
}

/// Fetch WFCD relics, diff against our relic lines, print the report. Always succeeds (diagnostic).
pub fn report(agent: &ureq::Agent, wfcd_url: &str, dt: &DropTables) -> anyhow::Result<()> {
    let bytes = wf_fetch::get_bytes(agent, wfcd_url)?;
    let value: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|e| anyhow::anyhow!("parse WFCD relics.json: {e}"))?;
    let (theirs, skipped) = wfcd_map(&value);
    let ours = ours_map(dt);

    let missing: Vec<&Key> = theirs.keys().filter(|k| !ours.contains_key(*k)).collect();
    let extra: Vec<&Key> = ours.keys().filter(|k| !theirs.contains_key(*k)).collect();
    let mut chance_mismatch: Vec<(&Key, f64, f64)> = Vec::new();
    for (k, &our_chance) in &ours {
        if let Some(&their_chance) = theirs.get(k) {
            if (our_chance - their_chance).abs() > 1e-6 {
                chance_mismatch.push((k, our_chance, their_chance));
            }
        }
    }

    println!(
        "WFCD oracle (relics): ours {} lines, wfcd {} lines",
        ours.len(),
        theirs.len()
    );
    if skipped > 0 {
        println!("  skipped WFCD relics (no name / unknown state): {skipped}");
    }
    print_sample("missing_in_ours", &missing);
    print_sample("extra_in_ours", &extra);
    println!("  chance_mismatch: {}", chance_mismatch.len());
    for (k, our_c, their_c) in chance_mismatch.iter().take(20) {
        println!(
            "    {} | {} | {}: ours {our_c:.4} vs wfcd {their_c:.4}",
            k.0, k.1, k.2
        );
    }
    Ok(())
}

/// Print a labelled count and up to 20 example keys.
fn print_sample(label: &str, keys: &[&Key]) {
    println!("  {label}: {}", keys.len());
    for k in keys.iter().take(20) {
        println!("    {} | {} | {}", k.0, k.1, k.2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wf_fetch::{Refinement, RelicReward};

    #[test]
    fn normalizes_wfcd_relic_and_maps_state() {
        let value = serde_json::json!({
            "relics": [
                { "tier": "Axi", "relicName": "A1", "state": "Radiant",
                  "rewards": [ { "itemName": "Akstiletto Prime Barrel", "rarity": "Uncommon", "chance": 20.0 } ] }
            ]
        });
        let (map, skipped) = wfcd_map(&value);
        assert_eq!(skipped, 0);
        let key = (
            "axi a1 relic".to_string(),
            "radiant".to_string(),
            "akstiletto prime barrel".to_string(),
        );
        assert!((map.get(&key).copied().unwrap() - 0.20).abs() < 1e-9);
    }

    #[test]
    fn ours_key_shape_matches_wfcd() {
        let dt = DropTables {
            relics: vec![RelicReward {
                relic_name: "Axi A1 Relic".to_string(),
                refinement: Refinement::Radiant,
                reward_name: "Akstiletto Prime Barrel".to_string(),
                rarity: "Uncommon".to_string(),
                chance: 0.20,
            }],
            drops: vec![],
            unknown_sections: vec![],
        };
        let ours = ours_map(&dt);
        let key = (
            "axi a1 relic".to_string(),
            "radiant".to_string(),
            "akstiletto prime barrel".to_string(),
        );
        assert!(ours.contains_key(&key));
    }

    #[test]
    fn skips_relic_without_name() {
        let value = serde_json::json!({
            "relics": [
                { "tier": "Requiem", "state": "Intact",
                  "rewards": [ { "itemName": "Lohk", "rarity": "Uncommon", "chance": 11 } ] }
            ]
        });
        let (map, skipped) = wfcd_map(&value);
        assert!(map.is_empty());
        assert_eq!(skipped, 1);
    }

    #[test]
    fn skips_unknown_state() {
        let value = serde_json::json!({
            "relics": [
                { "tier": "Axi", "relicName": "A1", "state": "Mystery",
                  "rewards": [ { "itemName": "Thing", "chance": 1.0 } ] }
            ]
        });
        let (map, skipped) = wfcd_map(&value);
        assert!(map.is_empty());
        assert_eq!(skipped, 1);
    }
}
