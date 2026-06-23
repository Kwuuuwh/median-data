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
}
