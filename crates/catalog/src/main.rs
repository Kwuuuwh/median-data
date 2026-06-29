use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use wf_fetch::{
    DeExportSource, IndexEntry, ManifestSource, http_agent, index_hash, items_from_manifest,
};

use crate::config::Config;

mod assemble;
mod category;
mod compress;
mod config;
mod drop_bridge;
mod joins;
mod quality;
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
        Some("probe-drops") => probe_drops(),
        Some("build") => build(&args[1..]),
        _ => anyhow::bail!(
            "usage: catalog (probe-index | probe-items | probe-joins | probe-wfm | probe-drops | build [--out PATH] [--langs en,ru] [--skip-unchanged] [--last-hash H])"
        ),
    }
}

/// Config directory (`CATALOG_CONFIG_DIR` or `config`).
fn config_dir() -> PathBuf {
    std::env::var_os("CATALOG_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("config"))
}

/// Build a DE Export source from configured endpoints.
fn de_source(config: &Config) -> DeExportSource {
    DeExportSource::new(
        http_agent(),
        config.endpoints.de_index_base.as_str(),
        config.endpoints.de_manifest_base.as_str(),
    )
}

/// Parsed `build` arguments.
struct BuildArgs {
    out: String,
    langs: Option<Vec<String>>,
    skip_unchanged: bool,
    last_hash: Option<String>,
}

/// Parse `build` flags, falling back to defaults.
fn parse_build_args(args: &[String]) -> BuildArgs {
    let mut out = "catalog.sqlite".to_string();
    let mut langs = None;
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
                    langs = Some(
                        v.split(',')
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty())
                            .collect(),
                    );
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
    let config = Config::load(&config_dir())?;
    let source = de_source(&config);
    let wfm_agent = http_agent();
    let langs = parsed.langs.unwrap_or_else(|| config.build.langs.clone());

    let en_index = source.fetch_index("en")?;
    let current_hash = schema::catalog_version(&index_hash(&en_index));
    if parsed.skip_unchanged && should_skip(parsed.last_hash.as_deref(), &current_hash) {
        println!("de index unchanged; skipping rebuild ({current_hash})");
        return Ok(());
    }

    let data = assemble::assemble_catalog(&source, &wfm_agent, &config, &langs, now_millis())?;
    let sqlite_path = PathBuf::from(&parsed.out);
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(write::write_catalog(&sqlite_path, &data))?;

    let zst_path = PathBuf::from(format!("{}.zst", parsed.out));
    compress::compress_file(&sqlite_path, &zst_path, 19)?;

    let hash_path = sqlite_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .join("catalog.de_index_hash");
    std::fs::write(&hash_path, &data.de_index_hash)?;

    let metrics_path = PathBuf::from(format!("{}.metrics.json", parsed.out));
    let metrics_file = std::fs::File::create(&metrics_path)?;
    serde_json::to_writer_pretty(metrics_file, &data.quality)?;

    println!(
        "wrote {} items ({}, {}, {}, {})",
        data.items.len(),
        sqlite_path.display(),
        zst_path.display(),
        hash_path.display(),
        metrics_path.display()
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
    let config = Config::load(&config_dir())?;
    let source = de_source(&config);
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
    let config = Config::load(&config_dir())?;
    let source = de_source(&config);
    let index = source.fetch_index("en")?;
    for name in ["ExportResources", "ExportWeapons", "ExportWarframes"] {
        let Some(entry) = index.iter().find(|e| e.manifest == name) else {
            continue;
        };
        let value = source.fetch_manifest(entry)?;
        let items = items_from_manifest(&value);
        println!("{name}: {} items", items.len());
        if let Some(first) = items.first() {
            let cat = category::derive_category(&config.categories, name, &first.unique_name);
            println!("  sample: {} | {} | {}", first.unique_name, first.name, cat);
        }
    }
    Ok(())
}

/// Build the ducat and icon maps and print a known prime part's joined values.
fn probe_joins() -> anyhow::Result<()> {
    const ASH_PRIME_CHASSIS: &str = "/Lotus/Types/Recipes/WarframeRecipes/AshPrimeChassisComponent";
    let config = Config::load(&config_dir())?;
    let source = de_source(&config);
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

/// Fetch the WFM bridge and print counts plus the `ash_prime_set` members.
fn probe_wfm() -> anyhow::Result<()> {
    let config = Config::load(&config_dir())?;
    let agent = http_agent();
    let bridge = wfm_bridge::fetch_bridge(&agent, &config.endpoints.wfm_items_url)?;
    println!(
        "wfm non-set items (by gameRef): {}",
        bridge.by_game_ref.len()
    );
    println!("wfm trade sets: {}", bridge.sets.len());
    if let Some(set) = bridge.sets.iter().find(|s| s.slug == "ash_prime_set") {
        println!("ash_prime_set members ({}):", set.members.len());
        for m in &set.members {
            println!("  {} | {}", m.slug, m.game_ref);
        }
    }
    Ok(())
}

/// Fetch + parse the drop tables, build a light EN name index, and report resolve stats.
fn probe_drops() -> anyhow::Result<()> {
    use std::collections::BTreeSet;

    let config = Config::load(&config_dir())?;
    let source = de_source(&config);
    let agent = http_agent();

    let html = wf_fetch::fetch_droptables(&agent, &config.endpoints.droptables_url)?;
    let dt = wf_fetch::parse_droptables(&html)?;
    println!(
        "parsed {} relic rewards, {} item drops",
        dt.relics.len(),
        dt.drops.len()
    );

    // Light EN name index from item manifests (no WFM/sets — the full build resolves more).
    let index = source.fetch_index("en")?;
    let mut names: Vec<write::NameRow> = Vec::new();
    for mname in &config.build.item_manifests {
        let Some(entry) = index.iter().find(|e| e.manifest == mname.as_str()) else {
            continue;
        };
        let value = source.fetch_manifest(entry)?;
        for raw in items_from_manifest(&value) {
            names.push(write::NameRow {
                unique_name: raw.unique_name,
                lang: "en".to_string(),
                source: "DE",
                name: raw.name,
            });
        }
    }
    let idx = drop_bridge::NameIndex::build(&names);
    println!(
        "en name index: {} names, {} collisions",
        idx.name_count(),
        idx.collisions()
    );

    let mut unresolved: BTreeSet<String> = BTreeSet::new();
    let mut relic_ok = 0usize;
    for r in &dt.relics {
        let relic = idx.resolve(&r.relic_name);
        let reward = idx.resolve(&r.reward_name);
        if relic.is_some() && reward.is_some() {
            relic_ok += 1;
        }
        if relic.is_none() {
            unresolved.insert(r.relic_name.clone());
        }
        if reward.is_none() {
            unresolved.insert(r.reward_name.clone());
        }
    }
    let mut drop_ok = 0usize;
    for d in &dt.drops {
        if idx.resolve(&d.item_name).is_some() {
            drop_ok += 1;
        } else {
            unresolved.insert(d.item_name.clone());
        }
    }

    println!(
        "relic rewards resolved (both ends): {relic_ok}/{}",
        dt.relics.len()
    );
    println!("item drops resolved: {drop_ok}/{}", dt.drops.len());
    println!("distinct unresolved names: {}", unresolved.len());
    for n in unresolved.iter().take(40) {
        println!("  unresolved: {n}");
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
