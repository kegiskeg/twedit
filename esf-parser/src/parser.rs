use crate::enums::{EsfType, EsfValueType};
use crate::objects::{
    EsfDocument, EsfHeader, EsfItem, EsfNodeData, EsfValue, EsfValueRecord, NodeId, NodeKind,
    NO_PARENT,
};
use std::fmt;
use std::path::Path;
use tracing::{debug, error, info, instrument};

/// Represents an error that occurred during parsing or I/O.
#[derive(Debug)]
pub enum EsfError {
    Io(std::io::Error),
    UnexpectedEof { at: usize },
    UnsupportedMagic(u32),
    UnknownValueType { byte: u8, at: usize },
    InvalidOffset { at: usize, target: u32 },
    RootIsNotANode,
}

impl fmt::Display for EsfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EsfError::Io(e) => write!(f, "I/O error: {e}"),
            EsfError::UnexpectedEof { at } => write!(f, "unexpected end of data at 0x{at:x}"),
            EsfError::UnsupportedMagic(m) => write!(f, "unsupported ESF magic 0x{m:x}"),
            EsfError::UnknownValueType { byte, at } => {
                write!(f, "unknown value type 0x{byte:02x} at 0x{at:x}")
            }
            EsfError::InvalidOffset { at, target } => {
                write!(f, "invalid offset 0x{target:x} referenced at 0x{at:x}")
            }
            EsfError::RootIsNotANode => write!(f, "root element is not a node"),
        }
    }
}

impl std::error::Error for EsfError {}

impl From<std::io::Error> for EsfError {
    fn from(e: std::io::Error) -> Self {
        EsfError::Io(e)
    }
}

/// Little-endian cursor over an in-memory byte slice.
struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Reader { data, pos: 0 }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], EsfError> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|&end| end <= self.data.len())
            .ok_or_else(|| {
                error!("Unexpected EOF at offset 0x{:x} trying to read {} bytes", self.pos, n);
                EsfError::UnexpectedEof { at: self.pos }
            })?;
        let slice = &self.data[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, EsfError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, EsfError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }

    fn i16(&mut self) -> Result<i16, EsfError> {
        Ok(i16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }

    fn u32(&mut self) -> Result<u32, EsfError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn i32(&mut self) -> Result<i32, EsfError> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn u64(&mut self) -> Result<u64, EsfError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }

    fn f32(&mut self) -> Result<f32, EsfError> {
        Ok(f32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    /// Skip forward to an absolute offset (for sized blocks).
    fn skip_to(&mut self, target: u32) -> Result<(), EsfError> {
        let target_pos = target as usize;
        if target_pos < self.pos || target_pos > self.data.len() {
            error!("Invalid offset jump from 0x{:x} to 0x{:x}", self.pos, target);
            return Err(EsfError::InvalidOffset {
                at: self.pos,
                target,
            });
        }
        self.pos = target_pos;
        Ok(())
    }
}

/// One open structural node during the iterative parse.
struct Frame {
    node_id: NodeId,
    /// Absolute offset where this node's content ends.
    end: u32,
    /// Poly frames consume record entries; other frames consume values/nodes.
    is_poly: bool,
    /// Start of this frame's items in the shared scratch buffer.
    scratch_start: usize,
}

/// Parse an ESF file from disk. Reads the whole file into memory first —
/// ESF offsets are 32-bit absolute positions, so random access is required
/// anyway, and buffered access is what makes parsing fast.
#[instrument(skip(path))]
pub fn load_file(path: impl AsRef<Path>) -> Result<EsfDocument, EsfError> {
    let path = path.as_ref();
    info!("Loading ESF file from disk: {}", path.display());
    let data = std::fs::read(path)?;
    parse_bytes(data)
}

/// Parse an ESF document from an owned byte buffer.
#[instrument(skip(data))]
pub fn parse_bytes(data: Vec<u8>) -> Result<EsfDocument, EsfError> {
    debug!("Parsing ESF bytes (size: {} bytes)", data.len());
    let (header, node_names, content_start) = parse_header_and_names(&data)?;
    debug!("Header parsed: magic={:?}, names={}, content_start=0x{:x}", header.magic, node_names.len(), content_start);

    let mut doc = EsfDocument {
        data,
        header,
        node_names,
        nodes: Vec::new(),
        items: Vec::new(),
        values: Vec::new(),
        root: 0,
    };

    parse_tree(&mut doc, content_start)?;
    info!("ESF tree parsed successfully: {} nodes, {} values", doc.nodes.len(), doc.values.len());
    Ok(doc)
}

fn parse_header_and_names(data: &[u8]) -> Result<(EsfHeader, Vec<String>, usize), EsfError> {
    let mut reader = Reader::new(data);

    let magic_val = reader.u32()?;
    let magic =
        EsfType::try_from(magic_val).map_err(|_| EsfError::UnsupportedMagic(magic_val))?;

    let (unknown1, unknown2) = if magic == EsfType::ABCE {
        (reader.u32()?, reader.u32()?)
    } else {
        (0, 0)
    };

    let offset_node_names = reader.u32()?;
    let content_start = reader.pos;

    // Node name table sits at the end of the file.
    reader.pos = offset_node_names as usize;
    if reader.pos > data.len() {
        return Err(EsfError::InvalidOffset {
            at: content_start,
            target: offset_node_names,
        });
    }
    let num_names = reader.u16()?;
    let mut node_names = Vec::with_capacity(num_names as usize);
    for _ in 0..num_names {
        let len = reader.u16()?;
        let bytes = reader.take(len as usize)?;
        node_names.push(String::from_utf8_lossy(bytes).into_owned());
    }

    let header = EsfHeader {
        magic,
        unknown1,
        unknown2,
        offset_node_names,
    };
    Ok((header, node_names, content_start))
}

/// Iterative depth-first parse of the node tree. An explicit frame stack
/// replaces recursion, so arbitrarily deep files cannot overflow the thread
/// stack. Items are accumulated in a shared scratch buffer and flushed to the
/// document's item table when a frame closes, giving each node a contiguous
/// item range.
fn parse_tree(doc: &mut EsfDocument, content_start: usize) -> Result<(), EsfError> {
    let mut reader = Reader::new(&doc.data);
    reader.pos = content_start;

    // The root element must be a node.
    let root_offset = reader.pos as u32;
    let type_byte = reader.u8()?;
    let root_type = EsfValueType::try_from(type_byte).map_err(|_| EsfError::UnknownValueType {
        byte: type_byte,
        at: root_offset as usize,
    })?;
    if root_type != EsfValueType::SingleNode && root_type != EsfValueType::PolyNode {
        return Err(EsfError::RootIsNotANode);
    }

    let mut nodes: Vec<EsfNodeData> = Vec::new();
    let mut items: Vec<EsfItem> = Vec::new();
    let mut values: Vec<EsfValueRecord> = Vec::new();
    let mut scratch: Vec<EsfItem> = Vec::new();
    let mut stack: Vec<Frame> = Vec::new();

    open_node(
        &mut reader,
        &mut nodes,
        &mut stack,
        &mut scratch,
        root_offset,
        root_type == EsfValueType::PolyNode,
        NO_PARENT,
    )?;

    while let Some(frame) = stack.last() {
        // Close the frame when its content is exhausted.
        if reader.pos as u32 >= frame.end {
            let frame = stack.pop().unwrap();
            let start = items.len() as u32;
            items.extend(scratch.drain(frame.scratch_start..));
            let node = &mut nodes[frame.node_id as usize];
            node.items_start = start;
            node.items_len = items.len() as u32 - start;
            continue;
        }

        if frame.is_poly {
            // Poly content is a sequence of records: end offset, then values.
            let record_offset = reader.pos as u32;
            let record_end = reader.u32()?;
            let poly_id = frame.node_id;
            let record_id = nodes.len() as NodeId;
            let poly = &nodes[poly_id as usize];
            nodes.push(EsfNodeData {
                kind: NodeKind::Record,
                name_index: poly.name_index,
                version: poly.version,
                offset: record_offset,
                offset_end: record_end,
                parent: poly_id,
                items_start: 0,
                items_len: 0,
            });
            scratch.push(EsfItem::Node(record_id));
            stack.push(Frame {
                node_id: record_id,
                end: record_end,
                is_poly: false,
                scratch_start: scratch.len(),
            });
            continue;
        }

        let offset = reader.pos as u32;
        let type_byte = reader.u8()?;
        let value_type =
            EsfValueType::try_from(type_byte).map_err(|_| EsfError::UnknownValueType {
                byte: type_byte,
                at: offset as usize,
            })?;

        match value_type {
            EsfValueType::SingleNode | EsfValueType::PolyNode => {
                let parent = stack.last().unwrap().node_id;
                open_node(
                    &mut reader,
                    &mut nodes,
                    &mut stack,
                    &mut scratch,
                    offset,
                    value_type == EsfValueType::PolyNode,
                    parent,
                )?;
            }
            _ => {
                let value = parse_leaf_value(&mut reader, value_type, type_byte)?;
                let value_id = values.len() as u32;
                values.push(EsfValueRecord { offset, value });
                scratch.push(EsfItem::Value(value_id));
            }
        }
    }

    doc.nodes = nodes;
    doc.items = items;
    doc.values = values;
    doc.root = 0;
    Ok(())
}

/// Read a Single/Poly node header, append the node to the arena, register it
/// as an item of the enclosing frame, and push its frame.
fn open_node(
    reader: &mut Reader<'_>,
    nodes: &mut Vec<EsfNodeData>,
    stack: &mut Vec<Frame>,
    scratch: &mut Vec<EsfItem>,
    offset: u32,
    is_poly: bool,
    parent: NodeId,
) -> Result<NodeId, EsfError> {
    let name_index = reader.u16()?;
    let version = reader.u8()?;
    let offset_end = reader.u32()?;
    if is_poly {
        // Record count; records are delimited by offsets, so it is redundant.
        let _count = reader.u32()?;
    }

    let node_id = nodes.len() as NodeId;
    nodes.push(EsfNodeData {
        kind: if is_poly {
            NodeKind::Poly
        } else {
            NodeKind::Single
        },
        name_index,
        version,
        offset,
        offset_end,
        parent,
        items_start: 0,
        items_len: 0,
    });
    if parent != NO_PARENT {
        // Register this node in the enclosing frame's item range.
        scratch.push(EsfItem::Node(node_id));
    }
    stack.push(Frame {
        node_id,
        end: offset_end,
        is_poly,
        scratch_start: scratch.len(),
    });
    Ok(node_id)
}

fn parse_leaf_value(
    reader: &mut Reader<'_>,
    value_type: EsfValueType,
    type_byte: u8,
) -> Result<EsfValue, EsfError> {
    let value = match value_type {
        EsfValueType::Short => EsfValue::Short(reader.i16()?),
        EsfValueType::Boolean => EsfValue::Bool(reader.u8()? != 0),
        EsfValueType::Int => EsfValue::Int(reader.i32()?),
        EsfValueType::Byte => EsfValue::Byte(reader.u8()?),
        EsfValueType::UInt16 => EsfValue::UInt16(reader.u16()?),
        EsfValueType::UInt => EsfValue::UInt(reader.u32()?),
        EsfValueType::UInt64 => EsfValue::UInt64(reader.u64()?),
        EsfValueType::Float => EsfValue::Float(reader.f32()?),
        EsfValueType::FloatPoint => EsfValue::FloatPoint {
            x: reader.f32()?,
            y: reader.f32()?,
        },
        EsfValueType::FloatPoint3D => EsfValue::FloatPoint3D {
            x: reader.f32()?,
            y: reader.f32()?,
            z: reader.f32()?,
        },
        EsfValueType::UTF16 => {
            let chars = reader.u16()?;
            let start = reader.pos as u32;
            reader.take(chars as usize * 2)?;
            EsfValue::Utf16 { start, chars }
        }
        EsfValueType::Ascii => {
            let len = reader.u16()?;
            let start = reader.pos as u32;
            reader.take(len as usize)?;
            EsfValue::Ascii { start, len }
        }
        EsfValueType::UShort => EsfValue::UShort(reader.u16()?),
        EsfValueType::Binary41
        | EsfValueType::Binary42
        | EsfValueType::Binary43
        | EsfValueType::Binary44
        | EsfValueType::Binary45
        | EsfValueType::Binary46
        | EsfValueType::Binary47
        | EsfValueType::Binary48
        | EsfValueType::Binary49
        | EsfValueType::Binary4A
        | EsfValueType::Binary4B
        | EsfValueType::Binary4C
        | EsfValueType::Binary4D => {
            let end = reader.u32()?;
            let start = reader.pos as u32;
            reader.skip_to(end)?;
            EsfValue::Binary {
                type_byte,
                start,
                end,
            }
        }
        EsfValueType::Unknown109 => {
            let bytes: [u8; 4] = reader.take(4)?.try_into().unwrap();
            EsfValue::Unknown109(bytes)
        }
        EsfValueType::OptimizedBlock140 => {
            let end = reader.u32()?;
            let start = reader.pos as u32;
            reader.skip_to(end)?;
            EsfValue::OptimizedBlock { start, end }
        }
        EsfValueType::SingleNode | EsfValueType::PolyNode => {
            unreachable!("structural nodes are handled by the frame stack")
        }
    };
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::objects::{EsfValue, NodeKind};

    /// Minimal ESF byte builder for tests.
    struct EsfBuilder {
        buf: Vec<u8>,
    }

    impl EsfBuilder {
        fn new() -> Self {
            EsfBuilder { buf: Vec::new() }
        }

        fn u8(&mut self, v: u8) -> &mut Self {
            self.buf.push(v);
            self
        }

        fn u16(&mut self, v: u16) -> &mut Self {
            self.buf.extend_from_slice(&v.to_le_bytes());
            self
        }

        fn u32(&mut self, v: u32) -> &mut Self {
            self.buf.extend_from_slice(&v.to_le_bytes());
            self
        }

        fn i32(&mut self, v: i32) -> &mut Self {
            self.buf.extend_from_slice(&v.to_le_bytes());
            self
        }

        fn f32(&mut self, v: f32) -> &mut Self {
            self.buf.extend_from_slice(&v.to_le_bytes());
            self
        }

        fn bytes(&mut self, v: &[u8]) -> &mut Self {
            self.buf.extend_from_slice(v);
            self
        }

        /// Reserve a u32 slot to be patched later; returns its position.
        fn placeholder_u32(&mut self) -> usize {
            let pos = self.buf.len();
            self.u32(0);
            pos
        }

        fn patch_u32(&mut self, pos: usize, v: u32) {
            self.buf[pos..pos + 4].copy_from_slice(&v.to_le_bytes());
        }

        fn pos(&self) -> u32 {
            self.buf.len() as u32
        }
    }

    /// Builds an ABCD file:
    /// root (single, "root") {
    ///   Bool(true), Int(-42), Ascii("hello"), Utf16("hi"),
    ///   child (single, "child") { Float(1.5) },
    ///   list (poly, "list") [ { Byte(7) }, { UInt(99) } ],
    /// }
    fn build_sample() -> Vec<u8> {
        let mut b = EsfBuilder::new();
        b.u32(0xABCD);
        let names_offset_slot = b.placeholder_u32();

        // root single node
        b.u8(0x80).u16(0).u8(1);
        let root_end_slot = b.placeholder_u32();

        b.u8(1).u8(1); // Bool(true)
        b.u8(4).i32(-42); // Int
        b.u8(0x0f).u16(5).bytes(b"hello"); // Ascii
        b.u8(0x0e).u16(2); // Utf16 "hi"
        for unit in "hi".encode_utf16() {
            b.u16(unit);
        }

        // child single node
        b.u8(0x80).u16(1).u8(2);
        let child_end_slot = b.placeholder_u32();
        b.u8(0x0a).f32(1.5);
        let child_end = b.pos();
        b.patch_u32(child_end_slot, child_end);

        // poly node with 2 records
        b.u8(0x81).u16(2).u8(3);
        let poly_end_slot = b.placeholder_u32();
        b.u32(2); // record count
        let rec1_end_slot = b.placeholder_u32();
        b.u8(6).u8(7); // Byte(7)
        let rec1_end = b.pos();
        b.patch_u32(rec1_end_slot, rec1_end);
        let rec2_end_slot = b.placeholder_u32();
        b.u8(8).u32(99); // UInt(99)
        let rec2_end = b.pos();
        b.patch_u32(rec2_end_slot, rec2_end);
        let poly_end = b.pos();
        b.patch_u32(poly_end_slot, poly_end);

        let root_end = b.pos();
        b.patch_u32(root_end_slot, root_end);

        // node name table
        let names_offset = b.pos();
        b.patch_u32(names_offset_slot, names_offset);
        b.u16(3);
        for name in ["root", "child", "list"] {
            b.u16(name.len() as u16).bytes(name.as_bytes());
        }

        b.buf
    }

    #[test]
    fn parses_synthetic_file() {
        let doc = parse_bytes(build_sample()).expect("parse failed");

        assert_eq!(doc.node_names, vec!["root", "child", "list"]);
        assert_eq!(doc.node_name(doc.root), "root");
        assert_eq!(doc.node(doc.root).version, 1);
        assert_eq!(doc.node(doc.root).parent, crate::objects::NO_PARENT);

        // Root: 4 values + 2 child nodes, order preserved.
        let root_values: Vec<_> = doc.node_values(doc.root).collect();
        assert_eq!(root_values.len(), 4);
        assert_eq!(root_values[0].value, EsfValue::Bool(true));
        assert_eq!(root_values[1].value, EsfValue::Int(-42));
        assert_eq!(doc.decode_string(&root_values[2].value).as_deref(), Some("hello"));
        assert_eq!(doc.decode_string(&root_values[3].value).as_deref(), Some("hi"));

        let children: Vec<_> = doc.children(doc.root).collect();
        assert_eq!(children.len(), 2);

        let child = children[0];
        assert_eq!(doc.node_name(child), "child");
        assert_eq!(doc.node(child).kind, NodeKind::Single);
        assert_eq!(doc.node(child).version, 2);
        let child_values: Vec<_> = doc.node_values(child).collect();
        assert_eq!(child_values.len(), 1);
        assert_eq!(child_values[0].value, EsfValue::Float(1.5));

        let poly = children[1];
        assert_eq!(doc.node_name(poly), "list");
        assert_eq!(doc.node(poly).kind, NodeKind::Poly);
        let records: Vec<_> = doc.children(poly).collect();
        assert_eq!(records.len(), 2);
        for record in &records {
            assert_eq!(doc.node(*record).kind, NodeKind::Record);
            assert_eq!(doc.node(*record).parent, poly);
            assert_eq!(doc.node_name(*record), "list");
        }
        let rec1_values: Vec<_> = doc.node_values(records[0]).collect();
        assert_eq!(rec1_values[0].value, EsfValue::Byte(7));
        let rec2_values: Vec<_> = doc.node_values(records[1]).collect();
        assert_eq!(rec2_values[0].value, EsfValue::UInt(99));
    }

    #[test]
    fn offsets_are_dfs_ordered_and_searchable() {
        let doc = parse_bytes(build_sample()).expect("parse failed");

        let mut prev = 0;
        for node in &doc.nodes {
            assert!(node.offset >= prev, "arena must be DFS pre-order");
            prev = node.offset;
        }

        for (idx, node) in doc.nodes.iter().enumerate() {
            assert_eq!(doc.find_node_by_offset(node.offset), Some(idx as u32));
        }
        assert_eq!(doc.find_node_by_offset(2), None);
    }

    #[test]
    fn search_and_paths() {
        let doc = parse_bytes(build_sample()).expect("parse failed");

        let hits = doc.search_nodes("chi", 100);
        assert_eq!(hits.len(), 1);
        assert_eq!(doc.node_name(hits[0]), "child");
        assert_eq!(doc.node_path(hits[0]), "root/child");

        // Records are skipped in search (they share the poly's name).
        let list_hits = doc.search_nodes("list", 100);
        assert_eq!(list_hits.len(), 1);
        assert_eq!(doc.node(list_hits[0]).kind, NodeKind::Poly);

        let record = doc.children(list_hits[0]).next().unwrap();
        assert_eq!(doc.node_path(record), "root/list/list[]");

        assert!(doc.search_nodes("", 100).is_empty());
        assert!(doc.search_nodes("nomatch", 100).is_empty());
    }

    #[test]
    fn rejects_bad_input() {
        assert!(matches!(
            parse_bytes(vec![0xEF, 0xBE, 0xAD, 0xDE, 0, 0, 0, 0]),
            Err(EsfError::UnsupportedMagic(0xDEADBEEF))
        ));
        assert!(parse_bytes(Vec::new()).is_err());
        // Truncated mid-node must error, not panic.
        let mut truncated = build_sample();
        truncated.truncate(20);
        assert!(parse_bytes(truncated).is_err());
    }

    #[test]
    fn edits_round_trip() {
        use crate::objects::EsfEdit;
        use std::collections::HashMap;

        let doc = parse_bytes(build_sample()).expect("parse failed");

        // Locate value ids: root Int(-42), child Float(1.5), record Byte(7).
        let root_entries: Vec<_> = doc.node_value_entries(doc.root).collect();
        let int_id = root_entries[1].0;
        assert_eq!(root_entries[1].1.value, EsfValue::Int(-42));

        let children: Vec<_> = doc.children(doc.root).collect();
        let (float_id, float_record) = doc.node_value_entries(children[0]).next().unwrap();
        assert_eq!(float_record.value, EsfValue::Float(1.5));
        let record_node = doc.children(children[1]).next().unwrap();
        let (byte_id, _) = doc.node_value_entries(record_node).next().unwrap();

        let mut edits = HashMap::new();
        edits.insert(int_id, EsfEdit::Value(EsfValue::Int(123456)));
        edits.insert(float_id, EsfEdit::Value(EsfValue::Float(-2.75)));
        edits.insert(byte_id, EsfEdit::Value(EsfValue::Byte(200)));
        // Variant mismatch must be skipped, not corrupt the file.
        edits.insert(byte_id + 1_000, EsfEdit::Value(EsfValue::Int(1)));

        let (bytes, applied) = doc.bytes_with_edits(&edits);
        assert_eq!(applied, 3);
        assert_eq!(bytes.len(), doc.data.len(), "in-place edits must not resize");

        let redoc = parse_bytes(bytes).expect("edited file must re-parse");
        let root_values: Vec<_> = redoc.node_values(redoc.root).collect();
        assert_eq!(root_values[1].value, EsfValue::Int(123456));
        // Untouched values survive.
        assert_eq!(root_values[0].value, EsfValue::Bool(true));
        assert_eq!(redoc.decode_string(&root_values[2].value).as_deref(), Some("hello"));

        let rechildren: Vec<_> = redoc.children(redoc.root).collect();
        let refloat = redoc.node_values(rechildren[0]).next().unwrap();
        assert_eq!(refloat.value, EsfValue::Float(-2.75));
        let rerecord = redoc.children(rechildren[1]).next().unwrap();
        let rebyte = redoc.node_values(rerecord).next().unwrap();
        assert_eq!(rebyte.value, EsfValue::Byte(200));
    }

    #[test]
    fn string_edits_rewrite_and_fix_offsets() {
        use crate::objects::EsfEdit;
        use std::collections::HashMap;

        let doc = parse_bytes(build_sample()).expect("parse failed");
        let root_entries: Vec<_> = doc.node_value_entries(doc.root).collect();
        let (ascii_id, ascii_rec) = root_entries[2];
        let (utf16_id, utf16_rec) = root_entries[3];
        assert_eq!(doc.decode_string(&ascii_rec.value).as_deref(), Some("hello"));
        assert_eq!(doc.decode_string(&utf16_rec.value).as_deref(), Some("hi"));
        let int_id = root_entries[1].0;

        // Grow the ascii string, shrink the utf16 string, and mix in an
        // in-place scalar edit whose value sits after both splices.
        let mut edits = HashMap::new();
        edits.insert(ascii_id, EsfEdit::Text("hey there".to_string()));
        edits.insert(utf16_id, EsfEdit::Text("!".to_string()));
        edits.insert(int_id, EsfEdit::Value(EsfValue::Int(7)));
        // Text on a non-string value must be skipped.
        let mut edits2 = edits.clone();
        edits2.insert(root_entries[0].0, EsfEdit::Text("nope".to_string()));

        let (bytes, applied) = doc.bytes_with_edits(&edits2);
        assert_eq!(applied, 3, "bool Text edit must be skipped");
        assert_ne!(bytes.len(), doc.data.len(), "length must change");

        let redoc = parse_bytes(bytes).expect("rewritten file must re-parse");
        assert_eq!(redoc.nodes.len(), doc.nodes.len());
        assert_eq!(redoc.values.len(), doc.values.len());
        assert_eq!(redoc.node_names, doc.node_names, "name table must survive");

        let re_entries: Vec<_> = redoc.node_value_entries(redoc.root).collect();
        assert_eq!(re_entries[0].1.value, EsfValue::Bool(true));
        assert_eq!(re_entries[1].1.value, EsfValue::Int(7));
        assert_eq!(redoc.decode_string(&re_entries[2].1.value).as_deref(), Some("hey there"));
        assert_eq!(redoc.decode_string(&re_entries[3].1.value).as_deref(), Some("!"));

        // Structure after the splices survives intact.
        let rechildren: Vec<_> = redoc.children(redoc.root).collect();
        assert_eq!(rechildren.len(), 2);
        let refloat = redoc.node_values(rechildren[0]).next().unwrap();
        assert_eq!(refloat.value, EsfValue::Float(1.5));
        let records: Vec<_> = redoc.children(rechildren[1]).collect();
        assert_eq!(records.len(), 2);
        assert_eq!(redoc.node_values(records[0]).next().unwrap().value, EsfValue::Byte(7));
        assert_eq!(redoc.node_values(records[1]).next().unwrap().value, EsfValue::UInt(99));

        // A second rewrite on the re-parsed doc must also work (utf16 grow).
        let re_utf16_id = re_entries[3].0;
        let mut edits3 = HashMap::new();
        edits3.insert(re_utf16_id, EsfEdit::Text("longer again".to_string()));
        let (bytes3, applied3) = redoc.bytes_with_edits(&edits3);
        assert_eq!(applied3, 1);
        let redoc3 = parse_bytes(bytes3).expect("second rewrite must re-parse");
        let e3: Vec<_> = redoc3.node_value_entries(redoc3.root).collect();
        assert_eq!(redoc3.decode_string(&e3[3].1.value).as_deref(), Some("longer again"));
    }

    #[test]
    fn parse_edit_stages_strings_and_scalars() {
        use crate::objects::EsfEdit;

        let doc = parse_bytes(build_sample()).expect("parse failed");
        let entries: Vec<_> = doc.node_value_entries(doc.root).collect();
        let ascii = &entries[2].1.value;
        let utf16 = &entries[3].1.value;

        assert_eq!(ascii.parse_edit("new text"), Some(EsfEdit::Text("new text".into())));
        assert_eq!(utf16.parse_edit(""), Some(EsfEdit::Text(String::new())));
        assert_eq!(
            EsfValue::Int(0).parse_edit("-5"),
            Some(EsfEdit::Value(EsfValue::Int(-5)))
        );
        assert_eq!(EsfValue::Int(0).parse_edit("x"), None);
        assert_eq!(EsfValue::Binary { type_byte: 0x41, start: 0, end: 0 }.parse_edit("x"), None);
    }

    #[test]
    fn parse_same_type_accepts_and_rejects() {
        assert_eq!(
            EsfValue::Int(0).parse_same_type(" -17 "),
            Some(EsfValue::Int(-17))
        );
        assert_eq!(EsfValue::Int(0).parse_same_type("abc"), None);
        assert_eq!(EsfValue::Int(0).parse_same_type("2.5"), None);
        assert_eq!(
            EsfValue::Bool(false).parse_same_type("TRUE"),
            Some(EsfValue::Bool(true))
        );
        assert_eq!(
            EsfValue::Bool(false).parse_same_type("0"),
            Some(EsfValue::Bool(false))
        );
        assert_eq!(EsfValue::Byte(0).parse_same_type("256"), None);
        assert_eq!(
            EsfValue::Float(0.0).parse_same_type("1.25"),
            Some(EsfValue::Float(1.25))
        );
        assert_eq!(
            EsfValue::FloatPoint { x: 0.0, y: 0.0 }.parse_same_type("(3, -4.5)"),
            Some(EsfValue::FloatPoint { x: 3.0, y: -4.5 })
        );
        assert_eq!(
            EsfValue::FloatPoint3D { x: 0.0, y: 0.0, z: 0.0 }.parse_same_type("1,2,3"),
            Some(EsfValue::FloatPoint3D { x: 1.0, y: 2.0, z: 3.0 })
        );
        assert_eq!(
            EsfValue::FloatPoint { x: 0.0, y: 0.0 }.parse_same_type("1,2,3"),
            None
        );
        // Strings go through parse_edit/EsfEdit::Text, not parse_same_type.
        assert_eq!(
            EsfValue::Ascii { start: 0, len: 0 }.parse_same_type("x"),
            None
        );
        assert!(EsfValue::Ascii { start: 0, len: 0 }.is_editable());
        assert!(EsfValue::Utf16 { start: 0, chars: 0 }.is_editable());
        assert!(!EsfValue::Unknown109([0; 4]).is_editable());
        assert!(EsfValue::UInt64(0).is_editable());
    }

    #[test]
    fn parses_real_save_if_present() {
        let path = r"C:\Projects\Rust\_old\esfeditor\saves\test_save.empire_save_multiplayer";
        if !std::path::Path::new(path).exists() {
            eprintln!("real save not present, skipping");
            return;
        }

        let start = std::time::Instant::now();
        let doc = load_file(path).expect("failed to parse real save");
        let elapsed = start.elapsed();

        assert!(!doc.nodes.is_empty());
        assert!(!doc.node_names.is_empty());
        println!(
            "parsed real save: {} nodes, {} values, {} names in {:?}",
            doc.nodes.len(),
            doc.values.len(),
            doc.node_names.len(),
            elapsed
        );
    }
}
