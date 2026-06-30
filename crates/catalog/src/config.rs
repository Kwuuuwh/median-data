use std::path::Path;

use serde::Deserialize;

/// Curated catalog configuration loaded from `config/*.toml`.
pub struct Config {
    pub endpoints: Endpoints,
    pub categories: Categories,
    pub build: BuildConfig,
    pub riven_attributes: RivenAttributes,
}

/// Source endpoints (DE Public Export + warframe.market + DE drop tables).
#[derive(Debug, Deserialize)]
pub struct Endpoints {
    pub de_index_base: String,
    pub de_manifest_base: String,
    pub wfm_items_url: String,
    pub droptables_url: String,
    pub wfcd_relics_url: String,
}

/// Category-derivation rules keyed by source manifest, with a global fallback.
#[derive(Debug, Deserialize)]
pub struct Categories {
    pub default: String,
    #[serde(default, rename = "manifest")]
    pub manifests: Vec<ManifestRule>,
}

/// How one manifest maps to a category: a fixed value, or a default plus path rules.
#[derive(Debug, Deserialize)]
pub struct ManifestRule {
    pub name: String,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub default: Option<String>,
    #[serde(default, rename = "rule")]
    pub rules: Vec<PathRule>,
}

/// A `uniqueName`-substring rule yielding a category when it matches.
#[derive(Debug, Deserialize)]
pub struct PathRule {
    pub contains: String,
    pub category: String,
}

/// Build inputs: default languages and the item-bearing manifests.
#[derive(Debug, Deserialize)]
pub struct BuildConfig {
    pub langs: Vec<String>,
    pub item_manifests: Vec<String>,
}

/// Curated riven attribute metadata (syllables, unit, names); base values come from DE.
#[derive(Debug, Deserialize)]
pub struct RivenAttributes {
    #[serde(default, rename = "attribute")]
    pub attributes: Vec<RivenAttributeRule>,
}

/// One curated riven attribute: its DE tag plus display/grammar metadata.
#[derive(Debug, Deserialize)]
pub struct RivenAttributeRule {
    /// DE stat tag (e.g. `WeaponCritChanceMod`).
    pub tag: String,
    /// Riven-name prefix syllable (may be empty).
    #[serde(default)]
    pub prefix_tag: String,
    /// Riven-name suffix syllable (may be empty).
    #[serde(default)]
    pub suffix_tag: String,
    /// Display unit: `percent` or `flat`.
    pub unit: String,
    /// English stat name.
    pub name_en: String,
    /// Russian stat name (curated; may be empty until filled).
    #[serde(default)]
    pub name_ru: String,
}

impl Config {
    /// Load and validate all config files from `dir`, failing before any network use.
    pub fn load(dir: &Path) -> anyhow::Result<Self> {
        let endpoints: Endpoints = read_toml(&dir.join("endpoints.toml"))?;
        let categories: Categories = read_toml(&dir.join("categories.toml"))?;
        let build: BuildConfig = read_toml(&dir.join("build.toml"))?;
        let riven_attributes: RivenAttributes = read_toml(&dir.join("riven_attributes.toml"))?;
        let config = Config {
            endpoints,
            categories,
            build,
            riven_attributes,
        };
        config.validate()?;
        Ok(config)
    }

    /// Fail-fast schema validation (run before any fetch).
    fn validate(&self) -> anyhow::Result<()> {
        for (field, value) in [
            ("endpoints.de_index_base", &self.endpoints.de_index_base),
            (
                "endpoints.de_manifest_base",
                &self.endpoints.de_manifest_base,
            ),
            ("endpoints.wfm_items_url", &self.endpoints.wfm_items_url),
            ("endpoints.droptables_url", &self.endpoints.droptables_url),
            ("endpoints.wfcd_relics_url", &self.endpoints.wfcd_relics_url),
        ] {
            if !value.starts_with("https://") {
                anyhow::bail!("config: {field} must be an https:// URL, got {value:?}");
            }
        }
        if self.categories.default.trim().is_empty() {
            anyhow::bail!("config: categories.default must be non-empty");
        }
        for m in &self.categories.manifests {
            let has_fixed = m.category.is_some();
            let has_rules = m.default.is_some() || !m.rules.is_empty();
            if has_fixed == has_rules {
                anyhow::bail!(
                    "config: manifest {:?} must have either `category` or (`default` + `rule`)",
                    m.name
                );
            }
            if has_rules && m.default.is_none() {
                anyhow::bail!("config: manifest {:?} has rules but no `default`", m.name);
            }
            for r in &m.rules {
                if r.contains.is_empty() || r.category.trim().is_empty() {
                    anyhow::bail!("config: manifest {:?} has an invalid rule", m.name);
                }
            }
        }
        if self.build.langs.is_empty() {
            anyhow::bail!("config: build.langs must be non-empty");
        }
        if self.build.item_manifests.is_empty() {
            anyhow::bail!("config: build.item_manifests must be non-empty");
        }
        if self.riven_attributes.attributes.is_empty() {
            anyhow::bail!("config: riven_attributes must be non-empty");
        }
        let mut seen_tags = std::collections::HashSet::new();
        for a in &self.riven_attributes.attributes {
            if a.tag.trim().is_empty() {
                anyhow::bail!("config: a riven_attribute has an empty tag");
            }
            if !seen_tags.insert(a.tag.as_str()) {
                anyhow::bail!("config: duplicate riven_attribute tag {:?}", a.tag);
            }
            if a.unit != "percent" && a.unit != "flat" {
                anyhow::bail!(
                    "config: riven_attribute {:?} unit must be 'percent' or 'flat', got {:?}",
                    a.tag,
                    a.unit
                );
            }
            if a.name_en.trim().is_empty() {
                anyhow::bail!("config: riven_attribute {:?} must have a name_en", a.tag);
            }
        }
        Ok(())
    }
}

/// Read and parse one TOML file, tagging errors with the path.
fn read_toml<T: serde::de::DeserializeOwned>(path: &Path) -> anyhow::Result<T> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("read {}: {e}", path.display()))?;
    toml::from_str(&text).map_err(|e| anyhow::anyhow!("parse {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid() -> Config {
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
                langs: vec!["en".into()],
                item_manifests: vec!["ExportWeapons".into()],
            },
            riven_attributes: RivenAttributes {
                attributes: vec![RivenAttributeRule {
                    tag: "WeaponCritChanceMod".into(),
                    prefix_tag: "crita".into(),
                    suffix_tag: "cron".into(),
                    unit: "percent".into(),
                    name_en: "Critical Chance".into(),
                    name_ru: String::new(),
                }],
            },
        }
    }

    #[test]
    fn accepts_valid_config() {
        assert!(valid().validate().is_ok());
    }

    #[test]
    fn rejects_non_https_endpoint() {
        let mut c = valid();
        c.endpoints.de_index_base = "http://x/index_".into();
        assert!(c.validate().is_err());
    }

    #[test]
    fn rejects_non_https_droptables() {
        let mut c = valid();
        c.endpoints.droptables_url = "http://x/droptables".into();
        assert!(c.validate().is_err());
    }

    #[test]
    fn rejects_empty_item_manifests() {
        let mut c = valid();
        c.build.item_manifests.clear();
        assert!(c.validate().is_err());
    }

    #[test]
    fn rejects_bad_riven_unit() {
        let mut c = valid();
        c.riven_attributes.attributes[0].unit = "ratio".into();
        assert!(c.validate().is_err());
    }

    #[test]
    fn rejects_empty_riven_attributes() {
        let mut c = valid();
        c.riven_attributes.attributes.clear();
        assert!(c.validate().is_err());
    }
}
