use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Row counts for each catalog table.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
pub struct TableCounts {
    pub items: u64,
    pub item_names: u64,
    pub relic_rewards: u64,
    pub item_drops: u64,
    pub drop_places: u64,
    pub place_names: u64,
}

/// Per-language name coverage: how many catalog items carry a name in that language.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
pub struct NameCoverage {
    pub items_total: u64,
    pub named: u64,
}

/// Quality snapshot embedded in `meta['quality']` and emitted as a build sidecar.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Quality {
    /// DE index identity this build was assembled from.
    pub de_index_hash: String,
    /// Build time, unix epoch milliseconds.
    pub built_at_ms: i64,
    /// Per-table row counts.
    pub tables: TableCounts,
    /// Relic reward lines parsed from the source.
    pub relics_total: u64,
    /// Relic reward lines that resolved both ends to catalog keys.
    pub relics_resolved: u64,
    /// Unresolved drops classified as expected non-catalog pickups.
    pub drops_unresolved_expected: u64,
    /// Unresolved drops classified as genuine misses.
    pub drops_unresolved_genuine: u64,
    /// Display names seen mapping to more than one `uniqueName`.
    pub name_collisions: u64,
    /// Resolved relic reward lines whose display name was colliding.
    pub reward_name_collisions: u64,
    /// Droptable section ids the parser did not handle.
    pub unknown_sections: Vec<String>,
    /// Name coverage keyed by language tag.
    pub name_coverage: BTreeMap<String, NameCoverage>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Quality {
        let mut name_coverage = BTreeMap::new();
        name_coverage.insert(
            "en".to_string(),
            NameCoverage {
                items_total: 10,
                named: 10,
            },
        );
        name_coverage.insert(
            "ru".to_string(),
            NameCoverage {
                items_total: 10,
                named: 7,
            },
        );
        Quality {
            de_index_hash: "h.s3".to_string(),
            built_at_ms: 1234,
            tables: TableCounts {
                items: 10,
                item_names: 17,
                relic_rewards: 4,
                item_drops: 3,
                drop_places: 2,
                place_names: 2,
            },
            relics_total: 5,
            relics_resolved: 5,
            drops_unresolved_expected: 1,
            drops_unresolved_genuine: 0,
            name_collisions: 0,
            reward_name_collisions: 0,
            unknown_sections: vec![],
            name_coverage,
        }
    }

    #[test]
    fn round_trips_through_json() {
        let q = sample();
        let json = serde_json::to_string(&q).unwrap();
        let back: Quality = serde_json::from_str(&json).unwrap();
        assert_eq!(q, back);
    }
}
