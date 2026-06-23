use std::path::Path;
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use wf_fetch::{
    DeExportSource, IndexEntry, ManifestSource, http_agent, index_hash, items_from_manifest,
};

mod assemble;
mod category;
mod compress;
mod joins;
mod schema;
mod wfm_bridge;
mod write;

/// CLI entry point.
fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

/// Dispatch on the first argument.
fn run(args: &[String]) -> anyhow::Result<()> {
    match args.first().map(String::as_str) {
        Some("probe-index") => probe_index(),
        Some("probe-items") => probe_items(),
        Some("probe-joins") => probe_joins(),
        Some("probe-wfm") => probe_wfm(),
        Some("build") => build(&args[1..]),
        _ => anyhow::bail!(
            "usage: catalog (probe-index | probe-items | probe-joins | probe-wfm | build [--out PATH] [--langs en,ru] [--skip-unchanged] [--last-hash H])"
        ),
    }
}

/// Parsed `build` arguments.
struct BuildArgs {
    out: String,
    langs: Vec<String>,
    skip_unchanged: bool,
    last_hash: Option<String>,
}

/// Parse `build` flags, falling back to defaults.
fn parse_build_args(args: &[String]) -> BuildArgs {
    let mut out = "catalog.sqlite".to_string();
    let mut langs = vec!["en".to_string(), "ru".to_string()];
    let mut skip_unchanged = false;
    let mut last_hash = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--out" => {
                if let Some(v) = args.get(i + 1) {
                    out = v.clone();
                    i += 1;
                }
            }
            "--langs" => {
                if let Some(v) = args.get(i + 1) {
                    langs = v
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                    i += 1;
                }
            }
            "--skip-unchanged" => skip_unchanged = true,
            "--last-hash" => {
                if let Some(v) = args.get(i + 1) {
                    last_hash = Some(v.clone());
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    BuildArgs {
        out,
        langs,
        skip_unchanged,
        last_hash,
    }
}

/// True when `--skip-unchanged` is set and the fresh DE index hash equals the last built one.
fn should_skip(last_hash: Option<&str>, current_hash: &str) -> bool {
    last_hash == Some(current_hash)
}

/// Assemble the catalog, write SQLite, compress to `.zst`, and emit the index-hash sidecar.
fn build(args: &[String]) -> anyhow::Result<()> {
    let parsed = parse_build_args(args);
    let source = DeExportSource::with_defaults(http_agent());
    let wfm_agent = http_agent();

    let en_index = source.fetch_index("en")?;
    let current_hash = index_hash(&en_index);
    if parsed.skip_unchanged && should_skip(parsed.last_hash.as_deref(), &current_hash) {
        println!("de index unchanged; skipping rebuild ({current_hash})");
        return Ok(());
    }

    let data = assemble::assemble_catalog(&source, &wfm_agent, &parsed.langs, now_millis())?;
    let sqlite_path = std::path::PathBuf::from(&parsed.out);
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(write::write_catalog(&sqlite_path, &data))?;

    let zst_path = std::path::PathBuf::from(format!("{}.zst", parsed.out));
    compress::compress_file(&sqlite_path, &zst_path, 19)?;

    let hash_path = sqlite_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .join("catalog.de_index_hash");
    std::fs::write(&hash_path, &data.de_index_hash)?;

    println!(
        "wrote {} items ({}, {}, {})",
        data.items.len(),
        sqlite_path.display(),
        zst_path.display(),
        hash_path.display()
    );
    Ok(())
}

/// Current wall-clock as unix epoch milliseconds.
fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Fetch the EN index and print the manifests with their hash tokens, plus the overall index hash.
fn probe_index() -> anyhow::Result<()> {
    let source = DeExportSource::with_defaults(http_agent());
    let entries = source.fetch_index("en")?;
    println!(
        "{} manifests (index hash {})",
        entries.len(),
        index_hash(&entries)
    );
    for e in &entries {
        println!("{:<22} {}", e.manifest, e.hash);
    }
    Ok(())
}

/// Fetch a few item-bearing manifests, print counts and a categorized sample item.
fn probe_items() -> anyhow::Result<()> {
    let source = DeExportSource::with_defaults(http_agent());
    let index = source.fetch_index("en")?;
    for name in ["ExportResources", "ExportWeapons", "ExportWarframes"] {
        let Some(entry) = index.iter().find(|e| e.manifest == name) else {
            continue;
        };
        let value = source.fetch_manifest(entry)?;
        let items = items_from_manifest(&value);
        println!("{name}: {} items", items.len());
        if let Some(first) = items.first() {
            let cat = category::derive_category(name, &first.unique_name);
            println!("  sample: {} | {} | {}", first.unique_name, first.name, cat);
        }
    }
    Ok(())
}

/// Build the ducat and icon maps and print a known prime part's joined values.
fn probe_joins() -> anyhow::Result<()> {
    const ASH_PRIME_CHASSIS: &str = "/Lotus/Types/Recipes/WarframeRecipes/AshPrimeChassisComponent";
    let source = DeExportSource::with_defaults(http_agent());
    let index = source.fetch_index("en")?;

    let recipes = manifest_value(&source, &index, "ExportRecipes")?;
    let manifest = manifest_value(&source, &index, "ExportManifest")?;
    let ducats = joins::ducat_map(&recipes);
    let icons = joins::icon_map(&manifest);

    println!("recipes with ducat: {}", ducats.len());
    println!("icon entries: {}", icons.len());
    println!(
        "Ash Prime Chassis ducat: {:?}",
        ducats.get(ASH_PRIME_CHASSIS)
    );
    println!(
        "Ash Prime Chassis icon:  {:?}",
        icons.get(ASH_PRIME_CHASSIS)
    );
    Ok(())
}

/// Fetch the WFM bridge and print a known prime part's match, with gameRef diagnostics if missing.
fn probe_wfm() -> anyhow::Result<()> {
    const ASH_PRIME_CHASSIS: &str = "/Lotus/Types/Recipes/WarframeRecipes/AshPrimeChassisComponent";
    let agent = http_agent();
    let bridge = wfm_bridge::fetch_bridge(&agent)?;
    println!("wfm items with gameRef: {}", bridge.len());
    match bridge.get(ASH_PRIME_CHASSIS) {
        Some(entry) => println!(
            "Ash Prime Chassis wfm: {} ({:?})",
            entry.url_name, entry.en_name
        ),
        None => {
            println!(
                "Ash Prime Chassis not matched by exact key; gameRefs containing \"AshPrime\":"
            );
            for (game_ref, entry) in bridge.iter().filter(|(k, _)| k.contains("AshPrime")) {
                println!("  {game_ref} -> {} ({:?})", entry.url_name, entry.en_name);
            }
        }
    }
    Ok(())
}

/// Fetch and parse a manifest by name from the index.
fn manifest_value(
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
    fn skips_only_on_matching_hash() {
        assert!(should_skip(Some("abc"), "abc"));
        assert!(!should_skip(Some("abc"), "xyz"));
        assert!(!should_skip(None, "abc"));
    }
}
