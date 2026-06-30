use std::collections::{BTreeMap, HashMap, HashSet};

use wf_fetch::{
    DropTables, IndexEntry, ManifestSource, RawItem, RawRivenBase, RawWeapon, index_hash,
    items_from_manifest, riven_bases_from_manifest, weapons_from_manifest,
};

use crate::config::Config;
use crate::write::{
    CatalogData, CatalogRow, DropPlaceRow, ItemDropRow, NameRow, PlaceNameRow, RelicRewardRow,
    RivenAttributeBaseRow, RivenAttributeNameRow, RivenAttributeRow, SetMemberRow, WeaponRow,
};
use crate::{category, drop_bridge, joins, quality, wfm_bridge};

/// Per-manifest raw items for one language, keeping the source manifest name for categorization.
pub struct ManifestItems {
    pub manifest: String,
    pub items: Vec<RawItem>,
}

/// Network-free inputs to the pure assembler core.
pub struct AssembleInputs {
    /// DE index identity (already schema-versioned).
    pub de_index_hash: String,
    /// EN item manifests in config order.
    pub en_items: Vec<ManifestItems>,
    /// Secondary-language item manifests, keyed by language tag.
    pub lang_items: BTreeMap<String, Vec<ManifestItems>>,
    /// `ExportRecipes` document (ducat + blueprint joins).
    pub recipes: serde_json::Value,
    /// `ExportManifest` document (icon join).
    pub manifest: serde_json::Value,
    /// WFM bridge (trade slugs, sets, English names).
    pub bridge: wfm_bridge::WfmBridge,
    /// Parsed DE drop tables.
    pub drop_tables: DropTables,
    /// Weapons with disposition (from `ExportWeapons`).
    pub weapons: Vec<RawWeapon>,
    /// Riven attribute base values per class (from `ExportUpgrades` templates).
    pub riven_bases: Vec<RawRivenBase>,
}

/// Fetch DE + WFM data and assemble the in-memory catalog for `langs` (EN is always included first).
pub fn assemble_catalog(
    source: &dyn ManifestSource,
    wfm_agent: &ureq::Agent,
    config: &Config,
    langs: &[String],
    built_at_ms: i64,
) -> anyhow::Result<CatalogData> {
    let langs = normalize_langs(langs);

    let en_index = source.fetch_index("en")?;
    let de_index_hash = crate::schema::catalog_version(&index_hash(&en_index));

    let mut en_items = Vec::new();
    let mut weapons = Vec::new();
    let mut riven_bases = Vec::new();
    for mname in &config.build.item_manifests {
        let Some(entry) = en_index.iter().find(|e| e.manifest == mname.as_str()) else {
            continue;
        };
        eprintln!("fetching {mname} (en)");
        let value = source.fetch_manifest(entry)?;
        if mname.as_str() == "ExportWeapons" {
            weapons = weapons_from_manifest(&value);
        } else if mname.as_str() == "ExportUpgrades" {
            riven_bases = riven_bases_from_manifest(&value);
        }
        en_items.push(ManifestItems {
            manifest: mname.clone(),
            items: items_from_manifest(&value),
        });
    }

    eprintln!("fetching ExportRecipes (en)");
    let recipes = fetch_named(source, &en_index, "ExportRecipes")?;
    eprintln!("fetching ExportManifest (en)");
    let manifest = fetch_named(source, &en_index, "ExportManifest")?;

    eprintln!("fetching WFM items");
    let bridge = wfm_bridge::fetch_bridge(wfm_agent, &config.endpoints.wfm_items_url)?;

    let mut lang_items: BTreeMap<String, Vec<ManifestItems>> = BTreeMap::new();
    for lang in langs.iter().filter(|l| l.as_str() != "en") {
        eprintln!("fetching {lang} index");
        let index = source.fetch_index(lang)?;
        let mut mis = Vec::new();
        for mname in &config.build.item_manifests {
            let Some(entry) = index.iter().find(|e| e.manifest == mname.as_str()) else {
                continue;
            };
            eprintln!("fetching {mname} ({lang})");
            let value = source.fetch_manifest(entry)?;
            mis.push(ManifestItems {
                manifest: mname.clone(),
                items: items_from_manifest(&value),
            });
        }
        lang_items.insert(lang.clone(), mis);
    }

    eprintln!("fetching droptables");
    let html = wf_fetch::fetch_droptables(wfm_agent, &config.endpoints.droptables_url)?;
    let drop_tables = wf_fetch::parse_droptables(&html)?;

    let inputs = AssembleInputs {
        de_index_hash,
        en_items,
        lang_items,
        recipes,
        manifest,
        bridge,
        drop_tables,
        weapons,
        riven_bases,
    };
    Ok(assemble_from_parts(&inputs, config, &langs, built_at_ms))
}

/// Pure assembler core: build the catalog from already-fetched inputs (no network).
pub fn assemble_from_parts(
    inputs: &AssembleInputs,
    config: &Config,
    langs: &[String],
    built_at_ms: i64,
) -> CatalogData {
    let langs = normalize_langs(langs);
    let want_ru = langs.iter().any(|l| l.as_str() == "ru");
    let de_index_hash = inputs.de_index_hash.clone();

    let mut items: HashMap<String, CatalogRow> = HashMap::new();
    let mut names: Vec<NameRow> = Vec::new();

    for mi in &inputs.en_items {
        let mname = mi.manifest.as_str();
        for raw in &mi.items {
            let category = category::derive_category(&config.categories, mname, &raw.unique_name);
            let key = catalog_key(&category, &raw.unique_name);
            items.entry(key.clone()).or_insert_with(|| CatalogRow {
                unique_name: key.clone(),
                category,
                ducat: None,
                wfm_url_name: None,
                tradable: false,
                icon: None,
            });
            names.push(NameRow {
                unique_name: key,
                lang: "en".to_string(),
                source: "DE",
                name: raw.name.clone(),
            });
        }
    }

    let ducats = joins::ducat_map(&inputs.recipes);
    let icons = joins::icon_map(&inputs.manifest);
    let blueprint_of = joins::component_blueprint_map(&inputs.recipes);
    for (unique_name, row) in items.iter_mut() {
        row.ducat = ducats.get(unique_name).copied();
        row.icon = icons.get(unique_name).cloned();
    }

    let bridge = &inputs.bridge;

    // Assembled primes are the gameRef targets of trade sets; they stay tradable=0 (the set is the
    // tradable entity, added synthetically below) — so they are skipped by the regular bridge.
    let assembled_primes: HashSet<&str> = bridge
        .sets
        .iter()
        .filter(|s| !s.game_ref.is_empty())
        .map(|s| s.game_ref.as_str())
        .collect();

    let mut bridged = 0usize;
    for (unique_name, row) in items.iter_mut() {
        if assembled_primes.contains(unique_name.as_str()) {
            continue;
        }
        let matched = bridge.by_game_ref.get(unique_name).or_else(|| {
            blueprint_of
                .get(unique_name)
                .and_then(|blueprint| bridge.by_game_ref.get(blueprint))
        });
        if let Some(entry) = matched {
            row.tradable = true;
            row.wfm_url_name = Some(entry.url_name.clone());
            bridged += 1;
            if let Some(en_name) = &entry.en_name {
                names.push(NameRow {
                    unique_name: unique_name.clone(),
                    lang: "en".to_string(),
                    source: "WFM",
                    name: en_name.clone(),
                });
            }
            if want_ru {
                if let Some(ru_name) = &entry.ru_name {
                    names.push(NameRow {
                        unique_name: unique_name.clone(),
                        lang: "ru".to_string(),
                        source: "WFM",
                        name: ru_name.clone(),
                    });
                }
            }
        }
    }
    tracing::info!(items = items.len(), bridged, "wfm bridge applied");

    for lang in langs.iter().filter(|l| l.as_str() != "en") {
        let Some(manifests) = inputs.lang_items.get(lang) else {
            continue;
        };
        for mi in manifests {
            let mname = mi.manifest.as_str();
            for raw in &mi.items {
                let category =
                    category::derive_category(&config.categories, mname, &raw.unique_name);
                let key = catalog_key(&category, &raw.unique_name);
                if items.contains_key(&key) {
                    names.push(NameRow {
                        unique_name: key,
                        lang: lang.clone(),
                        source: "DE",
                        name: raw.name.clone(),
                    });
                }
            }
        }
    }

    // Synthetic trade sets + their members.
    let mut set_members: Vec<SetMemberRow> = Vec::new();
    let reverse: HashMap<String, String> = items
        .values()
        .filter_map(|row| {
            row.wfm_url_name
                .clone()
                .map(|slug| (slug, row.unique_name.clone()))
        })
        .collect();

    for set in &bridge.sets {
        let set_key = format!("median:set:{}", set.base);
        items.entry(set_key.clone()).or_insert_with(|| CatalogRow {
            unique_name: set_key.clone(),
            category: "set".to_string(),
            ducat: None,
            wfm_url_name: Some(set.slug.clone()),
            tradable: true,
            icon: None,
        });
        if let Some(en) = &set.en_name {
            names.push(NameRow {
                unique_name: set_key.clone(),
                lang: "en".to_string(),
                source: "WFM",
                name: en.clone(),
            });
        }
        if want_ru {
            if let Some(ru) = &set.ru_name {
                names.push(NameRow {
                    unique_name: set_key.clone(),
                    lang: "ru".to_string(),
                    source: "WFM",
                    name: ru.clone(),
                });
            }
        }
        for member in &set.members {
            let member_key = if let Some(existing) = reverse.get(&member.slug) {
                existing.clone()
            } else if !member.game_ref.is_empty() {
                let key = member.game_ref.clone();
                items.entry(key.clone()).or_insert_with(|| CatalogRow {
                    unique_name: key.clone(),
                    category: "part".to_string(),
                    ducat: None,
                    wfm_url_name: Some(member.slug.clone()),
                    tradable: true,
                    icon: None,
                });
                if let Some(en) = &member.en_name {
                    names.push(NameRow {
                        unique_name: key.clone(),
                        lang: "en".to_string(),
                        source: "WFM",
                        name: en.clone(),
                    });
                }
                if want_ru {
                    if let Some(ru) = &member.ru_name {
                        names.push(NameRow {
                            unique_name: key.clone(),
                            lang: "ru".to_string(),
                            source: "WFM",
                            name: ru.clone(),
                        });
                    }
                }
                key
            } else {
                continue;
            };
            set_members.push(SetMemberRow {
                set_unique_name: set_key.clone(),
                member_unique_name: member_key,
                count: 1,
            });
        }
    }

    // Drop / relic source tables: bridge display names from the parsed DE drop tables.
    let name_index = drop_bridge::NameIndex::build(&names);
    let dt = &inputs.drop_tables;

    let mut relic_rewards: Vec<RelicRewardRow> = Vec::new();
    let mut relics_resolved = 0usize;
    let mut relic_unresolved = 0usize;
    let mut relics_unresolved_expected = 0usize;
    let mut relics_unresolved_genuine = 0usize;
    let mut reward_name_collisions = 0usize;
    for r in &dt.relics {
        if r.chance <= 0.0 {
            continue;
        }
        let relic_uid = name_index.resolve(&r.relic_name);
        let reward_uid = name_index.resolve(&r.reward_name);
        let (Some(relic_uid), Some(reward_uid)) = (relic_uid, reward_uid) else {
            relic_unresolved += 1;
            if relic_uid.is_some() && is_expected_unresolved(&r.reward_name) {
                relics_unresolved_expected += 1;
            } else {
                relics_unresolved_genuine += 1;
            }
            continue;
        };
        if name_index.is_colliding(&r.reward_name) || name_index.is_colliding(&r.relic_name) {
            reward_name_collisions += 1;
        }
        relics_resolved += 1;
        relic_rewards.push(RelicRewardRow {
            relic_unique_name: relic_uid.to_string(),
            reward_unique_name: reward_uid.to_string(),
            refinement: r.refinement.as_str().to_string(),
            rarity: r.rarity.clone(),
            chance: r.chance,
        });
    }

    let mut item_drops: Vec<ItemDropRow> = Vec::new();
    let mut place_kinds: HashMap<String, &'static str> = HashMap::new();
    let mut place_labels: HashMap<String, String> = HashMap::new();
    let mut drop_unresolved = 0usize;
    let mut drops_unresolved_expected = 0usize;
    let mut drops_unresolved_genuine = 0usize;
    let mut drops_zero_chance = 0usize;
    for d in &dt.drops {
        if d.chance <= 0.0 {
            drops_zero_chance += 1;
            continue;
        }
        let Some(item_uid) = name_index.resolve(&d.item_name) else {
            drop_unresolved += 1;
            if is_expected_unresolved(&d.item_name) {
                drops_unresolved_expected += 1;
            } else {
                drops_unresolved_genuine += 1;
            }
            continue;
        };
        let kind = d.place_kind.as_str();
        let place_ref = format!("{kind}:{}", d.place_name);
        place_kinds.entry(place_ref.clone()).or_insert(kind);
        place_labels
            .entry(place_ref.clone())
            .or_insert_with(|| d.place_name.clone());
        item_drops.push(ItemDropRow {
            item_unique_name: item_uid.to_string(),
            place_ref,
            rotation: d.rotation.clone(),
            stage: d.stage.clone(),
            rarity: d.rarity.clone(),
            chance: d.chance,
            source: d.source.clone(),
        });
    }

    let mut drop_places: Vec<DropPlaceRow> = place_kinds
        .into_iter()
        .map(|(place_ref, kind)| DropPlaceRow {
            place_ref,
            kind: kind.to_string(),
        })
        .collect();
    let mut place_names: Vec<PlaceNameRow> = place_labels
        .into_iter()
        .map(|(place_ref, name)| PlaceNameRow {
            place_ref,
            lang: "en".to_string(),
            name,
        })
        .collect();

    tracing::info!(
        relics = relic_rewards.len(),
        relic_unresolved,
        relics_unresolved_genuine,
        reward_name_collisions,
        drops = item_drops.len(),
        drop_unresolved,
        drops_unresolved_expected,
        drops_unresolved_genuine,
        drops_zero_chance,
        places = drop_places.len(),
        name_collisions = name_index.collisions(),
        "drop tables assembled"
    );

    let mut items: Vec<CatalogRow> = items.into_values().collect();
    items.sort_by(|a, b| a.unique_name.cmp(&b.unique_name));
    names.sort_by(|a, b| {
        (&a.unique_name, &a.lang, a.source).cmp(&(&b.unique_name, &b.lang, b.source))
    });
    set_members.sort_by(|a, b| {
        (&a.set_unique_name, &a.member_unique_name)
            .cmp(&(&b.set_unique_name, &b.member_unique_name))
    });
    set_members.dedup_by(|a, b| {
        a.set_unique_name == b.set_unique_name && a.member_unique_name == b.member_unique_name
    });
    relic_rewards.sort_by(|a, b| {
        (&a.relic_unique_name, &a.reward_unique_name, &a.refinement).cmp(&(
            &b.relic_unique_name,
            &b.reward_unique_name,
            &b.refinement,
        ))
    });
    item_drops.sort_by(|a, b| {
        a.item_unique_name
            .cmp(&b.item_unique_name)
            .then_with(|| a.place_ref.cmp(&b.place_ref))
            .then_with(|| a.rotation.cmp(&b.rotation))
            .then_with(|| a.stage.cmp(&b.stage))
            .then_with(|| a.source.cmp(&b.source))
            .then_with(|| a.rarity.cmp(&b.rarity))
            .then_with(|| a.chance.total_cmp(&b.chance))
    });
    // Collapse only byte-identical edges (the source occasionally lists one twice); distinct
    // place/rotation/chance rows for the same item are kept.
    item_drops.dedup_by(|a, b| {
        a.item_unique_name == b.item_unique_name
            && a.place_ref == b.place_ref
            && a.rotation == b.rotation
            && a.stage == b.stage
            && a.source == b.source
            && a.rarity == b.rarity
            && a.chance == b.chance
    });
    drop_places.sort_by(|a, b| a.place_ref.cmp(&b.place_ref));
    place_names.sort_by(|a, b| a.place_ref.cmp(&b.place_ref));

    let mut weapons: Vec<WeaponRow> = inputs
        .weapons
        .iter()
        .map(|w| WeaponRow {
            unique_name: w.unique_name.clone(),
            weapon_type: w.weapon_type.clone(),
            omega_attenuation: w.omega_attenuation,
        })
        .collect();
    weapons.sort_by(|a, b| a.unique_name.cmp(&b.unique_name));
    weapons.dedup_by(|a, b| a.unique_name == b.unique_name);

    let mut riven_attribute_bases: Vec<RivenAttributeBaseRow> = inputs
        .riven_bases
        .iter()
        .map(|b| RivenAttributeBaseRow {
            riven_class: b.riven_class.clone(),
            tag: b.tag.clone(),
            base_value: b.base_value,
        })
        .collect();
    riven_attribute_bases.sort_by(|a, b| (&a.riven_class, &a.tag).cmp(&(&b.riven_class, &b.tag)));
    riven_attribute_bases.dedup_by(|a, b| a.riven_class == b.riven_class && a.tag == b.tag);

    let mut riven_attributes: Vec<RivenAttributeRow> = Vec::new();
    let mut riven_attribute_names: Vec<RivenAttributeNameRow> = Vec::new();
    for a in &config.riven_attributes.attributes {
        riven_attributes.push(RivenAttributeRow {
            tag: a.tag.clone(),
            prefix_tag: non_empty(&a.prefix_tag),
            suffix_tag: non_empty(&a.suffix_tag),
            unit: a.unit.clone(),
        });
        riven_attribute_names.push(RivenAttributeNameRow {
            tag: a.tag.clone(),
            lang: "en".to_string(),
            name: a.name_en.clone(),
        });
        if want_ru && !a.name_ru.trim().is_empty() {
            riven_attribute_names.push(RivenAttributeNameRow {
                tag: a.tag.clone(),
                lang: "ru".to_string(),
                name: a.name_ru.clone(),
            });
        }
    }
    riven_attributes.sort_by(|a, b| a.tag.cmp(&b.tag));
    riven_attributes.dedup_by(|a, b| a.tag == b.tag);
    riven_attribute_names.sort_by(|a, b| (&a.tag, &a.lang).cmp(&(&b.tag, &b.lang)));

    let tables = quality::TableCounts {
        items: items.len() as u64,
        item_names: names.len() as u64,
        relic_rewards: relic_rewards.len() as u64,
        item_drops: item_drops.len() as u64,
        drop_places: drop_places.len() as u64,
        place_names: place_names.len() as u64,
    };

    let item_keys: HashSet<&str> = items.iter().map(|i| i.unique_name.as_str()).collect();
    let mut name_coverage: BTreeMap<String, quality::NameCoverage> = BTreeMap::new();
    for lang in &langs {
        name_coverage.insert(
            lang.clone(),
            quality::NameCoverage {
                items_total: items.len() as u64,
                named: 0,
            },
        );
    }
    let mut seen_named: HashSet<(&str, &str)> = HashSet::new();
    for n in &names {
        if !item_keys.contains(n.unique_name.as_str()) {
            continue;
        }
        if name_coverage.contains_key(&n.lang)
            && seen_named.insert((n.lang.as_str(), n.unique_name.as_str()))
        {
            if let Some(cov) = name_coverage.get_mut(&n.lang) {
                cov.named += 1;
            }
        }
    }

    let quality = quality::Quality {
        de_index_hash: de_index_hash.clone(),
        built_at_ms,
        tables,
        relics_total: dt.relics.len() as u64,
        relics_resolved: relics_resolved as u64,
        relics_unresolved_expected: relics_unresolved_expected as u64,
        relics_unresolved_genuine: relics_unresolved_genuine as u64,
        drops_unresolved_expected: drops_unresolved_expected as u64,
        drops_unresolved_genuine: drops_unresolved_genuine as u64,
        drops_zero_chance: drops_zero_chance as u64,
        name_collisions: name_index.collisions() as u64,
        reward_name_collisions: reward_name_collisions as u64,
        unknown_sections: dt.unknown_sections.clone(),
        name_coverage,
    };

    CatalogData {
        items,
        names,
        set_members,
        relic_rewards,
        item_drops,
        drop_places,
        place_names,
        weapons,
        riven_attributes,
        riven_attribute_bases,
        riven_attribute_names,
        de_index_hash,
        langs,
        built_at_ms,
        quality,
    }
}

/// Normalize a requested language list: lowercase, de-duplicated, with `en` forced first.
fn normalize_langs(langs: &[String]) -> Vec<String> {
    let mut out = vec!["en".to_string()];
    for l in langs {
        let l = l.trim().to_lowercase();
        if !l.is_empty() && !out.contains(&l) {
            out.push(l);
        }
    }
    out
}

/// Fetch and parse a manifest by name from an index.
fn fetch_named(
    source: &dyn ManifestSource,
    index: &[IndexEntry],
    name: &str,
) -> anyhow::Result<serde_json::Value> {
    let entry = index
        .iter()
        .find(|e| e.manifest == name)
        .ok_or_else(|| anyhow::anyhow!("manifest {name} not in index"))?;
    source.fetch_manifest(entry)
}

/// Relic refinement suffixes appended to relic uniqueNames (Intact/Exceptional/Flawless/Radiant).
const RELIC_REFINEMENTS: [&str; 4] = ["Bronze", "Silver", "Gold", "Platinum"];

/// Catalog key for a raw item: relics collapse to their base uniqueName.
fn catalog_key(category: &str, unique_name: &str) -> String {
    if category == "relic" {
        for suffix in RELIC_REFINEMENTS {
            if let Some(base) = unique_name.strip_suffix(suffix) {
                return base.to_string();
            }
        }
    }
    unique_name.to_string()
}

/// Map an empty/whitespace string to `None`, otherwise `Some`.
fn non_empty(s: &str) -> Option<String> {
    if s.trim().is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

/// Whether an unresolved reward/drop name is an expected non-catalog pickup (currency, Forma, endo).
///
/// Starting allowlist — extend from the `genuine` list surfaced by the first real builds.
fn is_expected_unresolved(display_name: &str) -> bool {
    let trimmed = display_name.trim();
    if trimmed.is_empty() {
        return true;
    }
    let first = trimmed.split_whitespace().next().unwrap_or_default();
    let qty = first.trim_end_matches(['x', 'X']);
    let numeric_prefix = !qty.is_empty()
        && qty.chars().any(|c| c.is_ascii_digit())
        && qty.chars().all(|c| c.is_ascii_digit() || c == ',');
    if numeric_prefix {
        return true;
    }
    let lower = trimmed.to_lowercase();
    matches!(
        lower.as_str(),
        "forma" | "forma blueprint" | "endo" | "kuva" | "credits"
    ) || lower.ends_with(" endo")
        || lower.ends_with(" credits")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{BuildConfig, Categories, Endpoints, RivenAttributeRule, RivenAttributes};
    use crate::wfm_bridge::{WfmBridge, WfmEntry, WfmMember, WfmSet};
    use wf_fetch::{ItemDrop, PlaceKind, Refinement, RelicReward};

    #[test]
    fn normalize_forces_en_first_and_dedups() {
        assert_eq!(
            normalize_langs(&["ru".into(), "EN".into(), "ru".into()]),
            vec!["en", "ru"]
        );
        assert_eq!(normalize_langs(&[]), vec!["en"]);
    }

    #[test]
    fn classifies_expected_vs_genuine_unresolved() {
        assert!(is_expected_unresolved("100 Endo"));
        assert!(is_expected_unresolved("15,000 Credits"));
        assert!(is_expected_unresolved("1000X Ducats"));
        assert!(is_expected_unresolved("10X Cryotic"));
        assert!(is_expected_unresolved("Forma Blueprint"));
        assert!(is_expected_unresolved("Endo"));
        assert!(!is_expected_unresolved("Akstiletto Prime Barrel"));
        assert!(!is_expected_unresolved("Kuva Bramma"));
    }

    fn golden_config() -> Config {
        Config {
            endpoints: Endpoints {
                de_index_base: "https://x/index_".into(),
                de_manifest_base: "https://x/Manifest/".into(),
                wfm_items_url: "https://x/items".into(),
                droptables_url: "https://x/droptables".into(),
                wfcd_relics_url: "https://x/relics".into(),
            },
            categories: Categories {
                default: "other".into(),
                manifests: vec![],
            },
            build: BuildConfig {
                langs: vec!["en".into(), "ru".into()],
                item_manifests: vec!["ExportWeapons".into()],
            },
            riven_attributes: RivenAttributes {
                attributes: vec![
                    RivenAttributeRule {
                        tag: "WeaponMeleeDamageMod".into(),
                        prefix_tag: "visi".into(),
                        suffix_tag: "ata".into(),
                        unit: "percent".into(),
                        name_en: "Melee Damage".into(),
                        name_ru: "Урон в ближнем бою".into(),
                    },
                    RivenAttributeRule {
                        tag: "WeaponCritChanceMod".into(),
                        prefix_tag: "crita".into(),
                        suffix_tag: "cron".into(),
                        unit: "percent".into(),
                        name_en: "Critical Chance".into(),
                        name_ru: String::new(),
                    },
                ],
            },
        }
    }

    fn golden_inputs() -> AssembleInputs {
        let en_items = vec![ManifestItems {
            manifest: "ExportWeapons".to_string(),
            items: vec![
                RawItem {
                    unique_name: "/Lotus/Relics/AxiA1".into(),
                    name: "Axi A1 Relic".into(),
                },
                RawItem {
                    unique_name: "/Lotus/Weapons/AkstilettoPrimeBarrel".into(),
                    name: "Akstiletto Prime Barrel".into(),
                },
                RawItem {
                    unique_name: "/Lotus/Weapons/NikanaPrime".into(),
                    name: "Nikana Prime".into(),
                },
            ],
        }];

        let mut lang_items = BTreeMap::new();
        lang_items.insert(
            "ru".to_string(),
            vec![ManifestItems {
                manifest: "ExportWeapons".to_string(),
                items: vec![RawItem {
                    unique_name: "/Lotus/Weapons/NikanaPrime".into(),
                    name: "Никана Прайм".into(),
                }],
            }],
        );

        let mut by_game_ref = HashMap::new();
        by_game_ref.insert(
            "/Lotus/Weapons/NikanaPrime".to_string(),
            WfmEntry {
                url_name: "nikana_prime".into(),
                en_name: Some("Nikana Prime".into()),
                ru_name: None,
            },
        );

        let bridge = WfmBridge {
            by_game_ref,
            sets: vec![WfmSet {
                base: "nikana_prime".into(),
                slug: "nikana_prime_set".into(),
                game_ref: "/Lotus/Weapons/NikanaPrimeSet".into(),
                en_name: Some("Nikana Prime Set".into()),
                ru_name: Some("Комплект Никана Прайм".into()),
                members: vec![WfmMember {
                    slug: "nikana_prime_blade".into(),
                    game_ref: "/Lotus/Weapons/NikanaPrimeBlade".into(),
                    en_name: Some("Nikana Prime Blade".into()),
                    ru_name: Some("Клинок Никана Прайм".into()),
                }],
            }],
        };

        let drop_tables = DropTables {
            relics: vec![RelicReward {
                relic_name: "Axi A1 Relic".into(),
                refinement: Refinement::Intact,
                reward_name: "Akstiletto Prime Barrel".into(),
                rarity: "Uncommon".into(),
                chance: 0.11,
            }],
            drops: vec![
                ItemDrop {
                    item_name: "Nikana Prime".into(),
                    place_name: "Mercury/Tolstoj (Assassination)".into(),
                    place_kind: PlaceKind::Node,
                    rotation: None,
                    stage: None,
                    rarity: "Common".into(),
                    chance: 0.20,
                    source: "missionRewards".into(),
                },
                ItemDrop {
                    item_name: "100 Endo".into(),
                    place_name: "Mercury/Apollodorus (Survival)".into(),
                    place_kind: PlaceKind::Node,
                    rotation: Some("A".into()),
                    stage: None,
                    rarity: "Common".into(),
                    chance: 0.5,
                    source: "missionRewards".into(),
                },
                ItemDrop {
                    item_name: "Akstiletto Prime Receiver".into(),
                    place_name: "Mercury/Apollodorus (Survival)".into(),
                    place_kind: PlaceKind::Node,
                    rotation: Some("B".into()),
                    stage: None,
                    rarity: "Rare".into(),
                    chance: 0.05,
                    source: "missionRewards".into(),
                },
            ],
            unknown_sections: vec![],
        };

        AssembleInputs {
            de_index_hash: "golden.s3".to_string(),
            en_items,
            lang_items,
            recipes: serde_json::json!({}),
            manifest: serde_json::json!({}),
            bridge,
            drop_tables,
            weapons: vec![RawWeapon {
                unique_name: "/Lotus/Weapons/NikanaPrime".into(),
                weapon_type: "Melee".into(),
                omega_attenuation: 1.35,
            }],
            riven_bases: vec![
                RawRivenBase {
                    riven_class: "melee".into(),
                    tag: "WeaponMeleeDamageMod".into(),
                    base_value: 1.65,
                },
                RawRivenBase {
                    riven_class: "melee".into(),
                    tag: "WeaponCritChanceMod".into(),
                    base_value: 2.7,
                },
            ],
        }
    }

    #[test]
    fn golden_assemble_from_parts() {
        let config = golden_config();
        let inputs = golden_inputs();
        let data = assemble_from_parts(&inputs, &config, &config.build.langs, 1234);

        assert_eq!(data.items.len(), 5);
        assert_eq!(data.langs, vec!["en", "ru"]);
        assert_eq!(data.de_index_hash, "golden.s3");

        assert_eq!(data.relic_rewards.len(), 1);
        let rr = &data.relic_rewards[0];
        assert_eq!(rr.relic_unique_name, "/Lotus/Relics/AxiA1");
        assert_eq!(
            rr.reward_unique_name,
            "/Lotus/Weapons/AkstilettoPrimeBarrel"
        );
        assert_eq!(rr.refinement, "intact");

        assert_eq!(data.item_drops.len(), 1);
        assert_eq!(
            data.item_drops[0].item_unique_name,
            "/Lotus/Weapons/NikanaPrime"
        );

        assert_eq!(data.set_members.len(), 1);
        assert_eq!(
            data.set_members[0].set_unique_name,
            "median:set:nikana_prime"
        );
        assert_eq!(
            data.set_members[0].member_unique_name,
            "/Lotus/Weapons/NikanaPrimeBlade"
        );

        let q = &data.quality;
        assert_eq!(q.tables.items, 5);
        assert_eq!(q.relics_total, 1);
        assert_eq!(q.relics_resolved, 1);
        assert_eq!(q.drops_unresolved_expected, 1);
        assert_eq!(q.drops_unresolved_genuine, 1);
        assert_eq!(q.relics_unresolved_genuine, 0);
        assert_eq!(q.drops_zero_chance, 0);
        assert_eq!(q.name_coverage["en"].named, 5);
        assert_eq!(q.name_coverage["en"].items_total, 5);
        assert_eq!(q.name_coverage["ru"].named, 3);
        assert_eq!(data.weapons.len(), 1);
        assert_eq!(data.weapons[0].unique_name, "/Lotus/Weapons/NikanaPrime");
        assert_eq!(data.weapons[0].weapon_type, "Melee");

        // sorted by (riven_class, tag): WeaponCritChanceMod precedes WeaponMeleeDamageMod
        assert_eq!(data.riven_attribute_bases.len(), 2);
        assert_eq!(data.riven_attribute_bases[0].tag, "WeaponCritChanceMod");
        assert_eq!(data.riven_attribute_bases[0].riven_class, "melee");

        assert_eq!(data.riven_attributes.len(), 2);
        let crit = data
            .riven_attributes
            .iter()
            .find(|a| a.tag == "WeaponCritChanceMod")
            .unwrap();
        assert_eq!(crit.prefix_tag.as_deref(), Some("crita"));
        assert_eq!(crit.unit, "percent");

        // en for both tags + ru only where name_ru is set (1 of 2) => 3
        assert_eq!(data.riven_attribute_names.len(), 3);
        let ru: Vec<_> = data
            .riven_attribute_names
            .iter()
            .filter(|n| n.lang == "ru")
            .collect();
        assert_eq!(ru.len(), 1);
        assert_eq!(ru[0].tag, "WeaponMeleeDamageMod");
    }
}
