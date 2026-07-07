//! Semantic extraction of Total War campaign entities from the generic ESF
//! tree — the layer that turns twedit from a tree editor into a campaign
//! editor. Extraction is read-only; edits flow through the ordinary staged
//! edit map keyed by global value id (see `FactionRow::treasury`).
//!
//! Field positions follow `assets/esf_schema.toml` and the empirical scan in
//! `docs/scan_report.md` (Empire save): FACTION i0 = id, s0 = key,
//! s1 = on-screen name, bool2 = major, i1 = capital region (0 = destroyed);
//! FACTION_ECONOMICS i0 = treasury; FACTION_FLAG_AND_COLOURS byte0..2 =
//! primary RGB; REGION s0 = key, s1 = theatre, u5 = town wealth,
//! u9 = controlling faction; REGION_FACTORS u2 = current population.
//! Nodes missing from a particular game's saves simply yield fewer rows —
//! metadata degrades silently per the project's failure-mode rules.

use esf_parser::objects::{EsfDocument, EsfValue, NodeId};

/// One faction, summarized for the Factions view.
#[derive(Debug, Clone, PartialEq)]
pub struct FactionRow {
    /// The FACTION record node (jump target for the tree view).
    pub node: NodeId,
    /// FACTION i0 — runtime faction id, referenced by regions/diplomacy.
    pub id: u32,
    /// factions-table key, e.g. "britain".
    pub key: String,
    /// On-screen name as stored in the save, e.g. "Great Britain".
    pub name: String,
    pub is_major: bool,
    /// True when the current-capital field is 0 (faction destroyed).
    pub destroyed: bool,
    /// Primary flag colour (FACTION_FLAG_AND_COLOURS byte0..2).
    pub color: Option<(u8, u8, u8)>,
    /// (global value id, current amount) of the treasury Int32 —
    /// the value id keys directly into the staged-edit map.
    pub treasury: Option<(u32, i32)>,
}

/// One region, summarized for the Regions view.
#[derive(Debug, Clone, PartialEq)]
pub struct RegionRow {
    pub node: NodeId,
    /// regions-table key, e.g. "norway".
    pub key: String,
    /// Theatre key: america / europe / india.
    pub theatre: String,
    /// REGION u9 — controlling faction id (matches `FactionRow::id`).
    pub owner_faction: u32,
    /// (global value id, amount) of REGION u5 "Town wealth".
    pub town_wealth: Option<(u32, u32)>,
    /// REGION_FACTORS u2 "Current population", when present.
    pub population: Option<u32>,
}

/// First node with the given name, scanning in DFS pre-order.
pub fn find_first_node(doc: &EsfDocument, name: &str) -> Option<NodeId> {
    let name_index = doc.node_names.iter().position(|n| n == name)? as u16;
    doc.nodes
        .iter()
        .position(|n| n.name_index == name_index)
        .map(|idx| idx as NodeId)
}

/// Direct child of `id` with the given node name.
fn find_child(doc: &EsfDocument, id: NodeId, name: &str) -> Option<NodeId> {
    doc.children(id).find(|&c| doc.node_name(c) == name)
}

/// The nth value of a node matching `pick`, with its global value id.
fn nth_value<T>(
    doc: &EsfDocument,
    id: NodeId,
    nth: usize,
    pick: impl Fn(&EsfValue) -> Option<T>,
) -> Option<(u32, T)> {
    doc.node_value_entries(id)
        .filter_map(|(vid, rec)| pick(&rec.value).map(|v| (vid, v)))
        .nth(nth)
}

fn as_i32(v: &EsfValue) -> Option<i32> {
    match v {
        EsfValue::I32(x) => Some(*x),
        _ => None,
    }
}

fn as_u32(v: &EsfValue) -> Option<u32> {
    match v {
        EsfValue::U32(x) => Some(*x),
        _ => None,
    }
}

fn as_u8(v: &EsfValue) -> Option<u8> {
    match v {
        EsfValue::U8(x) => Some(*x),
        _ => None,
    }
}

fn as_bool(v: &EsfValue) -> Option<bool> {
    match v {
        EsfValue::Bool(x) => Some(*x),
        _ => None,
    }
}

fn as_string(doc: &EsfDocument, v: &EsfValue) -> Option<String> {
    matches!(v, EsfValue::Utf16 { .. } | EsfValue::Ascii { .. })
        .then(|| doc.decode_string(v))
        .flatten()
}

/// Members of a poly array, descending through the unnamed record wrapper:
/// FACTION_ARRAY -> FACTION_ARRAY[] record -> FACTION (records inherit the
/// poly's name, and the semantic node sits one level below).
fn array_members<'a>(
    doc: &'a EsfDocument,
    array: NodeId,
    member_name: &'a str,
) -> impl Iterator<Item = NodeId> + 'a {
    doc.children(array).filter_map(move |child| {
        if doc.node_name(child) == member_name {
            Some(child)
        } else {
            doc.children(child).find(|&c| doc.node_name(c) == member_name)
        }
    })
}

/// All factions in the campaign, in save-file order. Empty when the file has
/// no FACTION_ARRAY (non-campaign ESF like startpos fragments).
pub fn extract_factions(doc: &EsfDocument) -> Vec<FactionRow> {
    let Some(array) = find_first_node(doc, "FACTION_ARRAY") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for node in array_members(doc, array, "FACTION") {
        let Some((_, id)) = nth_value(doc, node, 0, as_i32) else {
            continue;
        };
        let key = nth_value(doc, node, 0, |v| as_string(doc, v))
            .map(|(_, s)| s)
            .unwrap_or_default();
        let name = nth_value(doc, node, 1, |v| as_string(doc, v))
            .map(|(_, s)| s)
            .unwrap_or_else(|| key.clone());
        let is_major = nth_value(doc, node, 2, as_bool).map(|(_, b)| b).unwrap_or(false);
        let destroyed = nth_value(doc, node, 1, as_i32)
            .map(|(_, capital)| capital == 0)
            .unwrap_or(false);

        let treasury = find_child(doc, node, "FACTION_ECONOMICS")
            .and_then(|econ| nth_value(doc, econ, 0, as_i32));

        let color = find_child(doc, node, "FACTION_FLAG_AND_COLOURS").and_then(|fc| {
            let r = nth_value(doc, fc, 0, as_u8)?.1;
            let g = nth_value(doc, fc, 1, as_u8)?.1;
            let b = nth_value(doc, fc, 2, as_u8)?.1;
            Some((r, g, b))
        });

        out.push(FactionRow {
            node,
            id: id as u32,
            key,
            name,
            is_major,
            destroyed,
            color,
            treasury,
        });
    }
    out
}

/// All regions in the campaign, in save-file order.
pub fn extract_regions(doc: &EsfDocument) -> Vec<RegionRow> {
    let Some(array) = find_first_node(doc, "REGIONS_ARRAY") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for node in array_members(doc, array, "REGION") {
        let Some((_, key)) = nth_value(doc, node, 0, |v| as_string(doc, v)) else {
            continue;
        };
        let theatre = nth_value(doc, node, 1, |v| as_string(doc, v))
            .map(|(_, s)| s)
            .unwrap_or_default();
        let owner_faction = nth_value(doc, node, 9, as_u32).map(|(_, v)| v).unwrap_or(0);
        let town_wealth = nth_value(doc, node, 5, as_u32);
        let population = find_child(doc, node, "POPULATION")
            .and_then(|p| find_child(doc, p, "REGION_FACTORS"))
            .and_then(|f| nth_value(doc, f, 2, as_u32))
            .map(|(_, v)| v);

        out.push(RegionRow {
            node,
            key,
            theatre,
            owner_faction,
            town_wealth,
            population,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use esf_parser::enums::EsfType;
    use esf_parser::objects::{
        EsfHeader, EsfItem, EsfNodeData, EsfValueRecord, NodeKind, NO_PARENT,
    };

    /// Hand-built arena: FACTION_ARRAY -> FACTION (values + ECONOMICS +
    /// FLAG_AND_COLOURS children), plus REGIONS_ARRAY -> REGION -> POPULATION
    /// -> REGION_FACTORS. String payloads live in `data` as UTF-16LE.
    struct Builder {
        data: Vec<u8>,
        names: Vec<String>,
        nodes: Vec<EsfNodeData>,
        items: Vec<EsfItem>,
        values: Vec<EsfValueRecord>,
    }

    impl Builder {
        fn new() -> Self {
            Self {
                data: Vec::new(),
                names: Vec::new(),
                nodes: Vec::new(),
                items: Vec::new(),
                values: Vec::new(),
            }
        }

        fn name_id(&mut self, name: &str) -> u16 {
            if let Some(i) = self.names.iter().position(|n| n == name) {
                return i as u16;
            }
            self.names.push(name.to_string());
            (self.names.len() - 1) as u16
        }

        fn utf16(&mut self, text: &str) -> EsfValue {
            let start = self.data.len() as u32;
            let units: Vec<u16> = text.encode_utf16().collect();
            for u in &units {
                self.data.extend_from_slice(&u.to_le_bytes());
            }
            EsfValue::Utf16 { start, chars: units.len() as u16 }
        }

        fn value(&mut self, v: EsfValue) -> EsfItem {
            self.values.push(EsfValueRecord { offset: 0, value: v });
            EsfItem::Value((self.values.len() - 1) as u32)
        }

        /// Add a node with the given items; returns its id.
        fn node(&mut self, name: &str, parent: NodeId, items: Vec<EsfItem>) -> NodeId {
            let name_index = self.name_id(name);
            let items_start = self.items.len() as u32;
            let items_len = items.len() as u32;
            self.items.extend(items);
            self.nodes.push(EsfNodeData {
                kind: NodeKind::Record,
                name_index,
                version: 0,
                offset: self.nodes.len() as u32 * 100,
                offset_end: self.nodes.len() as u32 * 100 + 100,
                parent,
                items_start,
                items_len,
            });
            (self.nodes.len() - 1) as NodeId
        }

        /// Fix up a child's parent link (nodes are built bottom-up, so the
        /// parent id does not exist yet when the child is created).
        fn set_parent(&mut self, child: NodeId, parent: NodeId) {
            self.nodes[child as usize].parent = parent;
        }

        fn build(self) -> EsfDocument {
            EsfDocument {
                data: self.data,
                header: EsfHeader {
                    magic: EsfType::ABCE,
                    unknown1: 0,
                    unknown2: 0,
                    offset_node_names: 0,
                },
                node_names: self.names,
                nodes: self.nodes,
                items: self.items,
                values: self.values,
                root: 0,
            }
        }
    }

    /// Bottom-up construction (children before parents) because a node's
    /// item range must be contiguous in the arena. Mirrors the real save
    /// nesting: ARRAY poly -> record (inherits the poly name) -> member.
    fn sample_doc() -> EsfDocument {
        let mut b = Builder::new();

        // FACTION subtree.
        let econ_items = vec![b.value(EsfValue::I32(20_000))];
        let econ = b.node("FACTION_ECONOMICS", NO_PARENT, econ_items);
        let flag_path = b.utf16(r"data\ui\flags\britain");
        let flag_items = vec![
            b.value(flag_path),
            b.value(EsfValue::U8(200)),
            b.value(EsfValue::U8(30)),
            b.value(EsfValue::U8(40)),
        ];
        let flags = b.node("FACTION_FLAG_AND_COLOURS", NO_PARENT, flag_items);

        // FACTION values: i32 id, s key, s name, bool, bool, bool(major),
        // i32 capital, per the scan-report ordering (subset).
        let key = b.utf16("britain");
        let name = b.utf16("Great Britain");
        let faction_items = vec![
            b.value(EsfValue::I32(4711)),
            b.value(key),
            b.value(name),
            b.value(EsfValue::Bool(false)),
            b.value(EsfValue::Bool(true)),
            b.value(EsfValue::Bool(true)), // bool2: major
            b.value(EsfValue::I32(99)),    // capital region != 0: alive
            EsfItem::Node(econ),
            EsfItem::Node(flags),
        ];
        let faction = b.node("FACTION", NO_PARENT, faction_items);
        b.set_parent(econ, faction);
        b.set_parent(flags, faction);
        let f_record = b.node("FACTION_ARRAY", NO_PARENT, vec![EsfItem::Node(faction)]);
        b.set_parent(faction, f_record);
        let f_array = b.node("FACTION_ARRAY", NO_PARENT, vec![EsfItem::Node(f_record)]);
        b.set_parent(f_record, f_array);

        // REGION subtree.
        let factors_items = vec![
            b.value(EsfValue::U32(0)),
            b.value(EsfValue::U32(1)),
            b.value(EsfValue::U32(123_456)), // u2 current population
        ];
        let factors = b.node("REGION_FACTORS", NO_PARENT, factors_items);
        let pop = b.node("POPULATION", NO_PARENT, vec![EsfItem::Node(factors)]);
        b.set_parent(factors, pop);

        // REGION: s key, i32, u32 filler … u32#5 wealth, u32#9 owner,
        // s theatre. Uses the class-occurrence positions the extractor reads.
        let r_key = b.utf16("norway");
        let theatre = b.utf16("europe");
        let mut region_items = vec![b.value(r_key), b.value(EsfValue::I32(1))];
        for n in 0..5 {
            region_items.push(b.value(EsfValue::U32(n))); // u0..u4
        }
        region_items.push(b.value(EsfValue::U32(1500))); // u5 town wealth
        for n in 0..3 {
            region_items.push(b.value(EsfValue::U32(n))); // u6..u8
        }
        region_items.push(b.value(EsfValue::U32(4711))); // u9 owner faction
        region_items.push(b.value(theatre)); // s1 theatre
        region_items.push(EsfItem::Node(pop));
        let region = b.node("REGION", NO_PARENT, region_items);
        b.set_parent(pop, region);
        let r_record = b.node("REGIONS_ARRAY", NO_PARENT, vec![EsfItem::Node(region)]);
        b.set_parent(region, r_record);
        let r_array = b.node("REGIONS_ARRAY", NO_PARENT, vec![EsfItem::Node(r_record)]);
        b.set_parent(r_record, r_array);

        let root_items = vec![EsfItem::Node(f_array), EsfItem::Node(r_array)];
        let root = b.node("root", NO_PARENT, root_items);
        b.set_parent(f_array, root);
        b.set_parent(r_array, root);

        b.build()
    }

    #[test]
    fn extracts_faction_with_treasury_and_colour() {
        let doc = sample_doc();
        let factions = extract_factions(&doc);
        assert_eq!(factions.len(), 1);
        let f = &factions[0];
        assert_eq!(f.id, 4711);
        assert_eq!(f.key, "britain");
        assert_eq!(f.name, "Great Britain");
        assert!(f.is_major);
        assert!(!f.destroyed);
        assert_eq!(f.color, Some((200, 30, 40)));
        let (treasury_vid, amount) = f.treasury.expect("treasury");
        assert_eq!(amount, 20_000);
        // The value id must key into the document's value table (that is
        // what the staged-edit map is keyed by).
        assert_eq!(doc.values[treasury_vid as usize].value, EsfValue::I32(20_000));
    }

    #[test]
    fn extracts_region_with_owner_and_population() {
        let doc = sample_doc();
        let regions = extract_regions(&doc);
        assert_eq!(regions.len(), 1);
        let r = &regions[0];
        assert_eq!(r.key, "norway");
        assert_eq!(r.theatre, "europe");
        assert_eq!(r.owner_faction, 4711);
        assert_eq!(r.town_wealth.map(|(_, w)| w), Some(1500));
        assert_eq!(r.population, Some(123_456));
    }

    #[test]
    fn missing_arrays_yield_empty_lists() {
        let mut b = Builder::new();
        b.node("root", NO_PARENT, vec![]);
        let doc = b.build();
        assert!(extract_factions(&doc).is_empty());
        assert!(extract_regions(&doc).is_empty());
    }
}
