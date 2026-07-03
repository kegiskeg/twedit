//! Loader for the original editor's NodesDescriptions.xml: a .NET-serialized
//! list of `NodeDescription { Name, ValuesDesciption: [string...] }` entries
//! mapping a node name to per-value descriptions (index-aligned with the
//! node's values). The "Desciption" typo is part of the legacy format.

use quick_xml::events::Event;
use quick_xml::Reader;
use std::collections::HashMap;
use std::path::Path;

pub type Descriptions = HashMap<String, Vec<Option<String>>>;

pub fn load_descriptions(path: impl AsRef<Path>) -> Option<Descriptions> {
    let xml = std::fs::read_to_string(path).ok()?;
    Some(parse_descriptions(&xml))
}

pub fn parse_descriptions(xml: &str) -> Descriptions {
    let mut reader = Reader::from_str(xml);
    let mut map = Descriptions::new();

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
}
