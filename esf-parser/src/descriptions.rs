//! Node/field documentation for the value grid.
//!
//! Two layered sources, merged at startup:
//! 1. The original editor's NodesDescriptions.xml: a .NET-serialized list of
//!    `NodeDescription { Name, ValuesDesciption: [string...] }` entries with
//!    per-value descriptions (index-aligned). The "Desciption" typo is part
//!    of the legacy format. Sparse: ~26 of 678 entries are populated.
//! 2. twedit's own assets/esf_schema.toml: per-node `doc` plus field labels
//!    in two addressing modes — `fields` (absolute value position) and
//!    `typed` (nth occurrence of a type class, e.g. "s0" = first string,
//!    robust when optional values shift positions). Curated from etwng's
//!    semantic converter and empirical scans; wins over the legacy XML.

use crate::objects::EsfValue;
use quick_xml::events::Event;
use quick_xml::Reader;
use serde::Deserialize;
use std::collections::HashMap;

/// Documentation for one node name.
#[derive(Debug, Default, Clone)]
pub struct NodeSchema {
    /// One-line description of what the node is, shown above the value grid.
    pub doc: Option<String>,
    /// Labels by absolute value position (legacy XML and TOML `fields`).
    pub fields: Vec<Option<String>>,
    /// Labels by type-class occurrence, e.g. "s0", "i1", "u_ary0".
    pub typed: HashMap<String, String>,
}

/// Curated node descriptions and semantic field labels.
#[derive(Debug, Default, Clone)]
pub struct Descriptions {
    pub nodes: HashMap<String, NodeSchema>,
    pub loc_map: HashMap<String, String>,
}

impl Descriptions {
    /// Node-level doc line for the header above the value grid.
    pub fn doc(&self, name: &str) -> Option<&str> {
        self.nodes.get(name)?.doc.as_deref()
    }

    /// Label for the value at `pos`, where `classes` is the type class of
    /// every value of the node in order (see [`type_class`]). Typed labels
    /// win over positional ones.
    pub fn label(&self, node_name: &str, classes: &[&str], value_index: usize, raw_value_str: &str) -> Option<String> {
        let node = self.nodes.get(node_name)?;

        // Try nth-occurrence-of-type (e.g. i0, s1)
        if value_index < classes.len() {
            let cls = classes[value_index];
            let nth = classes[..value_index].iter().filter(|&&c| c == cls).count();
            let key = format!("{}{}", cls, nth);
            if let Some(lbl) = node.typed.get(&key) {
                return Some(self.augment_label_with_loc(lbl, raw_value_str));
            }
        }

        // Try absolute index (e.g. fields[0])
        if let Some(lbl) = node.fields.get(value_index).and_then(|o| o.as_ref()) {
            return Some(self.augment_label_with_loc(lbl, raw_value_str));
        }

        None
    }

    fn augment_label_with_loc(&self, lbl: &str, raw_value_str: &str) -> String {
        if let Some(loc) = self.loc_map.get(raw_value_str) {
            format!("{} ({})", lbl, loc)
        } else {
            lbl.to_string()
        }
    }
}

/// Type-class key of a value for `typed` addressing. The vocabulary follows
/// etwng's converters (i/u/s/bool/flt/byte/u2/v2/v3, `_ary` suffix for
/// arrays) so their annotations can be transcribed directly.
pub fn type_class(value: &EsfValue) -> &'static str {
    use crate::enums::ArrayElem;
    match value {
        EsfValue::Bool(_) => "bool",
        EsfValue::I8(_) => "i8",
        EsfValue::I16(_) | EsfValue::LegacyShort(_) => "i16",
        EsfValue::I32(_) => "i",
        EsfValue::I64(_) => "i64",
        EsfValue::U8(_) => "byte",
        EsfValue::U16(_) => "u2",
        EsfValue::U32(_) => "u",
        EsfValue::U64(_) => "u64",
        EsfValue::F32(_) => "flt",
        EsfValue::F64(_) => "f64",
        EsfValue::Angle(_) => "angle",
        EsfValue::Coord2D { .. } => "v2",
        EsfValue::Coord3D { .. } => "v3",
        EsfValue::Utf16 { .. } | EsfValue::Ascii { .. } => "s",
        EsfValue::Array { elem, .. } => match elem {
            ArrayElem::Bool => "bool_ary",
            ArrayElem::I32 => "i_ary",
            ArrayElem::U8 => "byte_ary",
            ArrayElem::U16 => "u2_ary",
            ArrayElem::U32 => "u_ary",
            ArrayElem::F32 => "flt_ary",
            ArrayElem::Coord2D => "v2_ary",
            _ => "ary",
        },
        EsfValue::Unknown6D(_) => "unk",
        EsfValue::SizedBlock { .. } => "blk",
    }
}

#[derive(Deserialize)]
struct TomlNode {
    doc: Option<String>,
    fields: Option<Vec<String>>,
    typed: Option<HashMap<String, String>>,
}

/// Parse both sources and merge: TOML doc/typed/fields win over the XML.
pub fn load(legacy_xml: &str, schema_toml: &str) -> Descriptions {
    let mut nodes: HashMap<String, NodeSchema> = HashMap::new();
    for (name, fields) in parse_descriptions(legacy_xml) {
        nodes.insert(name, NodeSchema { doc: None, fields, typed: HashMap::new() });
    }

    match toml::from_str::<HashMap<String, TomlNode>>(schema_toml) {
        Ok(map) => {
            for (name, entry) in map {
                let node = nodes.entry(name).or_default();
                if entry.doc.is_some() {
                    node.doc = entry.doc;
                }
                if let Some(fields) = entry.fields {
                    node.fields = fields
                        .into_iter()
                        .map(|s| if s.is_empty() { None } else { Some(s) })
                        .collect();
                }
                if let Some(typed) = entry.typed {
                    node.typed = typed;
                }
            }
        }
        Err(e) => {
            tracing::error!("esf_schema.toml failed to parse: {e}");
        }
    }

    Descriptions { nodes, loc_map: HashMap::new() }
}

/// Load the descriptions embedded in the crate (legacy XML + curated TOML).
/// Both front-ends call this so labels stay identical across UIs. Callers may
/// then set `loc_map` from [`crate::pack_parser::get_etw_localisation`].
pub fn embedded() -> Descriptions {
    load(
        include_str!("../assets/NodesDescriptions.xml"),
        include_str!("../assets/esf_schema.toml"),
    )
}

/// Parse the legacy NodesDescriptions.xml into name -> positional labels.
pub fn parse_descriptions(xml: &str) -> HashMap<String, Vec<Option<String>>> {
    let mut reader = Reader::from_str(xml);
    let mut map = HashMap::new();

    let mut current_name: Option<String> = None;
    let mut values: Vec<Option<String>> = Vec::new();
    let mut in_name = false;
    let mut in_string = false;
    let mut string_text = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => match e.name().as_ref() {
                b"NodeDescription" => {
                    current_name = None;
                    values.clear();
                }
                b"Name" => in_name = true,
                b"string" => {
                    in_string = true;
                    string_text.clear();
                }
                _ => {}
            },
            // Self-closing: <string xsi:nil="true" /> or <string /> = no description.
            Ok(Event::Empty(e)) => {
                if e.name().as_ref() == b"string" {
                    values.push(None);
                }
            }
            Ok(Event::Text(t)) => {
                let text = t.unescape().unwrap_or_default();
                if in_name {
                    current_name = Some(text.into_owned());
                } else if in_string {
                    string_text.push_str(&text);
                }
            }
            Ok(Event::End(e)) => match e.name().as_ref() {
                b"Name" => in_name = false,
                b"string" => {
                    in_string = false;
                    let text = string_text.trim();
                    values.push(if text.is_empty() {
                        None
                    } else {
                        Some(text.to_string())
                    });
                }
                b"NodeDescription" => {
                    if let Some(name) = current_name.take() {
                        map.insert(name, std::mem::take(&mut values));
                    }
                }
                _ => {}
            },
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_legacy_format() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<ArrayOfNodeDescription xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
  <NodeDescription><Name>EMPTY</Name><ValuesDesciption /></NodeDescription>
  <NodeDescription>
    <Name>MAPS</Name>
    <ValuesDesciption>
      <string>Theatre</string>
      <string xsi:nil="true" />
      <string />
      <string>A &amp; B</string>
    </ValuesDesciption>
  </NodeDescription>
</ArrayOfNodeDescription>"#;

        let map = parse_descriptions(xml);
        assert_eq!(map.get("EMPTY"), Some(&Vec::new()));
        let maps = map.get("MAPS").expect("MAPS entry");
        assert_eq!(maps.len(), 4);
        assert_eq!(maps[0].as_deref(), Some("Theatre"));
        assert_eq!(maps[1], None);
        assert_eq!(maps[2], None);
        assert_eq!(maps[3].as_deref(), Some("A & B"));
    }

    #[test]
    fn toml_overlays_legacy_and_typed_wins() {
        let xml = r#"<ArrayOfNodeDescription>
  <NodeDescription>
    <Name>REL</Name>
    <ValuesDesciption><string>legacy 0</string><string>legacy 1</string></ValuesDesciption>
  </NodeDescription>
</ArrayOfNodeDescription>"#;
        let toml = r#"
[REL]
doc = "A relationship."
[REL.typed]
s0 = "First string"
i1 = "Second int"
"#;
        let descs = load(xml, toml);
        assert_eq!(descs.doc("REL"), Some("A relationship."));

        // Values: i32, string, i32, i32 -> classes i, s, i, i.
        let classes = vec!["i", "s", "i", "i"];
        // Position 0: class i nth 0 -> no typed "i0", falls back to legacy.
        assert_eq!(descs.label("REL", &classes, 0, "").as_deref(), Some("legacy 0"));
        // Position 1: class s nth 0 -> typed s0 wins over legacy 1.
        assert_eq!(descs.label("REL", &classes, 1, "").as_deref(), Some("First string"));
        // Position 2: class i nth 1 -> typed i1.
        assert_eq!(descs.label("REL", &classes, 2, "").as_deref(), Some("Second int"));
        // Position 3: class i nth 2 -> nothing.
        assert_eq!(descs.label("REL", &classes, 3, ""), None);
    }

    #[test]
    fn real_assets_parse_and_resolve_diplomacy() {
        let descs = load(
            include_str!("../assets/NodesDescriptions.xml"),
            include_str!("../assets/esf_schema.toml"),
        );
        assert!(descs.doc("DIPLOMACY_RELATIONSHIP").is_some());
        assert!(descs.doc("FACTION").is_some());

        // The DIPLOMACY_RELATIONSHIP value signature observed in a real
        // Empire save (docs/scan_report.md).
        let classes = vec![
            "i", "bool", "i", "s", "i", "u", "i", "i", "i", "i", "i", "u", "u", "u", "u",
            "u_ary", "u", "s", "bool", "bool", "i",
        ];
        assert_eq!(
            descs.label("DIPLOMACY_RELATIONSHIP", &classes, 0, "").as_deref(),
            Some("Target faction ID")
        );
        assert_eq!(
            descs.label("DIPLOMACY_RELATIONSHIP", &classes, 1, "").as_deref(),
            Some("Trade agreement active")
        );
        assert_eq!(
            descs.label("DIPLOMACY_RELATIONSHIP", &classes, 3, "").as_deref(),
            Some("Relationship state (war / neutral / allied / patron / protectorate)")
        );
        assert_eq!(
            descs.label("DIPLOMACY_RELATIONSHIP", &classes, 11, "").as_deref(),
            Some("Turns at war")
        );
        assert_eq!(
            descs.label("DIPLOMACY_RELATIONSHIP", &classes, 20, "").as_deref(),
            Some("Overall relation (sum of attitude values)")
        );
    }

    #[test]
    fn label_appends_localised_name_when_value_is_a_loc_key() {
        let toml = r#"
[FACTION]
[FACTION.typed]
s0 = "Faction key"
"#;
        let mut descs = load("<ArrayOfNodeDescription/>", toml);
        descs.loc_map.insert(
            "factions_screen_name_britain".to_string(),
            "Great Britain".to_string(),
        );
        let classes = vec!["s"];
        assert_eq!(
            descs.label("FACTION", &classes, 0, "factions_screen_name_britain").as_deref(),
            Some("Faction key (Great Britain)")
        );
        // Values that aren't loc keys keep the plain label.
        assert_eq!(
            descs.label("FACTION", &classes, 0, "not_a_key").as_deref(),
            Some("Faction key")
        );
    }

    #[test]
    fn type_class_vocabulary() {
        use crate::enums::ArrayElem;
        assert_eq!(type_class(&EsfValue::I32(0)), "i");
        assert_eq!(type_class(&EsfValue::U32(0)), "u");
        assert_eq!(type_class(&EsfValue::Utf16 { start: 0, chars: 0 }), "s");
        assert_eq!(type_class(&EsfValue::Ascii { start: 0, len: 0 }), "s");
        assert_eq!(
            type_class(&EsfValue::Array { elem: ArrayElem::U32, start: 0, end: 0 }),
            "u_ary"
        );
    }
}
