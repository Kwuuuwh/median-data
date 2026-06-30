use crate::net::get_bytes;

/// A parsed line of the DE Public Export index.
#[derive(Debug, Clone)]
pub struct IndexEntry {
    /// Manifest base name, e.g. `ExportResources`.
    pub manifest: String,
    /// Full index token, e.g. `ExportResources_en.json!00_<hash>`.
    pub file: String,
    /// DE content-hash token, e.g. `00_<hash>`.
    pub hash: String,
}

/// A raw item extracted from a manifest: stable key and display name.
#[derive(Debug, Clone)]
pub struct RawItem {
    /// DE stable key.
    pub unique_name: String,
    /// Per-language display name.
    pub name: String,
}

/// Fetch and decode `index_<lang>.txt.lzma` (LZMA-alone) into parsed index entries.
pub fn fetch_index(
    agent: &ureq::Agent,
    index_base: &str,
    lang: &str,
) -> anyhow::Result<Vec<IndexEntry>> {
    let url = format!("{index_base}{lang}.txt.lzma");
    let compressed = get_bytes(agent, &url)?;
    let mut decoded = Vec::new();
    lzma_rs::lzma_decompress(&mut std::io::Cursor::new(&compressed), &mut decoded)?;
    let text = String::from_utf8_lossy(&decoded);
    let entries = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(parse_line)
        .collect();
    Ok(entries)
}

/// Parse one index line, stripping `.json` and an optional `_<lang>` from the manifest name.
fn parse_line(line: &str) -> IndexEntry {
    let (left, hash) = line.split_once('!').unwrap_or((line, ""));
    let stem = left.strip_suffix(".json").unwrap_or(left);
    let manifest = stem
        .rsplit_once('_')
        .map(|(base, _lang)| base)
        .unwrap_or(stem)
        .to_string();
    IndexEntry {
        manifest,
        file: line.to_string(),
        hash: hash.to_string(),
    }
}

/// Stable identity of the index: its per-manifest hash tokens joined.
pub fn index_hash(entries: &[IndexEntry]) -> String {
    entries
        .iter()
        .map(|e| e.hash.as_str())
        .collect::<Vec<_>>()
        .join(".")
}

/// Content URL for an index entry under `manifest_base`.
pub fn manifest_url(manifest_base: &str, entry: &IndexEntry) -> String {
    format!("{manifest_base}{}", entry.file)
}

/// Replace raw control bytes with spaces so `serde_json` accepts the manifest's strings.
pub fn sanitize_json(raw: &[u8]) -> String {
    let cleaned: Vec<u8> = raw
        .iter()
        .map(|&b| if b < 0x20 { b' ' } else { b })
        .collect();
    String::from_utf8_lossy(&cleaned).into_owned()
}

/// Fetch one manifest under `manifest_base`, sanitize control chars, and parse it to JSON.
pub fn fetch_manifest(
    agent: &ureq::Agent,
    manifest_base: &str,
    entry: &IndexEntry,
) -> anyhow::Result<serde_json::Value> {
    let raw = get_bytes(agent, &manifest_url(manifest_base, entry))?;
    let cleaned = sanitize_json(&raw);
    let value = serde_json::from_str(&cleaned)?;
    Ok(value)
}

/// Extract every object carrying both `uniqueName` and `name` from a manifest's arrays.
pub fn items_from_manifest(value: &serde_json::Value) -> Vec<RawItem> {
    let mut out = Vec::new();
    let Some(obj) = value.as_object() else {
        return out;
    };
    for v in obj.values() {
        let Some(arr) = v.as_array() else {
            continue;
        };
        for el in arr {
            let (Some(unique_name), Some(name)) = (
                el.get("uniqueName").and_then(|x| x.as_str()),
                el.get("name").and_then(|x| x.as_str()),
            ) else {
                continue;
            };
            if unique_name.is_empty() || name.is_empty() {
                continue;
            }
            out.push(RawItem {
                unique_name: unique_name.to_string(),
                name: name.to_string(),
            });
        }
    }
    out
}

/// A weapon's riven-relevant fields: stable key, product category, and disposition.
#[derive(Debug, Clone)]
pub struct RawWeapon {
    /// DE stable key.
    pub unique_name: String,
    /// DE `productCategory` (e.q. `LongGuns`, `Pistols`, `Melee`).
    pub weapon_type: String,
    /// Riven disposition (`omegaAttenuation`).
    pub omega_attenuation: f64,
}

/// One riven attribute's base value for a given riven class, from a DE riven template.
#[derive(Debug, Clone)]
pub struct RawRivenBase {
    /// Canonical riven class token (`rifle`, `shotgun`, ...).
    pub riven_class: String,
    /// DE stat tag (e.q. `WeaponCritChanceMod`).
    pub tag: String,
    /// Base value at the template's reference.
    pub base_value: f64,
}

/// DE riven-template uniqueNames that carry a class's full attribute pool, by class token.
const RIVEN_CLASS_TEMPLATES: [(&str, &str); 7] = [
    (
        "/Lotus/Upgrades/Mods/Randomized/LotusRifleRandomModRare",
        "rifle",
    ),
    (
        "/Lotus/Upgrades/Mods/Randomized/LotusShotgunRandomModRare",
        "shotgun",
    ),
    (
        "/Lotus/Upgrades/Mods/Randomized/LotusPistolRandomModRare",
        "pistol",
    ),
    (
        "/Lotus/Upgrades/Mods/Randomized/PlayerMeleeWeaponRandomModRare",
        "melee",
    ),
    (
        "/Lotus/Upgrades/Mods/Randomized/LotusArchgunRandomModRare",
        "archgun",
    ),
    (
        "/Lotus/Upgrades/Mods/Randomized/LotusModularPistolRandomModRare",
        "kitgun",
    ),
    (
        "/Lotus/Upgrades/Mods/Randomized/LotusModularMeleeRandomModRare",
        "zaw",
    ),
];

/// Extract every weapon carrying both a `productCategory` and an `omegaAttenuation`.
pub fn weapons_from_manifest(value: &serde_json::Value) -> Vec<RawWeapon> {
    let mut out = Vec::new();
    let Some(obj) = value.as_object() else {
        return out;
    };
    for v in obj.values() {
        let Some(arr) = v.as_array() else {
            continue;
        };
        for el in arr {
            let (Some(unique_name), Some(weapon_type), Some(omega)) = (
                el.get("uniqueName").and_then(|x| x.as_str()),
                el.get("productCategory").and_then(|x| x.as_str()),
                el.get("omegaAttenuation").and_then(|x| x.as_f64()),
            ) else {
                continue;
            };
            if unique_name.is_empty() || weapon_type.is_empty() {
                continue;
            }
            out.push(RawWeapon {
                unique_name: unique_name.to_string(),
                weapon_type: weapon_type.to_string(),
                omega_attenuation: omega,
            });
        }
    }
    out
}

/// Extract per-class riven attribute base values from `ExportUpgrades` riven templates.
pub fn riven_bases_from_manifest(value: &serde_json::Value) -> Vec<RawRivenBase> {
    let mut out = Vec::new();
    let Some(obj) = value.as_object() else {
        return out;
    };
    let class_of: std::collections::HashMap<&str, &str> =
        RIVEN_CLASS_TEMPLATES.iter().copied().collect();
    for v in obj.values() {
        let Some(arr) = v.as_array() else {
            continue;
        };
        for el in arr {
            let Some(unique_name) = el.get("uniqueName").and_then(|x| x.as_str()) else {
                continue;
            };
            let Some(&riven_class) = class_of.get(unique_name) else {
                continue;
            };
            let Some(entries) = el.get("upgradeEntries").and_then(|x| x.as_array()) else {
                continue;
            };
            for entry in entries {
                let Some(tag) = entry.get("tag").and_then(|x| x.as_str()) else {
                    continue;
                };
                let Some(base_value) = entry
                    .get("upgradeValues")
                    .and_then(|x| x.as_array())
                    .and_then(|vals| vals.first())
                    .and_then(|first| first.get("value"))
                    .and_then(|x| x.as_f64())
                else {
                    continue;
                };
                out.push(RawRivenBase {
                    riven_class: riven_class.to_string(),
                    tag: tag.to_string(),
                    base_value,
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_index_line() {
        let e = parse_line("ExportResources_en.json!00_abc123");
        assert_eq!(e.manifest, "ExportResources");
        assert_eq!(e.hash, "00_abc123");
        assert_eq!(e.file, "ExportResources_en.json!00_abc123");
    }

    #[test]
    fn parses_index_line_without_lang_suffix() {
        let e = parse_line("ExportManifest.json!00_def456");
        assert_eq!(e.manifest, "ExportManifest");
        assert_eq!(e.hash, "00_def456");
    }

    #[test]
    fn index_hash_is_join_of_tokens() {
        let entries = vec![
            IndexEntry {
                manifest: "A".into(),
                file: "A!h1".into(),
                hash: "h1".into(),
            },
            IndexEntry {
                manifest: "B".into(),
                file: "B!h2".into(),
                hash: "h2".into(),
            },
        ];
        assert_eq!(index_hash(&entries), "h1.h2");
    }

    #[test]
    fn sanitizes_control_chars_so_serde_parses() {
        let raw = b"{\"name\":\"a\x01\x1fb\"}";
        let cleaned = sanitize_json(raw);
        let v: serde_json::Value = serde_json::from_str(&cleaned).expect("parses after sanitize");
        assert_eq!(v["name"], "a  b");
    }

    #[test]
    fn extracts_named_items_across_multiple_arrays() {
        let value = serde_json::json!({
            "ExportWeapons": [{ "uniqueName": "/W/A", "name": "Braton", "productCategory": "Pistols" }],
            "ExportRailjackWeapons": [{ "uniqueName": "/W/B", "name": "Carcinnox" }],
            "Ignored": [{ "uniqueName": "/R/1" }]
        });
        let items = items_from_manifest(&value);
        assert_eq!(items.len(), 2);
        assert!(
            items
                .iter()
                .any(|i| i.unique_name == "/W/A" && i.name == "Braton")
        );
        assert!(items.iter().all(|i| i.unique_name != "/R/1"));
    }

    #[test]
    fn extracts_weapons_with_disposition() {
        let value = serde_json::json!({
            "ExportWeapons": [
                { "uniqueName": "/W/Braton", "name": "Braton",
                  "productCategory": "LongGuns", "omegaAttenuation": 1.0 },
                { "uniqueName": "/W/NoOmega", "name": "X", "productCategory": "LongGuns" }
            ],
            "ExportRailjackWeapons": [
                { "uniqueName": "/W/Cryophon", "name": "Cryophon",
                  "productCategory": "CrewShipWeapons", "omegaAttenuation": 0.5 }
            ]
        });
        let w = weapons_from_manifest(&value);
        assert_eq!(w.len(), 2);
        let braton = w.iter().find(|x| x.unique_name == "/W/Braton").unwrap();
        assert_eq!(braton.weapon_type, "LongGuns");
        assert_eq!(braton.omega_attenuation, 1.0);
        assert!(w.iter().all(|x| x.unique_name != "/W/NoOmega"));
    }

    #[test]
    fn extracts_riven_bases_for_known_class_templates_only() {
        let value = serde_json::json!({
            "ExportUpgrades": [
                {
                    "uniqueName": "/Lotus/Upgrades/Mods/Randomized/LotusRifleRandomModRare",
                    "upgradeEntries": [
                        { "tag": "WeaponCritChanceMod", "prefixTag": "crita", "suffixTag": "cron",
                          "upgradeValues": [{ "value": 0.0167, "locTag": "|val|% Critical Chance" }] }
                    ]
                },
                { "uniqueName": "/Lotus/Upgrades/Mods/Randomized/RawRifleRandomMod" }
            ]
        });
        let bases = riven_bases_from_manifest(&value);
        assert_eq!(bases.len(), 1);
        assert_eq!(bases[0].riven_class, "rifle");
        assert_eq!(bases[0].tag, "WeaponCritChanceMod");
        assert!((bases[0].base_value - 0.0167).abs() < 1e-9);
    }
}
