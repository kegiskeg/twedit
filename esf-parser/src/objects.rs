use crate::enums::{ArrayElem, EsfType};
use std::collections::HashMap;

/// Index of a structural node in [`EsfDocument::nodes`].
pub type NodeId = u32;

/// Sentinel parent id for the root node.
pub const NO_PARENT: NodeId = NodeId::MAX;

/// Maximum array elements rendered by [`EsfDocument::format_value`] before
/// truncating with an ellipsis.
pub const ARRAY_DISPLAY_LIMIT: usize = 16;

#[derive(Debug, Clone)]
pub struct EsfHeader {
    pub magic: EsfType,
    /// ABCE only: always observed as zero.
    pub unknown1: u32,
    /// ABCE only: Unix timestamp of when the file was written.
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

/// A staged edit for one value.
///
/// `Value` replaces a fixed-size scalar in place. `Text` replaces a string
/// value (`Utf16`/`Ascii`); because the payload length can change, applying
/// one triggers a full rewrite that shifts the file and fixes up every
/// stored absolute offset.
#[derive(Debug, Clone, PartialEq)]
pub enum EsfEdit {
    Value(EsfValue),
    Text(String),
}

impl EsfValue {
    /// Whether this value can be edited. Fixed-size scalars are patched in
    /// place; strings are replaced via a rewrite with offset fixups. Arrays
    /// and opaque blocks are read-only for now.
    pub fn is_editable(&self) -> bool {
        !matches!(
            self,
            EsfValue::Array { .. } | EsfValue::Unknown6D(_) | EsfValue::SizedBlock { .. }
        )
    }

    /// Parse user `text` into a staged edit for this value. `None` when the
    /// text does not parse for this variant (often a typing intermediate
    /// like "-"), the string is too long for the u16 length field, or the
    /// variant is not editable.
    pub fn parse_edit(&self, text: &str) -> Option<EsfEdit> {
        match self {
            EsfValue::Utf16 { .. } => {
                if text.encode_utf16().count() > u16::MAX as usize {
                    return None;
                }
                Some(EsfEdit::Text(text.to_string()))
            }
            EsfValue::Ascii { .. } => {
                if text.len() > u16::MAX as usize {
                    return None;
                }
                Some(EsfEdit::Text(text.to_string()))
            }
            _ => self.parse_same_type(text).map(EsfEdit::Value),
        }
    }

    /// Parse `text` as a new value of the same variant as `self`.
    /// Returns `None` when the text does not parse or the variant is not
    /// editable in place.
    pub fn parse_same_type(&self, text: &str) -> Option<EsfValue> {
        let text = text.trim();
        Some(match self {
            EsfValue::Bool(_) => match text.to_ascii_lowercase().as_str() {
                "true" | "1" => EsfValue::Bool(true),
                "false" | "0" => EsfValue::Bool(false),
                _ => return None,
            },
            EsfValue::I8(_) => EsfValue::I8(text.parse().ok()?),
            EsfValue::I16(_) => EsfValue::I16(text.parse().ok()?),
            EsfValue::LegacyShort(_) => EsfValue::LegacyShort(text.parse().ok()?),
            EsfValue::I32(_) => EsfValue::I32(text.parse().ok()?),
            EsfValue::I64(_) => EsfValue::I64(text.parse().ok()?),
            EsfValue::U8(_) => EsfValue::U8(text.parse().ok()?),
            EsfValue::U16(_) => EsfValue::U16(text.parse().ok()?),
            EsfValue::U32(_) => EsfValue::U32(text.parse().ok()?),
            EsfValue::U64(_) => EsfValue::U64(text.parse().ok()?),
            EsfValue::F32(_) => EsfValue::F32(text.parse().ok()?),
            EsfValue::F64(_) => EsfValue::F64(text.parse().ok()?),
            EsfValue::Angle(_) => EsfValue::Angle(text.parse().ok()?),
            EsfValue::Coord2D { .. } => {
                let (x, y) = parse_float_pair(text)?;
                EsfValue::Coord2D { x, y }
            }
            EsfValue::Coord3D { .. } => {
                let mut parts = split_float_list(text);
                let x = parts.next()?.parse().ok()?;
                let y = parts.next()?.parse().ok()?;
                let z = parts.next()?.parse().ok()?;
                if parts.next().is_some() {
                    return None;
                }
                EsfValue::Coord3D { x, y, z }
            }
            _ => return None,
        })
    }

    /// Little-endian payload bytes as stored in the file, excluding the
    /// type byte. `None` for variable-size values.
    pub fn payload_bytes(&self) -> Option<Vec<u8>> {
        Some(match self {
            EsfValue::Bool(v) => vec![*v as u8],
            EsfValue::I8(v) => v.to_le_bytes().to_vec(),
            EsfValue::U8(v) => vec![*v],
            EsfValue::I16(v) | EsfValue::LegacyShort(v) => v.to_le_bytes().to_vec(),
            EsfValue::U16(v) | EsfValue::Angle(v) => v.to_le_bytes().to_vec(),
            EsfValue::I32(v) => v.to_le_bytes().to_vec(),
            EsfValue::U32(v) => v.to_le_bytes().to_vec(),
            EsfValue::I64(v) => v.to_le_bytes().to_vec(),
            EsfValue::U64(v) => v.to_le_bytes().to_vec(),
            EsfValue::F32(v) => v.to_le_bytes().to_vec(),
            EsfValue::F64(v) => v.to_le_bytes().to_vec(),
            EsfValue::Coord2D { x, y } => {
                let mut out = x.to_le_bytes().to_vec();
                out.extend_from_slice(&y.to_le_bytes());
                out
            }
            EsfValue::Coord3D { x, y, z } => {
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

/// Decoded leaf value. Strings and arrays are stored as ranges into
/// [`EsfDocument::data`] and decoded on demand, so a 100MB save does not
/// duplicate its payloads on the heap.
///
/// Variant names follow the community taxonomy (taw's spec / RPFM); see
/// [`crate::enums::EsfValueType`] for byte layouts and provenance notes.
#[derive(Debug, Clone, PartialEq)]
pub enum EsfValue {
    /// Tag 0x01.
    Bool(bool),
    /// Tag 0x02.
    I8(i8),
    /// Tag 0x03.
    I16(i16),
    /// Tag 0x00 — C# EsfEditor compatibility, not in the ESF spec.
    LegacyShort(i16),
    /// Tag 0x04.
    I32(i32),
    /// Tag 0x05.
    I64(i64),
    /// Tag 0x06.
    U8(u8),
    /// Tag 0x07.
    U16(u16),
    /// Tag 0x08.
    U32(u32),
    /// Tag 0x09.
    U64(u64),
    /// Tag 0x0A.
    F32(f32),
    /// Tag 0x0B.
    F64(f64),
    /// Tag 0x0C: map coordinates.
    Coord2D { x: f32, y: f32 },
    /// Tag 0x0D.
    Coord3D { x: f32, y: f32, z: f32 },
    /// Tag 0x0E: `chars` is the number of 2-byte code units at `start`.
    Utf16 { start: u32, chars: u16 },
    /// Tag 0x0F: `len` bytes at `start`.
    Ascii { start: u32, len: u16 },
    /// Tag 0x10: angle in degrees (0-360).
    Angle(u16),
    /// Tags 0x41-0x50: typed array; packed elements in `start..end`.
    Array { elem: ArrayElem, start: u32, end: u32 },
    /// Tag 0x6D: opaque 4 bytes — C# EsfEditor compatibility.
    Unknown6D([u8; 4]),
    /// Tag 0x8C: opaque sized block — C# EsfEditor compatibility.
    SizedBlock { start: u32, end: u32 },
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
    /// global value id. `EsfEdit::Value` patches must match the original
    /// variant and are written in place at the value's payload offset.
    /// `EsfEdit::Text` edits on string values re-encode the string; when the
    /// length changes, the file is rebuilt and every stored absolute offset
    /// (node ends, array/sized-block ends, the header's name-table pointer)
    /// is adjusted. Mismatched or non-editable edits are skipped and
    /// reported back through the applied count.
    pub fn bytes_with_edits(&self, edits: &HashMap<u32, EsfEdit>) -> (Vec<u8>, usize) {
        let mut out = self.data.clone();
        let mut applied = 0;

        // (start of the value in the file, bytes it occupied, replacement)
        struct Splice {
            start: u32,
            old_len: usize,
            new_bytes: Vec<u8>,
        }
        let mut splices: Vec<Splice> = Vec::new();

        for (&value_id, edit) in edits {
            let Some(record) = self.values.get(value_id as usize) else {
                continue;
            };
            match edit {
                EsfEdit::Value(new_value) => {
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
                EsfEdit::Text(text) => {
                    // Keep the original type tag, write a new u16 count and
                    // payload: [tag][count][payload].
                    let (old_payload_len, count, payload) = match record.value {
                        EsfValue::Utf16 { chars, .. } => {
                            let units: Vec<u16> = text.encode_utf16().collect();
                            let Ok(count) = u16::try_from(units.len()) else {
                                continue;
                            };
                            let bytes: Vec<u8> =
                                units.iter().flat_map(|u| u.to_le_bytes()).collect();
                            (chars as usize * 2, count, bytes)
                        }
                        EsfValue::Ascii { len, .. } => {
                            let Ok(count) = u16::try_from(text.len()) else {
                                continue;
                            };
                            (len as usize, count, text.as_bytes().to_vec())
                        }
                        _ => continue,
                    };
                    let mut new_bytes = Vec::with_capacity(3 + payload.len());
                    new_bytes.push(self.data[record.offset as usize]);
                    new_bytes.extend_from_slice(&count.to_le_bytes());
                    new_bytes.extend_from_slice(&payload);
                    splices.push(Splice {
                        start: record.offset,
                        old_len: 1 + 2 + old_payload_len,
                        new_bytes,
                    });
                    applied += 1;
                }
            }
        }

        if splices.is_empty() {
            return (out, applied);
        }

        // Rebuild the file with the splices applied.
        splices.sort_by_key(|s| s.start);
        let delta_at = |pos: u64| -> i64 {
            splices
                .iter()
                .take_while(|s| (s.start as u64) < pos)
                .map(|s| s.new_bytes.len() as i64 - s.old_len as i64)
                .sum()
        };
        let total_delta: i64 = splices
            .iter()
            .map(|s| s.new_bytes.len() as i64 - s.old_len as i64)
            .sum();
        let mut rebuilt = Vec::with_capacity((out.len() as i64 + total_delta) as usize);
        let mut cursor = 0usize;
        for s in &splices {
            rebuilt.extend_from_slice(&out[cursor..s.start as usize]);
            rebuilt.extend_from_slice(&s.new_bytes);
            cursor = s.start as usize + s.old_len;
        }
        rebuilt.extend_from_slice(&out[cursor..]);

        // Fix every absolute offset stored in the file. delta_at counts
        // splices strictly before a position, so a stored END offset shifts
        // iff a splice lies inside the region it closes, and a field's own
        // position shifts iff a splice lies before it.
        let header_ptr_pos: u64 = match self.header.magic {
            EsfType::ABCD => 4,
            _ => 12,
        };
        let mut fixup = |field_pos: u64, stored: u32| {
            let new_stored = (stored as i64 + delta_at(stored as u64)) as u32;
            let new_pos = (field_pos as i64 + delta_at(field_pos)) as usize;
            rebuilt[new_pos..new_pos + 4].copy_from_slice(&new_stored.to_le_bytes());
        };
        fixup(header_ptr_pos, self.header.offset_node_names);
        for node in &self.nodes {
            // Single/Poly store their end offset after [tag][name u16][ver];
            // a Record's own offset IS its end-offset field.
            let field_pos = match node.kind {
                NodeKind::Single | NodeKind::Poly => node.offset as u64 + 4,
                NodeKind::Record => node.offset as u64,
            };
            fixup(field_pos, node.offset_end);
        }
        for rec in &self.values {
            if let EsfValue::Array { end, .. } | EsfValue::SizedBlock { end, .. } = rec.value {
                fixup(rec.offset as u64 + 1, end);
            }
        }

        (rebuilt, applied)
    }

    /// Write the document with `edits` applied to `path`. Returns the number
    /// of edits actually applied.
    pub fn save_with_edits(
        &self,
        path: impl AsRef<std::path::Path>,
        edits: &HashMap<u32, EsfEdit>,
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

    /// Number of elements in an array value; `None` for non-arrays or when
    /// the byte range is not a whole multiple of the element size.
    pub fn array_len(&self, value: &EsfValue) -> Option<usize> {
        let EsfValue::Array { elem, start, end } = value else {
            return None;
        };
        let span = (*end as usize).checked_sub(*start as usize)?;
        match elem.fixed_size() {
            Some(size) => {
                if span % size == 0 {
                    Some(span / size)
                } else {
                    None
                }
            }
            None => {
                // String elements: walk u16 length prefixes.
                let mut pos = *start as usize;
                let end = *end as usize;
                let mut count = 0usize;
                while pos + 2 <= end {
                    let n =
                        u16::from_le_bytes([self.data[pos], self.data[pos + 1]]) as usize;
                    let elem_bytes = if *elem == ArrayElem::Utf16 { n * 2 } else { n };
                    pos += 2 + elem_bytes;
                    if pos > end {
                        return None;
                    }
                    count += 1;
                }
                if pos == end {
                    Some(count)
                } else {
                    None
                }
            }
        }
    }

    /// Decode up to `limit` elements of an array value into display strings,
    /// returning them plus the total element count. `None` for non-arrays or
    /// malformed ranges (caller should fall back to a raw-bytes label).
    pub fn array_element_strings(
        &self,
        value: &EsfValue,
        limit: usize,
    ) -> Option<(Vec<String>, usize)> {
        let EsfValue::Array { elem, start, end } = value else {
            return None;
        };
        let total = self.array_len(value)?;
        let bytes = self.data.get(*start as usize..*end as usize)?;
        let mut out = Vec::with_capacity(total.min(limit));

        match elem.fixed_size() {
            Some(size) => {
                for chunk in bytes.chunks_exact(size).take(limit) {
                    out.push(format_fixed_elem(*elem, chunk));
                }
            }
            None => {
                let mut pos = 0usize;
                while pos + 2 <= bytes.len() && out.len() < limit {
                    let n = u16::from_le_bytes([bytes[pos], bytes[pos + 1]]) as usize;
                    let elem_bytes = if *elem == ArrayElem::Utf16 { n * 2 } else { n };
                    let data = bytes.get(pos + 2..pos + 2 + elem_bytes)?;
                    out.push(if *elem == ArrayElem::Utf16 {
                        let units: Vec<u16> = data
                            .chunks_exact(2)
                            .map(|p| u16::from_le_bytes([p[0], p[1]]))
                            .collect();
                        String::from_utf16_lossy(&units)
                    } else {
                        String::from_utf8_lossy(data).into_owned()
                    });
                    pos += 2 + elem_bytes;
                }
            }
        }
        Some((out, total))
    }

    /// Decode a value for display.
    pub fn format_value(&self, value: &EsfValue) -> String {
        match value {
            EsfValue::Bool(v) => v.to_string(),
            EsfValue::I8(v) => v.to_string(),
            EsfValue::I16(v) | EsfValue::LegacyShort(v) => v.to_string(),
            EsfValue::I32(v) => v.to_string(),
            EsfValue::I64(v) => v.to_string(),
            EsfValue::U8(v) => v.to_string(),
            EsfValue::U16(v) => v.to_string(),
            EsfValue::U32(v) => v.to_string(),
            EsfValue::U64(v) => v.to_string(),
            EsfValue::F32(v) => v.to_string(),
            EsfValue::F64(v) => v.to_string(),
            EsfValue::Angle(v) => format!("{v}°"),
            EsfValue::Coord2D { x, y } => format!("({x}, {y})"),
            EsfValue::Coord3D { x, y, z } => format!("({x}, {y}, {z})"),
            EsfValue::Utf16 { .. } | EsfValue::Ascii { .. } => {
                self.decode_string(value).unwrap_or_default()
            }
            EsfValue::Array { elem, start, end } => {
                match self.array_element_strings(value, ARRAY_DISPLAY_LIMIT) {
                    Some((elems, total)) => {
                        if total > elems.len() {
                            format!("[{}, … {} items]", elems.join(", "), total)
                        } else {
                            format!("[{}]", elems.join(", "))
                        }
                    }
                    None => format!(
                        "<{} {} bytes>",
                        elem.display_name(),
                        end.saturating_sub(*start)
                    ),
                }
            }
            EsfValue::Unknown6D(bytes) => format!("<unknown 0x6D {bytes:02x?}>"),
            EsfValue::SizedBlock { start, end } => {
                format!("<sized block, {} bytes>", end.saturating_sub(*start))
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

    /// Raw bytes of an array/sized-block value; `None` for other values.
    pub fn raw_bytes(&self, value: &EsfValue) -> Option<&[u8]> {
        match value {
            EsfValue::Array { start, end, .. } | EsfValue::SizedBlock { start, end } => {
                self.data.get(*start as usize..*end as usize)
            }
            _ => None,
        }
    }

    /// Short type label for a value, for UI display. Names follow the
    /// community taxonomy; legacy C#-compat types are flagged.
    pub fn value_type_name(value: &EsfValue) -> &'static str {
        match value {
            EsfValue::Bool(_) => "Boolean",
            EsfValue::I8(_) => "Int8",
            EsfValue::I16(_) => "Int16",
            EsfValue::LegacyShort(_) => "Int16 (legacy 0x00)",
            EsfValue::I32(_) => "Int32",
            EsfValue::I64(_) => "Int64",
            EsfValue::U8(_) => "UInt8",
            EsfValue::U16(_) => "UInt16",
            EsfValue::U32(_) => "UInt32",
            EsfValue::U64(_) => "UInt64",
            EsfValue::F32(_) => "Float32",
            EsfValue::F64(_) => "Float64",
            EsfValue::Angle(_) => "Angle",
            EsfValue::Coord2D { .. } => "Point2D",
            EsfValue::Coord3D { .. } => "Point3D",
            EsfValue::Utf16 { .. } => "UTF-16",
            EsfValue::Ascii { .. } => "ASCII",
            EsfValue::Array { elem, .. } => elem.display_name(),
            EsfValue::Unknown6D(_) => "Unknown 0x6D",
            EsfValue::SizedBlock { .. } => "Sized Block 0x8C",
        }
    }
}

/// Format one fixed-size array element from its packed bytes.
fn format_fixed_elem(elem: ArrayElem, b: &[u8]) -> String {
    match elem {
        ArrayElem::Bool => (b[0] != 0).to_string(),
        ArrayElem::I8 => (b[0] as i8).to_string(),
        ArrayElem::U8 => b[0].to_string(),
        ArrayElem::I16 => i16::from_le_bytes([b[0], b[1]]).to_string(),
        ArrayElem::U16 => u16::from_le_bytes([b[0], b[1]]).to_string(),
        ArrayElem::Angle => format!("{}°", u16::from_le_bytes([b[0], b[1]])),
        ArrayElem::I32 => i32::from_le_bytes([b[0], b[1], b[2], b[3]]).to_string(),
        ArrayElem::U32 => u32::from_le_bytes([b[0], b[1], b[2], b[3]]).to_string(),
        ArrayElem::F32 => f32::from_le_bytes([b[0], b[1], b[2], b[3]]).to_string(),
        ArrayElem::I64 => i64::from_le_bytes(b[..8].try_into().unwrap()).to_string(),
        ArrayElem::U64 => u64::from_le_bytes(b[..8].try_into().unwrap()).to_string(),
        ArrayElem::F64 => f64::from_le_bytes(b[..8].try_into().unwrap()).to_string(),
        ArrayElem::Coord2D => {
            let x = f32::from_le_bytes(b[0..4].try_into().unwrap());
            let y = f32::from_le_bytes(b[4..8].try_into().unwrap());
            format!("({x}, {y})")
        }
        ArrayElem::Coord3D => {
            let x = f32::from_le_bytes(b[0..4].try_into().unwrap());
            let y = f32::from_le_bytes(b[4..8].try_into().unwrap());
            let z = f32::from_le_bytes(b[8..12].try_into().unwrap());
            format!("({x}, {y}, {z})")
        }
        ArrayElem::Utf16 | ArrayElem::Ascii => unreachable!("strings are variable-size"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_doc(data: Vec<u8>) -> EsfDocument {
        EsfDocument {
            data,
            header: EsfHeader {
                magic: EsfType::ABCE,
                unknown1: 0,
                unknown2: 0,
                offset_node_names: 0,
            },
            node_names: vec![],
            nodes: vec![],
            items: vec![],
            values: vec![],
            root: 0,
        }
    }

    #[test]
    fn test_format_value() {
        let doc = empty_doc(b"hello\0".to_vec());
        assert_eq!(doc.format_value(&EsfValue::I32(-42)), "-42");
        assert_eq!(doc.format_value(&EsfValue::Coord2D { x: 1.0, y: 2.5 }), "(1, 2.5)");
        assert_eq!(doc.format_value(&EsfValue::Ascii { start: 0, len: 5 }), "hello");
        assert_eq!(doc.format_value(&EsfValue::Angle(90)), "90°");
    }

    #[test]
    fn test_array_decoding() {
        // Three u32 elements: 7, 0, 300.
        let mut data = Vec::new();
        data.extend_from_slice(&7u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&300u32.to_le_bytes());
        let doc = empty_doc(data);
        let arr = EsfValue::Array { elem: ArrayElem::U32, start: 0, end: 12 };
        assert_eq!(doc.array_len(&arr), Some(3));
        assert_eq!(doc.format_value(&arr), "[7, 0, 300]");

        // Misaligned range -> raw fallback.
        let bad = EsfValue::Array { elem: ArrayElem::U32, start: 0, end: 10 };
        assert_eq!(doc.array_len(&bad), None);
        assert_eq!(doc.format_value(&bad), "<UInt32 Array 10 bytes>");
    }

    #[test]
    fn test_string_array_decoding() {
        // Two ascii elements: "ab", "c".
        let mut data = Vec::new();
        data.extend_from_slice(&2u16.to_le_bytes());
        data.extend_from_slice(b"ab");
        data.extend_from_slice(&1u16.to_le_bytes());
        data.extend_from_slice(b"c");
        let len = data.len() as u32;
        let doc = empty_doc(data);
        let arr = EsfValue::Array { elem: ArrayElem::Ascii, start: 0, end: len };
        assert_eq!(doc.array_len(&arr), Some(2));
        assert_eq!(doc.format_value(&arr), "[ab, c]");
    }

    #[test]
    fn test_array_display_truncation() {
        let mut data = Vec::new();
        for i in 0..20u32 {
            data.extend_from_slice(&i.to_le_bytes());
        }
        let doc = empty_doc(data);
        let arr = EsfValue::Array { elem: ArrayElem::U32, start: 0, end: 80 };
        let text = doc.format_value(&arr);
        assert!(text.ends_with("… 20 items]"), "got: {text}");
    }

    #[test]
    fn test_value_type_name() {
        assert_eq!(EsfDocument::value_type_name(&EsfValue::Bool(true)), "Boolean");
        assert_eq!(EsfDocument::value_type_name(&EsfValue::Unknown6D([0; 4])), "Unknown 0x6D");
        assert_eq!(EsfDocument::value_type_name(&EsfValue::Angle(0)), "Angle");
        assert_eq!(
            EsfDocument::value_type_name(&EsfValue::Array {
                elem: ArrayElem::U32,
                start: 0,
                end: 0
            }),
            "UInt32 Array"
        );
    }
}
