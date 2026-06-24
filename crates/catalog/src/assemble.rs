use std::collections::{HashMap, HashSet};

use wf_fetch::{IndexEntry, ManifestSource, index_hash, items_from_manifest};

use crate::config::Config;
use crate::write::{CatalogData, CatalogRow, NameRow, SetMemberRow};
use crate::{category, joins, wfm_bridge};

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

    let mut items: HashMap<String, CatalogRow> = HashMap::new();
    let mut names: Vec<NameRow> = Vec::new();

    for mname in &config.build.item_manifests {
        let mname = mname.as_str();
        let Some(entry) = en_index.iter().find(|e| e.manifest == mname) else {
            continue;
        };
        eprintln!("fetching {mname} (en)");
        let value = source.fetch_manifest(entry)?;
        for raw in items_from_manifest(&value) {
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
                name: raw.name,
            });
        }
    }

    eprintln!("fetching ExportRecipes (en)");
    let recipes = fetch_named(source, &en_index, "ExportRecipes")?;
    eprintln!("fetching ExportManifest (en)");
    let manifest = fetch_named(source, &en_index, "ExportManifest")?;
    let ducats = joins::ducat_map(&recipes);
    let icons = joins::icon_map(&manifest);
    let blueprint_of = joins::component_blueprint_map(&recipes);
    for (unique_name, row) in items.iter_mut() {
        row.ducat = ducats.get(unique_name).copied();
        row.icon = icons.get(unique_name).cloned();
    }

    eprintln!("fetching WFM items");
    let bridge = wfm_bridge::fetch_bridge(wfm_agent, &config.endpoints.wfm_items_url)?;

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
        }
    }
    tracing::info!(items = items.len(), bridged, "wfm bridge applied");

    for lang in langs.iter().filter(|l| l.as_str() != "en") {
        eprintln!("fetching {lang} index");
        let index = source.fetch_index(lang)?;
        for mname in &config.build.item_manifests {
            let mname = mname.as_str();
            let Some(entry) = index.iter().find(|e| e.manifest == mname) else {
                continue;
            };
            eprintln!("fetching {mname} ({lang})");
            let value = source.fetch_manifest(entry)?;
            for raw in items_from_manifest(&value) {
                let category =
                    category::derive_category(&config.categories, mname, &raw.unique_name);
                let key = catalog_key(&category, &raw.unique_name);
                if items.contains_key(&key) {
                    names.push(NameRow {
                        unique_name: key,
                        lang: lang.clone(),
                        source: "DE",
                        name: raw.name,
                    });
                }
            }
        }
    }

    // Synthetic trade sets + their members (ADR-0056).
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

    Ok(CatalogData {
        items,
        names,
        set_members,
        de_index_hash,
        langs,
        built_at_ms,
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_forces_en_first_and_dedups() {
        assert_eq!(
            normalize_langs(&["ru".into(), "EN".into(), "ru".into()]),
            vec!["en", "ru"]
        );
        assert_eq!(normalize_langs(&[]), vec!["en"]);
    }
}
