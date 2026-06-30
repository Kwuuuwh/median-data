use std::path::Path;

use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

/// Tunable validation thresholds. Defaults are starting values — calibrate from real metrics.
pub struct Thresholds {
    pub min_items: i64,
    pub min_item_names: i64,
    pub min_relic_rewards: i64,
    pub min_item_drops: i64,
    pub min_drop_places: i64,
    pub min_place_names: i64,
    pub min_weapon: i64,
    pub min_riven_attribute: i64,
    pub min_riven_attribute_base: i64,
    pub ru_name_gap_budget: i64,
    pub relic_resolve_min: f64,
    pub overshoot_max: f64,
    pub reward_collision_budget: u64,
    pub genuine_unresolved_budget: u64,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            min_items: 1000,
            min_item_names: 1000,
            min_relic_rewards: 200,
            min_item_drops: 200,
            min_drop_places: 20,
            min_place_names: 20,
            min_weapon: 900,
            min_riven_attribute: 30,
            min_riven_attribute_base: 150,
            ru_name_gap_budget: 600,
            relic_resolve_min: 0.98,
            overshoot_max: 1.02,
            reward_collision_budget: 50,
            genuine_unresolved_budget: 200,
        }
    }
}

/// Collected validation outcome: hard failures (gate) and soft warnings (diagnostic).
pub struct Report {
    pub failures: Vec<String>,
    pub warnings: Vec<String>,
}

impl Report {
    fn new() -> Self {
        Self {
            failures: Vec::new(),
            warnings: Vec::new(),
        }
    }

    /// Whether the catalog cleared every hard check.
    pub fn passed(&self) -> bool {
        self.failures.is_empty()
    }

    fn fail(&mut self, msg: impl Into<String>) {
        self.failures.push(msg.into());
    }

    fn warn(&mut self, msg: impl Into<String>) {
        self.warnings.push(msg.into());
    }

    /// Print warnings then either PASS or the failure list.
    pub fn print(&self) {
        for w in &self.warnings {
            println!("WARN: {w}");
        }
        if self.failures.is_empty() {
            println!(
                "PASS: catalog validation ({} warning(s))",
                self.warnings.len()
            );
        } else {
            for f in &self.failures {
                println!("FAIL: {f}");
            }
            println!("FAILED: {} issue(s)", self.failures.len());
        }
    }
}

/// Open the catalog at `path` read-only and run every check with default thresholds.
pub async fn run(path: &Path) -> anyhow::Result<Report> {
    let options = SqliteConnectOptions::new().filename(path).read_only(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .map_err(|e| anyhow::anyhow!("open {}: {e}", path.display()))?;
    let report = check(&pool, &Thresholds::default()).await?;
    pool.close().await;
    report.print();
    Ok(report)
}

/// Run all checks against an open pool.
async fn check(pool: &SqlitePool, t: &Thresholds) -> anyhow::Result<Report> {
    let mut report = Report::new();

    for (table, floor) in [
        ("items", t.min_items),
        ("item_names", t.min_item_names),
        ("relic_rewards", t.min_relic_rewards),
        ("item_drops", t.min_item_drops),
        ("drop_places", t.min_drop_places),
        ("place_names", t.min_place_names),
        ("weapon", t.min_weapon),
        ("riven_attribute", t.min_riven_attribute),
        ("riven_attribute_base", t.min_riven_attribute_base),
    ] {
        let n = count(pool, &format!("SELECT count(*) FROM {table}")).await?;
        if n < floor {
            report.fail(format!("table {table}: {n} rows < floor {floor}"));
        }
    }

    let fk_checks: [(&str, &str); 5] = [
        (
            "item_drops.item_unique_name -> items",
            "SELECT count(*) FROM item_drops d \
             LEFT JOIN items i ON d.item_unique_name = i.unique_name WHERE i.unique_name IS NULL",
        ),
        (
            "item_drops.place_ref -> drop_places",
            "SELECT count(*) FROM item_drops d \
             LEFT JOIN drop_places p ON d.place_ref = p.place_ref WHERE p.place_ref IS NULL",
        ),
        (
            "relic_rewards.relic_unique_name -> items",
            "SELECT count(*) FROM relic_rewards r \
             LEFT JOIN items i ON r.relic_unique_name = i.unique_name WHERE i.unique_name IS NULL",
        ),
        (
            "relic_rewards.reward_unique_name -> items",
            "SELECT count(*) FROM relic_rewards r \
             LEFT JOIN items i ON r.reward_unique_name = i.unique_name WHERE i.unique_name IS NULL",
        ),
        (
            "place_names.place_ref -> drop_places",
            "SELECT count(*) FROM place_names n \
             LEFT JOIN drop_places p ON n.place_ref = p.place_ref WHERE p.place_ref IS NULL",
        ),
    ];
    for (label, sql) in fk_checks {
        let n = count(pool, sql).await?;
        if n > 0 {
            report.fail(format!("soft FK {label}: {n} dangling row(s)"));
        }
    }

    let refinement_bad = count(
        pool,
        "SELECT count(*) FROM relic_rewards \
         WHERE refinement NOT IN ('intact','exceptional','flawless','radiant')",
    )
    .await?;
    if refinement_bad > 0 {
        report.fail(format!(
            "relic_rewards.refinement: {refinement_bad} out-of-enum row(s)"
        ));
    }

    let rarity_bad = count(
        pool,
        "SELECT (SELECT count(*) FROM relic_rewards WHERE rarity = '') \
              + (SELECT count(*) FROM item_drops WHERE rarity = '')",
    )
    .await?;
    if rarity_bad > 0 {
        report.fail(format!("empty rarity labels: {rarity_bad} row(s)"));
    }

    let chance_bad = count(
        pool,
        "SELECT (SELECT count(*) FROM relic_rewards WHERE NOT (chance > 0 AND chance <= 1)) \
              + (SELECT count(*) FROM item_drops WHERE NOT (chance > 0 AND chance <= 1))",
    )
    .await?;
    if chance_bad > 0 {
        report.fail(format!("chance out of (0,1]: {chance_bad} row(s)"));
    }

    let overshoot = count(
        pool,
        &format!(
            "SELECT count(*) FROM (SELECT relic_unique_name, refinement, sum(chance) s \
             FROM relic_rewards GROUP BY relic_unique_name, refinement HAVING s > {})",
            t.overshoot_max
        ),
    )
    .await?;
    if overshoot > 0 {
        report.fail(format!(
            "relic chance overshoot (> {}): {overshoot} (relic, refinement) group(s)",
            t.overshoot_max
        ));
    }

    let en_missing = count(
        pool,
        "SELECT count(*) FROM items \
         WHERE unique_name NOT IN (SELECT unique_name FROM item_names WHERE lang = 'en')",
    )
    .await?;
    if en_missing > 0 {
        report.fail(format!("items without an en name: {en_missing}"));
    }

    let langs = meta_value(pool, "langs").await?.unwrap_or_default();
    if langs.split(',').any(|l| l.trim() == "ru") {
        let ru_gap = count(
            pool,
            "SELECT count(*) FROM items \
             WHERE unique_name NOT IN (SELECT unique_name FROM item_names WHERE lang = 'ru')",
        )
        .await?;
        if ru_gap > t.ru_name_gap_budget {
            report.fail(format!(
                "items without a ru name: {ru_gap} > budget {}",
                t.ru_name_gap_budget
            ));
        }
    }

    let disposition_bad = count(
        pool,
        "SELECT count(*) FROM weapon \
         WHERE NOT (omega_attenuation >= 0.5 AND omega_attenuation <= 1.55)",
    )
    .await?;
    if disposition_bad > 0 {
        report.fail(format!(
            "weapon disposition out of [0.5, 1.55]: {disposition_bad} row(s)"
        ));
    }

    let base_zero = count(
        pool,
        "SELECT count(*) FROM riven_attribute_base WHERE base_value = 0",
    )
    .await?;
    if base_zero > 0 {
        report.fail(format!(
            "riven_attribute_base with zero base_value: {base_zero} row(s)"
        ));
    }

    let base_uncovered = count(
        pool,
        "SELECT count(*) FROM riven_attribute_base b \
         LEFT JOIN riven_attribute a ON b.tag = a.tag WHERE a.tag IS NULL",
    )
    .await?;
    if base_uncovered > 0 {
        report.fail(format!(
            "riven_attribute_base tags without metadata: {base_uncovered} row(s)"
        ));
    }

    let name_uncovered = count(
        pool,
        "SELECT count(*) FROM riven_attribute_name n \
         LEFT JOIN riven_attribute a ON n.tag = a.tag WHERE a.tag IS NULL",
    )
    .await?;
    if name_uncovered > 0 {
        report.fail(format!(
            "riven_attribute_name tags without metadata: {name_uncovered} row(s)"
        ));
    }

    match meta_value(pool, "quality").await? {
        None => report.fail("meta['quality'] is missing"),
        Some(raw) => {
            let q: crate::quality::Quality = serde_json::from_str(&raw)
                .map_err(|e| anyhow::anyhow!("parse meta['quality']: {e}"))?;
            if !q.unknown_sections.is_empty() {
                report.fail(format!(
                    "unknown droptable sections: {:?}",
                    q.unknown_sections
                ));
            }
            if q.relics_total > 0 {
                let effective = q.effective_resolve_ratio();
                if effective < t.relic_resolve_min {
                    report.fail(format!(
                        "effective relic resolve {effective:.4} < min {}",
                        t.relic_resolve_min
                    ));
                }
                let raw = q.relic_resolve_ratio();
                if raw < t.relic_resolve_min {
                    report.warn(format!(
                        "raw relic resolve {raw:.4} below {}",
                        t.relic_resolve_min
                    ));
                }
            }
            if q.reward_name_collisions > t.reward_collision_budget {
                report.warn(format!(
                    "reward-name collisions {} > budget {}",
                    q.reward_name_collisions, t.reward_collision_budget
                ));
            }
            if q.drops_unresolved_genuine > t.genuine_unresolved_budget {
                report.warn(format!(
                    "genuine unresolved drops {} > budget {}",
                    q.drops_unresolved_genuine, t.genuine_unresolved_budget
                ));
            }
        }
    }

    Ok(report)
}

/// Run a single-column integer-count query.
async fn count(pool: &SqlitePool, sql: &str) -> anyhow::Result<i64> {
    let n: i64 = sqlx::query_scalar(sql)
        .fetch_one(pool)
        .await
        .map_err(|e| anyhow::anyhow!("query [{sql}]: {e}"))?;
    Ok(n)
}

/// Read a single `meta` value by key.
async fn meta_value(pool: &SqlitePool, key: &str) -> anyhow::Result<Option<String>> {
    let v: Option<String> = sqlx::query_scalar("SELECT value FROM meta WHERE key = ?")
        .bind(key)
        .fetch_optional(pool)
        .await
        .map_err(|e| anyhow::anyhow!("read meta[{key}]: {e}"))?;
    Ok(v)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::str::FromStr;

    use super::*;
    use crate::quality::{NameCoverage, Quality, TableCounts};
    use crate::schema::DDL_STATEMENTS;

    async fn mem_pool() -> SqlitePool {
        let options = SqliteConnectOptions::from_str("sqlite::memory:").unwrap();
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .min_connections(1)
            .connect_with(options)
            .await
            .unwrap();
        for stmt in DDL_STATEMENTS {
            sqlx::query(stmt).execute(&pool).await.unwrap();
        }
        pool
    }

    fn test_thresholds() -> Thresholds {
        Thresholds {
            min_items: 1,
            min_item_names: 1,
            min_relic_rewards: 1,
            min_item_drops: 1,
            min_drop_places: 1,
            min_place_names: 1,
            min_weapon: 1,
            min_riven_attribute: 1,
            min_riven_attribute_base: 1,
            ru_name_gap_budget: 0,
            relic_resolve_min: 0.98,
            overshoot_max: 1.02,
            reward_collision_budget: 0,
            genuine_unresolved_budget: 0,
        }
    }

    fn healthy_quality_json() -> String {
        let mut name_coverage = BTreeMap::new();
        name_coverage.insert(
            "en".to_string(),
            NameCoverage {
                items_total: 3,
                named: 3,
            },
        );
        let q = Quality {
            de_index_hash: "test.s3".to_string(),
            built_at_ms: 0,
            tables: TableCounts {
                items: 3,
                item_names: 3,
                relic_rewards: 1,
                item_drops: 1,
                drop_places: 1,
                place_names: 1,
                weapon: 1,
                riven_attribute: 1,
                riven_attribute_base: 1,
                riven_attribute_name: 1,
            },
            relics_total: 1,
            relics_resolved: 1,
            relics_unresolved_expected: 0,
            relics_unresolved_genuine: 0,
            drops_unresolved_expected: 0,
            drops_unresolved_genuine: 0,
            drops_zero_chance: 0,
            name_collisions: 0,
            reward_name_collisions: 0,
            unknown_sections: vec![],
            name_coverage,
        };
        serde_json::to_string(&q).unwrap()
    }

    async fn insert_meta(pool: &SqlitePool, key: &str, value: &str) {
        sqlx::query("INSERT INTO meta (key, value) VALUES (?, ?)")
            .bind(key)
            .bind(value)
            .execute(pool)
            .await
            .unwrap();
    }

    async fn insert_healthy(pool: &SqlitePool) {
        for (uid, cat) in [
            ("/Relic/AxiA1", "relic"),
            ("/Reward/AkstilettoBarrel", "part"),
            ("/Item/Seer", "weapon"),
        ] {
            sqlx::query("INSERT INTO items (unique_name, category, tradable) VALUES (?, ?, 0)")
                .bind(uid)
                .bind(cat)
                .execute(pool)
                .await
                .unwrap();
            sqlx::query(
                "INSERT INTO item_names (unique_name, lang, source, name) VALUES (?, 'en', 'DE', ?)",
            )
            .bind(uid)
            .bind(uid)
            .execute(pool)
            .await
            .unwrap();
        }
        sqlx::query(
            "INSERT INTO drop_places (place_ref, kind) VALUES ('node:Mercury/Tolstoj', 'node')",
        )
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO place_names (place_ref, lang, name) \
             VALUES ('node:Mercury/Tolstoj', 'en', 'Mercury/Tolstoj')",
        )
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO relic_rewards \
             (relic_unique_name, reward_unique_name, refinement, rarity, chance) \
             VALUES ('/Relic/AxiA1', '/Reward/AkstilettoBarrel', 'intact', 'Uncommon', 0.11)",
        )
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO item_drops \
             (item_unique_name, place_ref, rotation, stage, rarity, chance, source) \
             VALUES ('/Item/Seer', 'node:Mercury/Tolstoj', NULL, NULL, 'Common', 0.38, 'missionRewards')",
        )
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO weapon (unique_name, weapon_type, omega_attenuation) \
             VALUES ('/Item/Seer', 'Pistols', 1.0)",
        )
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO riven_attribute (tag, prefix_tag, suffix_tag, unit) \
             VALUES ('WeaponCritChanceMod', 'crita', 'cron', 'percent')",
        )
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO riven_attribute_base (riven_class, tag, base_value) \
             VALUES ('pistol', 'WeaponCritChanceMod', 0.0233)",
        )
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO riven_attribute_name (tag, lang, name) \
             VALUES ('WeaponCritChanceMod', 'en', 'Critical Chance')",
        )
        .execute(pool)
        .await
        .unwrap();
        insert_meta(pool, "langs", "en").await;
        insert_meta(pool, "quality", &healthy_quality_json()).await;
    }

    #[tokio::test]
    async fn healthy_catalog_passes() {
        let pool = mem_pool().await;
        insert_healthy(&pool).await;
        let report = check(&pool, &test_thresholds()).await.unwrap();
        assert!(
            report.passed(),
            "unexpected failures: {:?}",
            report.failures
        );
    }

    #[tokio::test]
    async fn dangling_drop_item_fails() {
        let pool = mem_pool().await;
        insert_healthy(&pool).await;
        sqlx::query(
            "INSERT INTO item_drops (item_unique_name, place_ref, rarity, chance, source) \
             VALUES ('/Item/Ghost', 'node:Mercury/Tolstoj', 'Common', 0.5, 'missionRewards')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let report = check(&pool, &test_thresholds()).await.unwrap();
        assert!(!report.passed());
        assert!(
            report
                .failures
                .iter()
                .any(|f| f.contains("item_unique_name"))
        );
    }

    #[tokio::test]
    async fn chance_overshoot_fails() {
        let pool = mem_pool().await;
        insert_healthy(&pool).await;
        sqlx::query(
            "INSERT INTO relic_rewards \
             (relic_unique_name, reward_unique_name, refinement, rarity, chance) \
             VALUES ('/Relic/AxiA1', '/Item/Seer', 'intact', 'Common', 0.95)",
        )
        .execute(&pool)
        .await
        .unwrap();
        let report = check(&pool, &test_thresholds()).await.unwrap();
        assert!(!report.passed());
        assert!(report.failures.iter().any(|f| f.contains("overshoot")));
    }

    #[tokio::test]
    async fn item_without_en_name_fails() {
        let pool = mem_pool().await;
        insert_healthy(&pool).await;
        sqlx::query("INSERT INTO items (unique_name, category, tradable) VALUES ('/Item/Nameless', 'other', 0)")
            .execute(&pool)
            .await
            .unwrap();
        let report = check(&pool, &test_thresholds()).await.unwrap();
        assert!(!report.passed());
        assert!(
            report
                .failures
                .iter()
                .any(|f| f.contains("without an en name"))
        );
    }

    #[tokio::test]
    async fn empty_relic_rewards_fails_floor() {
        let pool = mem_pool().await;
        insert_healthy(&pool).await;
        sqlx::query("DELETE FROM relic_rewards")
            .execute(&pool)
            .await
            .unwrap();
        let report = check(&pool, &test_thresholds()).await.unwrap();
        assert!(!report.passed());
        assert!(report.failures.iter().any(|f| f.contains("relic_rewards")));
    }

    #[tokio::test]
    async fn disposition_out_of_range_fails() {
        let pool = mem_pool().await;
        insert_healthy(&pool).await;
        sqlx::query(
            "INSERT INTO weapon (unique_name, weapon_type, omega_attenuation) \
             VALUES ('/W/Bad', 'LongGuns', 2.0)",
        )
        .execute(&pool)
        .await
        .unwrap();
        let report = check(&pool, &test_thresholds()).await.unwrap();
        assert!(!report.passed());
        assert!(report.failures.iter().any(|f| f.contains("disposition")));
    }

    #[tokio::test]
    async fn uncovered_riven_base_tag_fails() {
        let pool = mem_pool().await;
        insert_healthy(&pool).await;
        sqlx::query(
            "INSERT INTO riven_attribute_base (riven_class, tag, base_value) \
             VALUES ('rifle', 'WeaponUnknownMod', 0.01)",
        )
        .execute(&pool)
        .await
        .unwrap();
        let report = check(&pool, &test_thresholds()).await.unwrap();
        assert!(!report.passed());
        assert!(
            report
                .failures
                .iter()
                .any(|f| f.contains("without metadata"))
        );
    }
}
