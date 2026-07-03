use crate::enums::EsfType;
use std::collections::HashMap;

/// Index of a structural node in [`EsfDocument::nodes`].
pub type NodeId = u32;

/// Sentinel parent id for the root node.
pub const NO_PARENT: NodeId = NodeId::MAX;

#[derive(Debug, Clone)]
pub struct EsfHeader {
    pub magic: EsfType,
    pub unknown1: u32,
    pub unknown2: u32,
    pub offset_node_names: u32,
}

/// Structural node kind.
///
/// `Record` corresponds to the old `EsfMultiNode`: one unnamed entry inside a
/// `Poly` node. It inherits the poly node's name/version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    Single,
    Poly,
    Record,
}

/// A structural node stored in the arena. Children and values are referenced
/// through a contiguous range in [`EsfDocument::items`].
#[derive(Debug, Clone)]
pub struct EsfNodeData {
    pub kind: NodeKind,
    pub name_index: u16,
    pub version: u8,
    pub offset: u32,
    pub offset_end: u32,
    pub parent: NodeId,
    pub items_start: u32,
    pub items_len: u32,
}

/// One entry in a node's content, in file order (values and child nodes
/// interleave, and that order must be preserved for saving).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EsfItem {
    Node(NodeId),
    Value(u32),
}

/// A leaf value plus the file offset of its type byte (needed for in-place
/// editing later).
#[derive(Debug, Clone)]
pub struct EsfValueRecord {
    pub offset: u32,
    pub value: EsfValue,
}

impl EsfValue {
    /// Whether this value can be edited in place. Fixed-size scalars keep
    /// the file layout unchanged; strings and blobs would shift every
    /// subsequent offset and need a full rewrite (not yet supported).
    pub fn is_editable(&self) -> bool {
        !matches!(
            self,
            EsfValue::Utf16 { .. }
                | EsfValue::Ascii { .. }
                | EsfValue::Binary { .. }
                | EsfValue::Unknown109(_)
                | EsfValue::OptimizedBlock { .. }
        )
    }

    /// Parse `text` as a new value of the same variant as `self`.
    /// Returns `None` when the text does not parse or the variant is not
    /// editable.
    pub fn parse_same_type(&self, text: &str) -> Option<EsfValue> {
        let text = text.trim();
        Some(match self {
            EsfValue::Bool(_) => match text.to_ascii_lowercase().as_str() {
                "true" | "1" => EsfValue::Bool(true),
                "false" | "0" => EsfValue::Bool(false),
                _ => return None,
            },
            EsfValue::Byte(_) => EsfValue::Byte(text.parse().ok()?),
            EsfValue::Short(_) => EsfValue::Short(text.parse().ok()?),
            EsfValue::UInt16(_) => EsfValue::UInt16(text.parse().ok()?),
            EsfValue::UShort(_) => EsfValue::UShort(text.parse().ok()?),
            EsfValue::Int(_) => EsfValue::Int(text.parse().ok()?),
            EsfValue::UInt(_) => EsfValue::UInt(text.parse().ok()?),
            EsfValue::UInt64(_) => EsfValue::UInt64(text.parse().ok()?),
            EsfValue::Float(_) => EsfValue::Float(text.parse().ok()?),
            EsfValue::FloatPoint { .. } => {
                let (x, y) = parse_float_pair(text)?;
                EsfValue::FloatPoint { x, y }
            }
            EsfValue::FloatPoint3D { .. } => {
                let mut parts = split_float_list(text);
                let x = parts.next()?.parse().ok()?;
                let y = parts.next()?.parse().ok()?;
                let z = parts.next()?.parse().ok()?;
                if parts.next().is_some() {
                    return None;
                }
                EsfValue::FloatPoint3D { x, y, z }
            }
            _ => return None,
        })
    }

    /// Little-endian payload bytes as stored in the file, excluding the
    /// type byte. `None` for variable-size values.
    pub fn payload_bytes(&self) -> Option<Vec<u8>> {
        Some(match self {
            EsfValue::Bool(v) => vec![*v as u8],
            EsfValue::Byte(v) => vec![*v],
            EsfValue::Short(v) => v.to_le_bytes().to_vec(),
            EsfValue::UInt16(v) | EsfValue::UShort(v) => v.to_le_bytes().to_vec(),
            EsfValue::Int(v) => v.to_le_bytes().to_vec(),
            EsfValue::UInt(v) => v.to_le_bytes().to_vec(),
            EsfValue::UInt64(v) => v.to_le_bytes().to_vec(),
            EsfValue::Float(v) => v.to_le_bytes().to_vec(),
            EsfValue::FloatPoint { x, y } => {
                let mut out = x.to_le_bytes().to_vec();
                out.extend_from_slice(&y.to_le_bytes());
                out
            }
            EsfValue::FloatPoint3D { x, y, z } => {
                let mut out = x.to_le_bytes().to_vec();
                out.extend_from_slice(&y.to_le_bytes());
                out.extend_from_slice(&z.to_le_bytes());
                out
            }
            _ => return None,
        })
    }

    /// True when both values are the same enum variant.
    pub fn same_variant(&self, other: &EsfValue) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}

fn split_float_list(text: &str) -> impl Iterator<Item = &str> {
    text.trim_matches(|c| c == '(' || c == ')')
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn parse_float_pair(text: &str) -> Option<(f32, f32)> {
    let mut parts = split_float_list(text);
    let x = parts.next()?.parse().ok()?;
    let y = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((x, y))
}

/// Decoded leaf value. Strings and binary blobs are stored as ranges into
/// [`EsfDocument::data`] and decoded on demand, so a 100MB save does not
/// duplicate its string/blob payloads on the heap.
#[derive(Debug, Clone, PartialEq)]
pub enum EsfValue {
    Bool(bool),
    Byte(u8),
    Short(i16),
    UInt16(u16),
    UShort(u16),
    Int(i32),
    UInt(u32),
    UInt64(u64),
    Float(f32),
    FloatPoint { x: f32, y: f32 },
    FloatPoint3D { x: f32, y: f32, z: f32 },
    /// UTF-16LE string: `chars` is the number of 2-byte code units at `start`.
    Utf16 { start: u32, chars: u16 },
    /// ASCII string: `len` bytes at `start`.
    Ascii { start: u32, len: u16 },
    /// Opaque sized block (types 0x41-0x4d); byte range into the file data.
    Binary { type_byte: u8, start: u32, end: u32 },
    Unknown109([u8; 4]),
    /// Optimized block (type 140); byte range into the file data.
    OptimizedBlock { start: u32, end: u32 },
}

/// A fully parsed ESF file: the raw bytes plus a flat arena describing its
/// tree. Node ids are DFS pre-order, so `nodes` is sorted by `offset`.
#[derive(Debug)]
pub struct EsfDocument {
    pub data: Vec<u8>,
    pub header: EsfHeader,
    pub node_names: Vec<String>,
    pub nodes: Vec<EsfNodeData>,
    pub items: Vec<EsfItem>,
    pub values: Vec<EsfValueRecord>,
    pub root: NodeId,
}

impl EsfDocument {
    pub fn node(&self, id: NodeId) -> &EsfNodeData {
        &self.nodes[id as usize]
    }

    pub fn node_name(&self, id: NodeId) -> &str {
        let node = self.node(id);
        self.node_names
            .get(node.name_index as usize)
            .map(String::as_str)
            .unwrap_or("<unknown>")
    }

    /// All items (values and child nodes) of a node, in file order.
    pub fn node_items(&self, id: NodeId) -> &[EsfItem] {
        let node = self.node(id);
        let start = node.items_start as usize;
        &self.items[start..start + node.items_len as usize]
    }

    /// Child structural nodes only.
    pub fn children(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        self.node_items(id).iter().filter_map(|item| match item {
            EsfItem::Node(child) => Some(*child),
            EsfItem::Value(_) => None,
        })
    }

    pub fn child_count(&self, id: NodeId) -> usize {
        self.children(id).count()
    }

    /// Leaf values only.
    pub fn node_values(&self, id: NodeId) -> impl Iterator<Item = &EsfValueRecord> + '_ {
        self.node_value_entries(id).map(|(_, record)| record)
    }

    /// Leaf values with their global value ids (keys for edit maps).
    pub fn node_value_entries(
        &self,
        id: NodeId,
    ) -> impl Iterator<Item = (u32, &EsfValueRecord)> + '_ {
        self.node_items(id).iter().filter_map(|item| match item {
            EsfItem::Node(_) => None,
            EsfItem::Value(v) => Some((*v, &self.values[*v as usize])),
        })
    }

    /// Produce the full file bytes with `edits` applied. Edits are keyed by
    /// global value id and must be same-variant fixed-size values, so every
    /// patch is written in place at the value's payload offset (the
    /// equivalent of the original editor's QuickSave). Mismatched or
    /// non-editable edits are skipped and reported back.
    pub fn bytes_with_edits(&self, edits: &HashMap<u32, EsfValue>) -> (Vec<u8>, usize) {
        let mut out = self.data.clone();
        let mut applied = 0;
        for (&value_id, new_value) in edits {
            let Some(record) = self.values.get(value_id as usize) else {
                continue;
            };
            if !record.value.same_variant(new_value) {
                continue;
            }
            let Some(payload) = new_value.payload_bytes() else {
                continue;
            };
            // Payload starts right after the 1-byte type tag.
            let start = record.offset as usize + 1;
            let end = start + payload.len();
            if end <= out.len() {
                out[start..end].copy_from_slice(&payload);
                applied += 1;
            }
        }
        (out, applied)
    }

    /// Write the document with `edits` applied to `path`. Returns the number
    /// of edits actually applied.
    pub fn save_with_edits(
        &self,
        path: impl AsRef<std::path::Path>,
        edits: &HashMap<u32, EsfValue>,
    ) -> std::io::Result<usize> {
        let (bytes, applied) = self.bytes_with_edits(edits);
        std::fs::write(path, bytes)?;
        Ok(applied)
    }

    /// Locate the structural node whose type byte sits at `offset`.
    /// Nodes are DFS pre-order, hence sorted by offset: binary search.
    pub fn find_node_by_offset(&self, offset: u32) -> Option<NodeId> {
        self.nodes
            .binary_search_by_key(&offset, |n| n.offset)
            .ok()
            .map(|idx| idx as NodeId)
    }

    /// Case-insensitive substring search over node names. Returns at most
    /// `limit` matches.
    pub fn search_nodes(&self, query: &str, limit: usize) -> Vec<NodeId> {
        let query = query.to_lowercase();
        if query.is_empty() {
            return Vec::new();
        }
        let mut matching_names: Vec<bool> = Vec::with_capacity(self.node_names.len());
        for name in &self.node_names {
            matching_names.push(name.to_lowercase().contains(&query));
        }
        let mut results = Vec::new();
        for (idx, node) in self.nodes.iter().enumerate() {
            if node.kind == NodeKind::Record {
                continue; // records share their poly parent's name
            }
            if matching_names
                .get(node.name_index as usize)
                .copied()
                .unwrap_or(false)
            {
                results.push(idx as NodeId);
                if results.len() >= limit {
                    break;
                }
            }
        }
        results
    }

    /// Slash-separated path of node names from the root to `id`.
    pub fn node_path(&self, id: NodeId) -> String {
        let mut parts = Vec::new();
        let mut current = id;
        loop {
            let node = self.node(current);
            match node.kind {
                NodeKind::Record => parts.push(format!("{}[]", self.node_name(current))),
                _ => parts.push(self.node_name(current).to_string()),
            }
            if node.parent == NO_PARENT {
                break;
            }
            current = node.parent;
        }
        parts.reverse();
        parts.join("/")
    }

    /// Decode a value for display.
    pub fn format_value(&self, value: &EsfValue) -> String {
        match value {
            EsfValue::Bool(v) => v.to_string(),
            EsfValue::Byte(v) => v.to_string(),
            EsfValue::Short(v) => v.to_string(),
            EsfValue::UInt16(v) => v.to_string(),
            EsfValue::UShort(v) => v.to_string(),
            EsfValue::Int(v) => v.to_string(),
            EsfValue::UInt(v) => v.to_string(),
            EsfValue::UInt64(v) => v.to_string(),
            EsfValue::Float(v) => v.to_string(),
            EsfValue::FloatPoint { x, y } => format!("({x}, {y})"),
            EsfValue::FloatPoint3D { x, y, z } => format!("({x}, {y}, {z})"),
            EsfValue::Utf16 { .. } | EsfValue::Ascii { .. } => {
                self.decode_string(value).unwrap_or_default()
            }
            EsfValue::Binary { type_byte, start, end } => {
                format!("<binary 0x{:02x}, {} bytes>", type_byte, end.saturating_sub(*start))
            }
            EsfValue::Unknown109(bytes) => format!("<unknown109 {bytes:02x?}>"),
            EsfValue::OptimizedBlock { start, end } => {
                format!("<optimized block, {} bytes>", end.saturating_sub(*start))
            }
        }
    }

    /// Decode a string value; `None` for non-string values.
    pub fn decode_string(&self, value: &EsfValue) -> Option<String> {
        match value {
            EsfValue::Utf16 { start, chars } => {
                let start = *start as usize;
                let end = start + (*chars as usize) * 2;
                let bytes = self.data.get(start..end)?;
                let units: Vec<u16> = bytes
                    .chunks_exact(2)
                    .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                    .collect();
                Some(String::from_utf16_lossy(&units))
            }
            EsfValue::Ascii { start, len } => {
                let start = *start as usize;
                let bytes = self.data.get(start..start + *len as usize)?;
                Some(String::from_utf8_lossy(bytes).into_owned())
            }
            _ => None,
        }
    }

    /// Raw bytes of a binary/optimized block value; `None` for other values.
    pub fn binary_bytes(&self, value: &EsfValue) -> Option<&[u8]> {
        match value {
            EsfValue::Binary { start, end, .. } | EsfValue::OptimizedBlock { start, end } => {
                self.data.get(*start as usize..*end as usize)
            }
            _ => None,
        }
    }

    /// Short type label for a value, for UI display.
    pub fn value_type_name(value: &EsfValue) -> &'static str {
        match value {
            EsfValue::Bool(_) => "Boolean",
            EsfValue::Byte(_) => "Byte",
            EsfValue::Short(_) => "Int16",
            EsfValue::UInt16(_) => "UInt16",
            EsfValue::UShort(_) => "UInt16 (0x10)",
            EsfValue::Int(_) => "Int32",
            EsfValue::UInt(_) => "UInt32",
            EsfValue::UInt64(_) => "UInt64",
            EsfValue::Float(_) => "Float",
            EsfValue::FloatPoint { .. } => "Point2D",
            EsfValue::FloatPoint3D { .. } => "Point3D",
            EsfValue::Utf16 { .. } => "UTF-16",
            EsfValue::Ascii { .. } => "ASCII",
            EsfValue::Binary { .. } => "Binary",
            EsfValue::Unknown109(_) => "Unknown109",
            EsfValue::OptimizedBlock { .. } => "OptimizedBlock",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_value() {
        // A minimal EsfDocument for testing format_value (string decoding needs data)
        let doc = EsfDocument {
            data: b"hello\0".to_vec(),
            header: EsfHeader { magic: EsfType::ABCE, unknown1: 0, unknown2: 0, offset_node_names: 0 },
            node_names: vec![],
            nodes: vec![],
            items: vec![],
            values: vec![],
            root: 0,
        };

        assert_eq!(doc.format_value(&EsfValue::Int(-42)), "-42");
        assert_eq!(doc.format_value(&EsfValue::FloatPoint { x: 1.0, y: 2.5 }), "(1, 2.5)");
        assert_eq!(doc.format_value(&EsfValue::Ascii { start: 0, len: 5 }), "hello");
    }

    #[test]
    fn test_value_type_name() {
        assert_eq!(EsfDocument::value_type_name(&EsfValue::Bool(true)), "Boolean");
        assert_eq!(EsfDocument::value_type_name(&EsfValue::Unknown109([0; 4])), "Unknown109");
    }
}
