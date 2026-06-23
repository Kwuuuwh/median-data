use std::collections::HashMap;

use wf_fetch::{IndexEntry, ManifestSource, index_hash, items_from_manifest};

use crate::write::{CatalogData, CatalogRow, NameRow};
use crate::{category, joins, wfm_bridge};

/// Manifests that contribute catalog items (each carries `uniqueName` + `name`).
const ITEM_MANIFESTS: &[&str] = &[
    "ExportResources",
    "ExportWeapons",
    "ExportWarframes",
    "ExportSentinels",
    "ExportUpgrades",
    "ExportRelicArcane",
];

/// Fetch DE + WFM data and assemble the in-memory catalog for `langs` (EN is always included first).
pub fn assemble_catalog(
    source: &dyn ManifestSource,
    wfm_agent: &ureq::Agent,
    langs: &[String],
    built_at_ms: i64,
) -> anyhow::Result<CatalogData> {
    let langs = normalize_langs(langs);

    let en_index = source.fetch_index("en")?;
    let de_index_hash = index_hash(&en_index);

    let mut items: HashMap<String, CatalogRow> = HashMap::new();
    let mut names: Vec<NameRow> = Vec::new();

    for mname in ITEM_MANIFESTS {
        let Some(entry) = en_index.iter().find(|e| &e.manifest == mname) else {
            continue;
        };
        eprintln!("fetching {mname} (en)");
        let value = source.fetch_manifest(entry)?;
        for raw in items_from_manifest(&value) {
            let category = category::derive_category(mname, &raw.unique_name);
            items
                .entry(raw.unique_name.clone())
                .or_insert_with(|| CatalogRow {
                    unique_name: raw.unique_name.clone(),
                    category,
                    ducat: None,
                    wfm_url_name: None,
                    tradable: false,
                    icon: None,
                });
            names.push(NameRow {
                unique_name: raw.unique_name,
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
    let bridge = wfm_bridge::fetch_bridge(wfm_agent)?;
    let mut bridged = 0usize;
    for (unique_name, row) in items.iter_mut() {
        let matched = bridge.get(unique_name).or_else(|| {
            blueprint_of
                .get(unique_name)
                .and_then(|blueprint| bridge.get(blueprint))
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
        for mname in ITEM_MANIFESTS {
            let Some(entry) = index.iter().find(|e| &e.manifest == mname) else {
                continue;
            };
            eprintln!("fetching {mname} ({lang})");
            let value = source.fetch_manifest(entry)?;
            for raw in items_from_manifest(&value) {
                if items.contains_key(&raw.unique_name) {
                    names.push(NameRow {
                        unique_name: raw.unique_name,
                        lang: lang.clone(),
                        source: "DE",
                        name: raw.name,
                    });
                }
            }
        }
    }

    let mut items: Vec<CatalogRow> = items.into_values().collect();
    items.sort_by(|a, b| a.unique_name.cmp(&b.unique_name));
    names.sort_by(|a, b| {
        (&a.unique_name, &a.lang, a.source).cmp(&(&b.unique_name, &b.lang, b.source))
    });

    Ok(CatalogData {
        items,
        names,
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
