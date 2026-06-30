use std::path::Path;

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

use crate::quality::Quality;
use crate::schema::{DDL_STATEMENTS, SCHEMA_VERSION};

/// One catalog item row.
pub struct CatalogRow {
    /// DE stable key (primary key).
    pub unique_name: String,
    /// Derived category value.
    pub category: String,
    /// Ducat value, if joined from a prime recipe.
    pub ducat: Option<i64>,
    /// WFM trade slug, if bridged.
    pub wfm_url_name: Option<String>,
    /// Tradable on WFM (derived from a successful bridge).
    pub tradable: bool,
    /// Raw DE texture location, if joined.
    pub icon: Option<String>,
}

/// One localized-name row.
pub struct NameRow {
    /// DE stable key this name belongs to.
    pub unique_name: String,
    /// BCP-ish language tag (`en`, `ru`, ...).
    pub lang: String,
    /// Name origin: `DE` or `WFM`.
    pub source: &'static str,
    /// Display name.
    pub name: String,
}

/// One set-membership row: a synthetic set and one of its member parts.
pub struct SetMemberRow {
    /// Synthetic set key (`median:set:<base>`).
    pub set_unique_name: String,
    /// Member part's catalog key.
    pub member_unique_name: String,
    /// How many of the member the set contains.
    pub count: i64,
}

/// One relic reward row at a given refinement.
pub struct RelicRewardRow {
    /// Relic catalog key.
    pub relic_unique_name: String,
    /// Rewarded item catalog key.
    pub reward_unique_name: String,
    /// Refinement token (`intact`/`exceptional`/`flawless`/`radiant`).
    pub refinement: String,
    /// Rarity label as printed by the source.
    pub rarity: String,
    /// Drop probability in `[0, 1]`.
    pub chance: f64,
}

/// One item-drop row tying an item to a place.
pub struct ItemDropRow {
    /// Dropped item catalog key.
    pub item_unique_name: String,
    /// Synthetic place reference (`<kind>:<display name>`).
    pub place_ref: String,
    /// Rotation letter when the place rotates.
    pub rotation: Option<String>,
    /// Stage label when the place is staged.
    pub stage: Option<String>,
    /// Rarity label as printed by the source.
    pub rarity: String,
    /// Drop probability in `[0, 1]`.
    pub chance: f64,
    /// Source section id this row came from.
    pub source: String,
}

/// One drop-place row.
pub struct DropPlaceRow {
    /// Synthetic place reference.
    pub place_ref: String,
    /// Place kind (`node`/`key`/`sortie`/`bounty`/`transient`/`enemy`).
    pub kind: String,
}

/// One localized place-name row.
pub struct PlaceNameRow {
    /// Synthetic place reference.
    pub place_ref: String,
    /// Language tag.
    pub lang: String,
    /// Place display name.
    pub name: String,
}

/// One weapon row: the disposition satellite.
pub struct WeaponRow {
    /// DE stable key (primary key).
    pub unique_name: String,
    /// DE `productCategory` (e.q. `LongGuns`).
    pub weapon_type: String,
    /// Riven disposition.
    pub omega_attenuation: f64,
}

/// One riven attribute row: per-tag display/grammar metadata.
pub struct RivenAttributeRow {
    /// DE stat tag (primary key).
    pub tag: String,
    /// Riven-name prefix syllable, if any.
    pub prefix_tag: Option<String>,
    /// Riven-name suffix syllable, if any.
    pub suffix_tag: Option<String>,
    /// Display unit (`percent`/`flat`).
    pub unit: String,
}

/// One riven attribute base value for a given riven class.
pub struct RivenAttributeBaseRow {
    /// Riven class token (`rifle`, `shotgun`, ...).
    pub riven_class: String,
    /// DE stat tag.
    pub tag: String,
    /// Base value at the riven template's reference.
    pub base_value: f64,
}

/// One localized riven attribute name.
pub struct RivenAttributeNameRow {
    /// DE stat tag.
    pub tag: String,
    /// Language tag (`en`/`ru`).
    pub lang: String,
    /// Localized stat name.
    pub name: String,
}

/// An assembled catalog ready to be written to SQLite.
pub struct CatalogData {
    /// Item rows.
    pub items: Vec<CatalogRow>,
    /// Localized-name rows.
    pub names: Vec<NameRow>,
    /// Set-membership rows.
    pub set_members: Vec<SetMemberRow>,
    /// Relic reward rows (per refinement).
    pub relic_rewards: Vec<RelicRewardRow>,
    /// Item-drop rows.
    pub item_drops: Vec<ItemDropRow>,
    /// Distinct drop places.
    pub drop_places: Vec<DropPlaceRow>,
    /// Localized place names.
    pub place_names: Vec<PlaceNameRow>,
    /// Weapon disposition rows.
    pub weapons: Vec<WeaponRow>,
    /// Riven attribute metadata rows.
    pub riven_attributes: Vec<RivenAttributeRow>,
    /// Riven attribute base values per class.
    pub riven_attribute_bases: Vec<RivenAttributeBaseRow>,
    /// Localized riven attribute names.
    pub riven_attribute_names: Vec<RivenAttributeNameRow>,
    /// Identity of the DE index this catalog was built from.
    pub de_index_hash: String,
    /// Languages included (EN always present).
    pub langs: Vec<String>,
    /// Build time, unix epoch milliseconds.
    pub built_at_ms: i64,
    /// Quality snapshot for this build.
    pub quality: Quality,
}

/// Write the assembled catalog to a fresh SQLite file at `path` (replacing any existing one).
pub async fn write_catalog(path: &Path, data: &CatalogData) -> anyhow::Result<()> {
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await?;

    for stmt in DDL_STATEMENTS {
        sqlx::query(stmt).execute(&pool).await?;
    }

    let mut tx = pool.begin().await?;

    for item in &data.items {
        sqlx::query(
            "INSERT INTO items (unique_name, category, ducat, wfm_url_name, tradable, icon) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&item.unique_name)
        .bind(item.category.as_str())
        .bind(item.ducat)
        .bind(item.wfm_url_name.as_deref())
        .bind(item.tradable as i64)
        .bind(item.icon.as_deref())
        .execute(&mut *tx)
        .await?;
    }

    for name in &data.names {
        sqlx::query(
            "INSERT OR IGNORE INTO item_names (unique_name, lang, source, name) VALUES (?, ?, ?, ?)",
        )
        .bind(&name.unique_name)
        .bind(&name.lang)
        .bind(name.source)
        .bind(&name.name)
        .execute(&mut *tx)
        .await?;
    }

    for m in &data.set_members {
        sqlx::query(
            "INSERT OR IGNORE INTO set_members (set_unique_name, member_unique_name, count) \
             VALUES (?, ?, ?)",
        )
        .bind(&m.set_unique_name)
        .bind(&m.member_unique_name)
        .bind(m.count)
        .execute(&mut *tx)
        .await?;
    }

    for r in &data.relic_rewards {
        sqlx::query(
            "INSERT OR IGNORE INTO relic_rewards \
             (relic_unique_name, reward_unique_name, refinement, rarity, chance) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&r.relic_unique_name)
        .bind(&r.reward_unique_name)
        .bind(&r.refinement)
        .bind(&r.rarity)
        .bind(r.chance)
        .execute(&mut *tx)
        .await?;
    }

    for d in &data.item_drops {
        sqlx::query(
            "INSERT INTO item_drops \
             (item_unique_name, place_ref, rotation, stage, rarity, chance, source) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&d.item_unique_name)
        .bind(&d.place_ref)
        .bind(d.rotation.as_deref())
        .bind(d.stage.as_deref())
        .bind(&d.rarity)
        .bind(d.chance)
        .bind(&d.source)
        .execute(&mut *tx)
        .await?;
    }

    for p in &data.drop_places {
        sqlx::query("INSERT OR IGNORE INTO drop_places (place_ref, kind) VALUES (?, ?)")
            .bind(&p.place_ref)
            .bind(&p.kind)
            .execute(&mut *tx)
            .await?;
    }

    for p in &data.place_names {
        sqlx::query("INSERT OR IGNORE INTO place_names (place_ref, lang, name) VALUES (?, ?, ?)")
            .bind(&p.place_ref)
            .bind(&p.lang)
            .bind(&p.name)
            .execute(&mut *tx)
            .await?;
    }

    for w in &data.weapons {
        sqlx::query(
            "INSERT OR IGNORE INTO weapon (unique_name, weapon_type, omega_attenuation) \
             VALUES (?, ?, ?)",
        )
        .bind(&w.unique_name)
        .bind(&w.weapon_type)
        .bind(w.omega_attenuation)
        .execute(&mut *tx)
        .await?;
    }

    for a in &data.riven_attributes {
        sqlx::query(
            "INSERT OR IGNORE INTO riven_attribute (tag, prefix_tag, suffix_tag, unit) \
             VALUES (?, ?, ?, ?)",
        )
        .bind(&a.tag)
        .bind(a.prefix_tag.as_deref())
        .bind(a.suffix_tag.as_deref())
        .bind(&a.unit)
        .execute(&mut *tx)
        .await?;
    }

    for b in &data.riven_attribute_bases {
        sqlx::query(
            "INSERT OR IGNORE INTO riven_attribute_base (riven_class, tag, base_value) \
             VALUES (?, ?, ?)",
        )
        .bind(&b.riven_class)
        .bind(&b.tag)
        .bind(b.base_value)
        .execute(&mut *tx)
        .await?;
    }

    for n in &data.riven_attribute_names {
        sqlx::query(
            "INSERT OR IGNORE INTO riven_attribute_name (tag, lang, name) VALUES (?, ?, ?)",
        )
        .bind(&n.tag)
        .bind(&n.lang)
        .bind(&n.name)
        .execute(&mut *tx)
        .await?;
    }

    let meta = [
        ("schema_version", SCHEMA_VERSION.to_string()),
        ("built_at", data.built_at_ms.to_string()),
        ("de_index_hash", data.de_index_hash.clone()),
        ("langs", data.langs.join(",")),
        ("quality", serde_json::to_string(&data.quality)?),
    ];
    for (key, value) in meta {
        sqlx::query("INSERT INTO meta (key, value) VALUES (?, ?)")
            .bind(key)
            .bind(value)
            .execute(&mut *tx)
            .await?;
    }

    tx.commit().await?;
    pool.close().await;
    Ok(())
}
