use scraper::{ElementRef, Html, Selector};

use crate::net::get_bytes;

/// Landing URL that redirects to the current drop-table HTML object.
pub const DROPTABLES_URL: &str = "https://www.warframe.com/droptables";

/// Relic refinement tier carried by the relic reward tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refinement {
    Intact,
    Exceptional,
    Flawless,
    Radiant,
}

impl Refinement {
    /// Stable lowercase token for storage.
    pub fn as_str(self) -> &'static str {
        match self {
            Refinement::Intact => "intact",
            Refinement::Exceptional => "exceptional",
            Refinement::Flawless => "flawless",
            Refinement::Radiant => "radiant",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            "Intact" => Some(Refinement::Intact),
            "Exceptional" => Some(Refinement::Exceptional),
            "Flawless" => Some(Refinement::Flawless),
            "Radiant" => Some(Refinement::Radiant),
            _ => None,
        }
    }
}

/// The kind of place an item drops from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaceKind {
    Node,
    Key,
    Sortie,
    Bounty,
    Transient,
    Enemy,
}

impl PlaceKind {
    /// Stable lowercase token for storage.
    pub fn as_str(self) -> &'static str {
        match self {
            PlaceKind::Node => "node",
            PlaceKind::Key => "key",
            PlaceKind::Sortie => "sortie",
            PlaceKind::Bounty => "bounty",
            PlaceKind::Transient => "transient",
            PlaceKind::Enemy => "enemy",
        }
    }
}

/// One reward line of a relic at a given refinement.
#[derive(Debug, Clone, PartialEq)]
pub struct RelicReward {
    /// Relic display name, e.g. `Axi A1 Relic`.
    pub relic_name: String,
    pub refinement: Refinement,
    /// Reward display name, e.g. `Akstiletto Prime Barrel`.
    pub reward_name: String,
    /// Rarity label as printed, e.g. `Uncommon`.
    pub rarity: String,
    /// Drop probability in `[0, 1]`.
    pub chance: f64,
}

/// One drop of an item from a place (mission node, key, bounty, sortie, or enemy).
#[derive(Debug, Clone, PartialEq)]
pub struct ItemDrop {
    /// Item display name as printed in the source.
    pub item_name: String,
    /// Place display name (node/key/bounty header, or enemy name).
    pub place_name: String,
    pub place_kind: PlaceKind,
    /// Rotation letter (`A`/`B`/`C`) when the place rotates.
    pub rotation: Option<String>,
    /// Bounty stage label when the place is staged.
    pub stage: Option<String>,
    /// Rarity label as printed, e.g. `Rare`.
    pub rarity: String,
    /// Drop probability in `[0, 1]`.
    pub chance: f64,
    /// Source section id, e.g. `missionRewards`.
    pub source: String,
}

/// All parsed drop relations from the droptables HTML.
#[derive(Debug, Clone, Default)]
pub struct DropTables {
    pub relics: Vec<RelicReward>,
    pub drops: Vec<ItemDrop>,
    /// Section ids that carried a table but the parser does not handle.
    pub unknown_sections: Vec<String>,
}

/// Fetch the drop-table HTML as text, following the landing redirect.
pub fn fetch_droptables(agent: &ureq::Agent, url: &str) -> anyhow::Result<String> {
    let raw = get_bytes(agent, url)?;
    Ok(String::from_utf8_lossy(&raw).into_owned())
}

/// Parse the whole drop-table HTML into relic and item-drop relations.
pub fn parse_droptables(html: &str) -> anyhow::Result<DropTables> {
    let sel = Selectors::new()?;
    let doc = Html::parse_document(html);
    let mut out = DropTables::default();
    for h3 in doc.select(&sel.h3) {
        let Some(id) = h3.value().attr("id") else {
            continue;
        };
        let Some(table) = next_table(h3) else {
            continue;
        };
        match id {
            "relicRewards" => parse_relics(&sel, table, &mut out.relics),
            "missionRewards" => parse_layout1(&sel, table, PlaceKind::Node, id, &mut out.drops),
            "keyRewards" => parse_layout1(&sel, table, PlaceKind::Key, id, &mut out.drops),
            "transientRewards" => {
                parse_layout1(&sel, table, PlaceKind::Transient, id, &mut out.drops)
            }
            "sortieRewards" => parse_layout1(&sel, table, PlaceKind::Sortie, id, &mut out.drops),
            "cetusRewards" | "solarisRewards" | "deimosRewards" | "zarimanRewards"
            | "entratiLabRewards" | "hexRewards" => {
                parse_layout1(&sel, table, PlaceKind::Bounty, id, &mut out.drops)
            }
            "modByAvatar"
            | "blueprintByAvatar"
            | "resourceByAvatar"
            | "sigilByAvatar"
            | "additionalItemByAvatar"
            | "relicByAvatar" => parse_by_avatar(&sel, table, id, &mut out.drops),
            "modByDrop" | "blueprintByDrop" | "resourceByDrop" => {
                parse_by_drop(&sel, table, id, &mut out.drops)
            }
            _ => out.unknown_sections.push(id.to_string()),
        }
    }
    out.unknown_sections.sort();
    out.unknown_sections.dedup();
    Ok(out)
}

/// Pre-parsed selectors reused across the document walk.
struct Selectors {
    h3: Selector,
    tr: Selector,
    cell: Selector,
}

impl Selectors {
    fn new() -> anyhow::Result<Self> {
        Ok(Self {
            h3: Selector::parse("h3[id]").map_err(|e| anyhow::anyhow!("selector h3[id]: {e:?}"))?,
            tr: Selector::parse("tr").map_err(|e| anyhow::anyhow!("selector tr: {e:?}"))?,
            cell: Selector::parse("th, td")
                .map_err(|e| anyhow::anyhow!("selector th,td: {e:?}"))?,
        })
    }
}

/// The first element sibling after `h3`, if it is a `<table>`.
fn next_table(h3: ElementRef<'_>) -> Option<ElementRef<'_>> {
    let table = h3.next_siblings().filter_map(ElementRef::wrap).next()?;
    (table.value().name() == "table").then_some(table)
}

/// Whether a row is the visual blank-row separator between blocks.
fn is_blank_row(tr: ElementRef<'_>) -> bool {
    tr.value()
        .attr("class")
        .is_some_and(|c| c.contains("blank-row"))
}

/// Trimmed, entity-decoded text content of a cell.
fn cell_text(cell: &ElementRef<'_>) -> String {
    cell.text().collect::<String>().trim().to_string()
}

/// Parse the relic reward table (header `Name (Refinement)`, then `item | rarity (chance)`).
fn parse_relics(sel: &Selectors, table: ElementRef<'_>, out: &mut Vec<RelicReward>) {
    let mut relic: Option<(String, Refinement)> = None;
    for tr in table.select(&sel.tr) {
        if is_blank_row(tr) {
            continue;
        }
        let cells: Vec<ElementRef> = tr.select(&sel.cell).collect();
        let Some(first) = cells.first() else {
            continue;
        };
        if first.value().name() == "th" {
            relic = parse_relic_header(&cell_text(first));
        } else if cells.len() >= 2 {
            let Some((relic_name, refinement)) = &relic else {
                continue;
            };
            let Some((rarity, chance)) = parse_rarity_chance(&cell_text(&cells[1])) else {
                continue;
            };
            let reward_name = cell_text(&cells[0]);
            if !reward_name.is_empty() {
                out.push(RelicReward {
                    relic_name: relic_name.clone(),
                    refinement: *refinement,
                    reward_name,
                    rarity,
                    chance,
                });
            }
        }
    }
}

/// Parse a 2-column "place → rotation/stage → rewards" table.
fn parse_layout1(
    sel: &Selectors,
    table: ElementRef<'_>,
    kind: PlaceKind,
    source: &str,
    out: &mut Vec<ItemDrop>,
) {
    let mut place: Option<String> = None;
    let mut rotation: Option<String> = None;
    let mut stage: Option<String> = None;
    for tr in table.select(&sel.tr) {
        if is_blank_row(tr) {
            place = None;
            rotation = None;
            stage = None;
            continue;
        }
        let cells: Vec<ElementRef> = tr.select(&sel.cell).collect();
        let Some(first) = cells.first() else {
            continue;
        };
        if first.value().name() == "th" {
            let text = cell_text(first);
            if let Some(rot) = text.strip_prefix("Rotation ") {
                rotation = Some(rot.trim().to_string());
                stage = None;
            } else if let Some(st) = text.strip_prefix("Stage ") {
                stage = Some(st.trim().to_string());
                rotation = None;
            } else {
                place = Some(text);
                rotation = None;
                stage = None;
            }
        } else if cells.len() >= 2 {
            let Some(place_name) = &place else {
                continue;
            };
            let Some((rarity, chance)) = parse_rarity_chance(&cell_text(&cells[1])) else {
                continue;
            };
            let item_name = cell_text(&cells[0]);
            if !item_name.is_empty() {
                out.push(ItemDrop {
                    item_name,
                    place_name: place_name.clone(),
                    place_kind: kind,
                    rotation: rotation.clone(),
                    stage: stage.clone(),
                    rarity,
                    chance,
                    source: source.to_string(),
                });
            }
        }
    }
}

/// Parse a 3-column "enemy → items" table (`*ByAvatar`).
fn parse_by_avatar(sel: &Selectors, table: ElementRef<'_>, source: &str, out: &mut Vec<ItemDrop>) {
    let mut enemy: Option<String> = None;
    for tr in table.select(&sel.tr) {
        if is_blank_row(tr) {
            enemy = None;
            continue;
        }
        let cells: Vec<ElementRef> = tr.select(&sel.cell).collect();
        let Some(first) = cells.first() else {
            continue;
        };
        if first.value().name() == "th" {
            enemy = Some(cell_text(first));
        } else if cells.len() >= 3 {
            let Some(place_name) = &enemy else {
                continue;
            };
            let Some((rarity, chance)) = parse_rarity_chance(&cell_text(&cells[2])) else {
                continue;
            };
            let item_name = cell_text(&cells[1]);
            if !item_name.is_empty() {
                out.push(ItemDrop {
                    item_name,
                    place_name: place_name.clone(),
                    place_kind: PlaceKind::Enemy,
                    rotation: None,
                    stage: None,
                    rarity,
                    chance,
                    source: source.to_string(),
                });
            }
        }
    }
}

/// Parse a 3-column "item → enemies" table (`*ByDrop`).
fn parse_by_drop(sel: &Selectors, table: ElementRef<'_>, source: &str, out: &mut Vec<ItemDrop>) {
    let mut item: Option<String> = None;
    for tr in table.select(&sel.tr) {
        if is_blank_row(tr) {
            item = None;
            continue;
        }
        let cells: Vec<ElementRef> = tr.select(&sel.cell).collect();
        let Some(first) = cells.first() else {
            continue;
        };
        if first.value().name() == "th" {
            if cells.len() == 1 {
                item = Some(cell_text(first));
            }
        } else if cells.len() >= 3 {
            let Some(item_name) = &item else {
                continue;
            };
            let Some((rarity, chance)) = parse_rarity_chance(&cell_text(&cells[2])) else {
                continue;
            };
            let place_name = cell_text(&cells[0]);
            if !place_name.is_empty() {
                out.push(ItemDrop {
                    item_name: item_name.clone(),
                    place_name,
                    place_kind: PlaceKind::Enemy,
                    rotation: None,
                    stage: None,
                    rarity,
                    chance,
                    source: source.to_string(),
                });
            }
        }
    }
}

/// Parse a `Rarity (chance%)` cell into a rarity label and a `[0, 1]` probability.
fn parse_rarity_chance(s: &str) -> Option<(String, f64)> {
    let open = s.rfind('(')?;
    let rarity = s[..open].trim().to_string();
    if rarity.is_empty() {
        return None;
    }
    let inner = s[open + 1..].trim_end().strip_suffix(')')?;
    let pct = inner.trim().strip_suffix('%')?;
    let value: f64 = pct.trim().parse().ok()?;
    Some((rarity, value / 100.0))
}

/// Parse a relic block header `Name (Refinement)` into the relic name and tier.
fn parse_relic_header(s: &str) -> Option<(String, Refinement)> {
    let open = s.rfind('(')?;
    let name = s[..open].trim().to_string();
    if name.is_empty() {
        return None;
    }
    let inner = s[open + 1..].trim_end().strip_suffix(')')?;
    let refinement = Refinement::parse(inner)?;
    Some((name, refinement))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rarity_and_fractional_chance() {
        let (rarity, chance) = parse_rarity_chance("Uncommon (11.00%)").unwrap();
        assert_eq!(rarity, "Uncommon");
        assert!((chance - 0.11).abs() < 1e-9);
    }

    #[test]
    fn parses_multiword_rarity_and_full_chance() {
        let (rarity, chance) = parse_rarity_chance("Very Common (100.00%)").unwrap();
        assert_eq!(rarity, "Very Common");
        assert!((chance - 1.0).abs() < 1e-9);
    }

    #[test]
    fn rejects_cell_without_parentheses() {
        assert!(parse_rarity_chance("Uncommon").is_none());
    }

    #[test]
    fn parses_relic_header_with_refinement() {
        let (name, refinement) = parse_relic_header("Axi A1 Relic (Intact)").unwrap();
        assert_eq!(name, "Axi A1 Relic");
        assert_eq!(refinement, Refinement::Intact);
    }

    #[test]
    fn rejects_relic_header_with_unknown_refinement() {
        assert!(parse_relic_header("Axi A1 Relic (Bronze)").is_none());
    }

    #[test]
    fn refinement_round_trips_to_token() {
        assert_eq!(Refinement::Radiant.as_str(), "radiant");
        assert_eq!(Refinement::parse("Radiant"), Some(Refinement::Radiant));
    }

    const FIXTURE: &str = r#"
        <html><body>
        <h3 id="relicRewards">Relics:</h3>
        <table>
        <tr><th colspan="2">Axi A1 Relic (Intact)</th></tr><tr><td>Akstiletto Prime Barrel</td><td>Uncommon (11.00%)</td></tr><tr><td>Nikana Prime Blueprint</td><td>Rare (2.00%)</td></tr>
        <tr class="blank-row"><td class="blank-row" colspan="2"></td></tr>
        <tr><th colspan="2">Axi A1 Relic (Radiant)</th></tr><tr><td>Akstiletto Prime Barrel</td><td>Uncommon (20.00%)</td></tr>
        </table>
        <h3 id="missionRewards">Missions:</h3>
        <table>
        <tr><th colspan="2">Mercury/Apollodorus (Survival)</th></tr><tr><th colspan="2">Rotation A</th></tr><tr><td>100 Endo</td><td>Common (50.00%)</td></tr><tr><th colspan="2">Rotation B</th></tr><tr><td>Lith Q3 Relic</td><td>Rare (7.69%)</td></tr>
        <tr class="blank-row"><td class="blank-row" colspan="2"></td></tr>
        <tr><th colspan="2">Mercury/Tolstoj (Assassination)</th></tr><tr><td>Seer Blueprint</td><td>Common (38.72%)</td></tr>
        </table>
        <h3 id="modByDrop">Mod Drops by Mod:</h3>
        <table>
        <tr><th colspan="3">Target Acquired</th></tr>
        <tr><th>Source</th><th>Mod Drop Chance</th><th>Chance</th></tr>
        <tr><td>Tusk Thumper Bull</td><td>15.00%</td><td>Uncommon (12.50%)</td></tr>
        <tr class="blank-row"><td class="blank-row" colspan="3"></td></tr>
        </table>
        <h3 id="relicByAvatar">Relic Drops by Source:</h3>
        <table>
        <tr><th>Hemocyte</th><th colspan="2">Relic Drop Chance: 20.00%</th></tr><tr><td></td><td>Lith K12 Relic</td><td>Uncommon (12.91%)</td></tr>
        <tr class="blank-row"><td class="blank-row" colspan="3"></td></tr>
        </table>
        </body></html>
        "#;

    #[test]
    fn parses_relics_per_refinement() {
        let dt = parse_droptables(FIXTURE).unwrap();
        let intact: Vec<_> = dt
            .relics
            .iter()
            .filter(|r| r.relic_name == "Axi A1 Relic" && r.refinement == Refinement::Intact)
            .collect();
        assert_eq!(intact.len(), 2);
        let barrel = intact
            .iter()
            .find(|r| r.reward_name == "Akstiletto Prime Barrel")
            .unwrap();
        assert_eq!(barrel.rarity, "Uncommon");
        assert!((barrel.chance - 0.11).abs() < 1e-9);
        assert!(
            dt.relics.iter().any(|r| r.refinement == Refinement::Radiant
                && r.reward_name == "Akstiletto Prime Barrel")
        );
    }

    #[test]
    fn parses_mission_rotations() {
        let dt = parse_droptables(FIXTURE).unwrap();
        let endo = dt.drops.iter().find(|d| d.item_name == "100 Endo").unwrap();
        assert_eq!(endo.place_name, "Mercury/Apollodorus (Survival)");
        assert_eq!(endo.place_kind, PlaceKind::Node);
        assert_eq!(endo.rotation.as_deref(), Some("A"));
        let relic = dt
            .drops
            .iter()
            .find(|d| d.item_name == "Lith Q3 Relic" && d.source == "missionRewards")
            .unwrap();
        assert_eq!(relic.rotation.as_deref(), Some("B"));
        let seer = dt
            .drops
            .iter()
            .find(|d| d.item_name == "Seer Blueprint")
            .unwrap();
        assert_eq!(seer.place_name, "Mercury/Tolstoj (Assassination)");
        assert_eq!(seer.rotation, None);
    }

    #[test]
    fn parses_by_drop_and_by_avatar_as_enemy_places() {
        let dt = parse_droptables(FIXTURE).unwrap();
        let by_drop = dt.drops.iter().find(|d| d.source == "modByDrop").unwrap();
        assert_eq!(by_drop.item_name, "Target Acquired");
        assert_eq!(by_drop.place_name, "Tusk Thumper Bull");
        assert_eq!(by_drop.place_kind, PlaceKind::Enemy);
        assert!((by_drop.chance - 0.125).abs() < 1e-9);

        let by_avatar = dt
            .drops
            .iter()
            .find(|d| d.source == "relicByAvatar")
            .unwrap();
        assert_eq!(by_avatar.item_name, "Lith K12 Relic");
        assert_eq!(by_avatar.place_name, "Hemocyte");
        assert_eq!(by_avatar.place_kind, PlaceKind::Enemy);
    }

    const UNKNOWN_FIXTURE: &str = r#"
        <html><body>
        <h3 id="newMystery">Mystery:</h3>
        <table><tr><th colspan="2">Whatever (Intact)</th></tr><tr><td>Thing</td><td>Common (1.00%)</td></tr></table>
        <h3 id="missionRewards">Missions:</h3>
        <table><tr><th colspan="2">Mercury/Tolstoj (Assassination)</th></tr><tr><td>Seer Blueprint</td><td>Common (38.72%)</td></tr></table>
        </body></html>
        "#;

    #[test]
    fn records_unknown_sections() {
        let dt = parse_droptables(UNKNOWN_FIXTURE).unwrap();
        assert_eq!(dt.unknown_sections, vec!["newMystery".to_string()]);
    }

    #[test]
    fn known_fixture_has_no_unknown_sections() {
        let dt = parse_droptables(FIXTURE).unwrap();
        assert!(dt.unknown_sections.is_empty());
    }
}
