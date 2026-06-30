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
    pub weapon: u64,
    pub riven_attribute: u64,
    pub riven_attribute_base: u64,
    pub riven_attribute_name: u64,
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
    /// Unresolved relic rewards classified as expected non-catalog pickups.
    pub relics_unresolved_expected: u64,
    /// Unresolved relic rewards classified as genuine misses.
    pub relics_unresolved_genuine: u64,
    /// Unresolved drops classified as expected non-catalog pickups.
    pub drops_unresolved_expected: u64,
    /// Unresolved drops classified as genuine misses.
    pub drops_unresolved_genuine: u64,
    /// Drop rows skipped for non-positive chance (source placeholders).
    pub drops_zero_chance: u64,
    /// Display names seen mapping to more than one `uniqueName`.
    pub name_collisions: u64,
    /// Resolved relic reward lines whose display name was colliding.
    pub reward_name_collisions: u64,
    /// Droptable section ids the parser did not handle.
    pub unknown_sections: Vec<String>,
    /// Name coverage keyed by language tag.
    pub name_coverage: BTreeMap<String, NameCoverage>,
}

impl Quality {
    /// Relic resolve ratio in `[0, 1]` (0 when no relics were parsed).
    pub fn relic_resolve_ratio(&self) -> f64 {
        if self.relics_total == 0 {
            0.0
        } else {
            self.relics_resolved as f64 / self.relics_total as f64
        }
    }

    /// Effective relic resolve: resolved plus expected non-catalog rewards, over total.
    pub fn effective_resolve_ratio(&self) -> f64 {
        if self.relics_total == 0 {
            0.0
        } else {
            (self.relics_resolved + self.relics_unresolved_expected) as f64
                / self.relics_total as f64
        }
    }
}

/// Tolerance for metric regression between two builds.
pub struct DiffTolerance {
    /// Max fractional drop in a table's row count before it is a regression (0.10 = 10%).
    pub table_drop_frac: f64,
    /// Max absolute drop in the relic resolve ratio before it is a regression.
    pub relic_resolve_drop: f64,
}

impl Default for DiffTolerance {
    fn default() -> Self {
        Self {
            table_drop_frac: 0.10,
            relic_resolve_drop: 0.02,
        }
    }
}

/// One metric that regressed in `current` relative to `baseline`.
#[derive(Debug, Clone, PartialEq)]
pub struct Regression {
    pub metric: String,
    pub baseline: f64,
    pub current: f64,
}

/// Compare `current` against `baseline`, one regression per metric that dropped past `tol`.
pub fn diff(current: &Quality, baseline: &Quality, tol: &DiffTolerance) -> Vec<Regression> {
    let mut out = Vec::new();

    let tables: [(&str, u64, u64); 6] = [
        ("items", current.tables.items, baseline.tables.items),
        (
            "item_names",
            current.tables.item_names,
            baseline.tables.item_names,
        ),
        (
            "relic_rewards",
            current.tables.relic_rewards,
            baseline.tables.relic_rewards,
        ),
        (
            "item_drops",
            current.tables.item_drops,
            baseline.tables.item_drops,
        ),
        (
            "drop_places",
            current.tables.drop_places,
            baseline.tables.drop_places,
        ),
        (
            "place_names",
            current.tables.place_names,
            baseline.tables.place_names,
        ),
    ];
    for (name, cur, base) in tables {
        if base > 0 && (cur as f64) < (base as f64) * (1.0 - tol.table_drop_frac) {
            out.push(Regression {
                metric: format!("table {name}"),
                baseline: base as f64,
                current: cur as f64,
            });
        }
    }

    let cur_ratio = current.relic_resolve_ratio();
    let base_ratio = baseline.relic_resolve_ratio();
    if base_ratio - cur_ratio > tol.relic_resolve_drop {
        out.push(Regression {
            metric: "relic_resolve_ratio".to_string(),
            baseline: base_ratio,
            current: cur_ratio,
        });
    }

    out
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
                weapon: 2,
                riven_attribute: 2,
                riven_attribute_base: 4,
                riven_attribute_name: 3,
            },
            relics_total: 5,
            relics_resolved: 5,
            relics_unresolved_expected: 0,
            relics_unresolved_genuine: 0,
            drops_unresolved_expected: 1,
            drops_unresolved_genuine: 0,
            drops_zero_chance: 0,
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

    #[test]
    fn diff_flags_table_drop() {
        let mut base = sample();
        base.tables.item_drops = 100;
        let mut cur = sample();
        cur.tables.item_drops = 60;
        let regs = diff(&cur, &base, &DiffTolerance::default());
        assert!(regs.iter().any(|r| r.metric == "table item_drops"));
    }

    #[test]
    fn diff_empty_when_stable() {
        let q = sample();
        assert!(diff(&q, &q, &DiffTolerance::default()).is_empty());
    }

    #[test]
    fn diff_flags_relic_resolve_drop() {
        let mut base = sample();
        base.relics_total = 100;
        base.relics_resolved = 100;
        let mut cur = sample();
        cur.relics_total = 100;
        cur.relics_resolved = 90;
        let regs = diff(&cur, &base, &DiffTolerance::default());
        assert!(regs.iter().any(|r| r.metric == "relic_resolve_ratio"));
    }
}
