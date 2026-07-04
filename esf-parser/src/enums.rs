//! Type tags and magic headers for the ESF format.
//!
//! Naming follows the community-established taxonomy, cross-confirmed by:
//! - taw's ESF specification: <https://t-a-w.blogspot.com/2012/03/esf-empire-total-war-object.html>
//! - RPFM (Rusted PackFile Manager): `rpfm_lib/src/files/esf/mod.rs`
//! - The original C# EsfEditor 1.4.4 (2009), which this project reimplements;
//!   where it disagreed with the spec, the spec wins and the legacy name is
//!   noted on the variant.

/// Magic headers identifying ESF files, read as a little-endian u32.
///
/// Four variants exist across the Total War series. Later variants change
/// how strings, sizes, and records are encoded:
/// - `ABCD`/`ABCE`: strings inline in the data, u32 absolute end-offsets.
/// - `ABCF`: strings moved to footer string tables (values hold u32 indexes).
/// - `ABCA`: additionally uses variable-length sizes ("uintvar"), compact
///   record tags with inlined version bits, and optimized primitive tags
///   (0x12-0x1D). Not used by Empire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EsfType {
    /// Earliest format: no unknown/timestamp header fields.
    /// Used by early Empire: Total War files.
    ABCD = 0xABCD,
    /// Adds two u32 header fields (always-zero + Unix timestamp) before the
    /// footer offset. Empire and Napoleon campaign saves use this.
    ABCE = 0xABCE,
    /// Strings live in footer string tables; values store u32 indexes.
    /// Used by Shogun 2. Parsing supported at the tag level, but string
    /// values would need the footer tables (not yet implemented).
    ABCF = 0xABCF,
    /// Variable-length sizes, compact records, optimized primitives.
    /// Rome 2 and later. NOT supported by this parser.
    ABCA = 0xABCA,
}

impl TryFrom<u32> for EsfType {
    type Error = String;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            0xABCD => Ok(EsfType::ABCD),
            0xABCE => Ok(EsfType::ABCE),
            0xABCF => Ok(EsfType::ABCF),
            0xABCA => Ok(EsfType::ABCA),
            _ => Err(format!("Unsupported ESF magic header type: 0x{:04X}", value)),
        }
    }
}

/// Element type of a typed array value (tags 0x41-0x50).
///
/// The array tag is `0x40 + primitive tag`; elements are packed back to back
/// with no per-element tag byte. String elements (`Utf16`/`Ascii`) keep their
/// u16 count prefix per element.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArrayElem {
    Bool,
    I8,
    I16,
    I32,
    I64,
    U8,
    U16,
    U32,
    U64,
    F32,
    F64,
    Coord2D,
    Coord3D,
    Utf16,
    Ascii,
    Angle,
}

impl ArrayElem {
    /// Size in bytes of one packed element, or `None` for variable-size
    /// string elements.
    pub fn fixed_size(self) -> Option<usize> {
        Some(match self {
            ArrayElem::Bool | ArrayElem::I8 | ArrayElem::U8 => 1,
            ArrayElem::I16 | ArrayElem::U16 | ArrayElem::Angle => 2,
            ArrayElem::I32 | ArrayElem::U32 | ArrayElem::F32 => 4,
            ArrayElem::I64 | ArrayElem::U64 | ArrayElem::F64 | ArrayElem::Coord2D => 8,
            ArrayElem::Coord3D => 12,
            ArrayElem::Utf16 | ArrayElem::Ascii => return None,
        })
    }

    /// Display name for the UI, e.g. "UInt32 Array".
    pub fn display_name(self) -> &'static str {
        match self {
            ArrayElem::Bool => "Boolean Array",
            ArrayElem::I8 => "Int8 Array",
            ArrayElem::I16 => "Int16 Array",
            ArrayElem::I32 => "Int32 Array",
            ArrayElem::I64 => "Int64 Array",
            ArrayElem::U8 => "UInt8 Array",
            ArrayElem::U16 => "UInt16 Array",
            ArrayElem::U32 => "UInt32 Array",
            ArrayElem::U64 => "UInt64 Array",
            ArrayElem::F32 => "Float32 Array",
            ArrayElem::F64 => "Float64 Array",
            ArrayElem::Coord2D => "Point2D Array",
            ArrayElem::Coord3D => "Point3D Array",
            ArrayElem::Utf16 => "UTF-16 Array",
            ArrayElem::Ascii => "ASCII Array",
            ArrayElem::Angle => "Angle Array",
        }
    }

    /// Map an array tag byte (0x41-0x50) to its element type.
    pub fn from_array_tag(tag: u8) -> Option<ArrayElem> {
        Some(match tag {
            0x41 => ArrayElem::Bool,
            0x42 => ArrayElem::I8,
            0x43 => ArrayElem::I16,
            0x44 => ArrayElem::I32,
            0x45 => ArrayElem::I64,
            0x46 => ArrayElem::U8,
            0x47 => ArrayElem::U16,
            0x48 => ArrayElem::U32,
            0x49 => ArrayElem::U64,
            0x4a => ArrayElem::F32,
            0x4b => ArrayElem::F64,
            0x4c => ArrayElem::Coord2D,
            0x4d => ArrayElem::Coord3D,
            0x4e => ArrayElem::Utf16,
            0x4f => ArrayElem::Ascii,
            0x50 => ArrayElem::Angle,
            _ => return None,
        })
    }
}

/// Type tags for leaf values and structural nodes.
///
/// Payload layouts (ABCD/ABCE; all integers little-endian):
/// - Primitives (0x00-0x10): fixed-size payload immediately after the tag.
/// - Strings 0x0E/0x0F: u16 count, then `count` UTF-16 code units / bytes.
/// - Arrays 0x41-0x50: u32 absolute end-offset, then packed elements.
/// - Records 0x80/0x81: u16 name index, u8 version, u32 absolute end-offset
///   (0x81 additionally has a u32 record count, then per-record u32
///   end-offsets delimiting each entry).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EsfValueType {
    /// NOT in the ESF spec (taw/RPFM define nothing at 0x00). The 2009 C#
    /// EsfEditor mapped 0x00 to a 2-byte "Short" and we keep that behavior
    /// for compatibility, but no occurrence has been observed in real
    /// Empire saves. The real int16 tag is 0x03.
    LegacyShort00 = 0x00,
    /// 1 byte: 0 = false, 1 = true.
    Bool = 0x01,
    /// 1-byte signed integer.
    I8 = 0x02,
    /// 2-byte signed integer.
    I16 = 0x03,
    /// 4-byte signed integer.
    I32 = 0x04,
    /// 8-byte signed integer.
    I64 = 0x05,
    /// 1-byte unsigned integer.
    U8 = 0x06,
    /// 2-byte unsigned integer.
    U16 = 0x07,
    /// 4-byte unsigned integer.
    U32 = 0x08,
    /// 8-byte unsigned integer.
    U64 = 0x09,
    /// IEEE 754 single-precision float.
    F32 = 0x0a,
    /// IEEE 754 double-precision float.
    F64 = 0x0b,
    /// Two f32: (x, y) map coordinates.
    Coord2D = 0x0c,
    /// Three f32: (x, y, z) coordinates.
    Coord3D = 0x0d,
    /// UTF-16LE string: u16 code-unit count + data (inline in ABCD/ABCE;
    /// u32 string-table index in ABCF/ABCA).
    Utf16 = 0x0e,
    /// ASCII string: u16 byte length + data (inline in ABCD/ABCE;
    /// u32 string-table index in ABCF/ABCA).
    Ascii = 0x0f,
    /// u16 angle in degrees (0-360). The C# EsfEditor called this "UShort".
    Angle = 0x10,
    /// Typed arrays: tag = 0x40 + element tag. u32 end-offset + packed
    /// elements. The C# EsfEditor treated these as opaque "Binary41-4D"
    /// blobs and did not know 0x4E-0x50.
    BoolArray = 0x41,
    I8Array = 0x42,
    I16Array = 0x43,
    I32Array = 0x44,
    I64Array = 0x45,
    U8Array = 0x46,
    U16Array = 0x47,
    U32Array = 0x48,
    U64Array = 0x49,
    F32Array = 0x4a,
    F64Array = 0x4b,
    Coord2DArray = 0x4c,
    Coord3DArray = 0x4d,
    Utf16Array = 0x4e,
    AsciiArray = 0x4f,
    AngleArray = 0x50,
    /// NOT in the ESF spec. The C# EsfEditor parsed 0x6D (109) as an opaque
    /// 4-byte value via an unnamed enum cast; kept for compatibility. No
    /// occurrence observed in real Empire saves so far.
    Unknown6D = 0x6d,
    /// Single record node: u16 name index, u8 version, u32 end-offset.
    SingleNode = 0x80,
    /// Record-array node: like 0x80 plus u32 count, then each record
    /// prefixed by its own u32 end-offset.
    PolyNode = 0x81,
    /// NOT in the ESF spec for ABCD/ABCE. The C# EsfEditor parsed 0x8C (140)
    /// as an "optimized block": u32 end-offset + opaque bytes. (In ABCA,
    /// 0x80-0x9F is the compact-record tag range instead.) Kept for
    /// compatibility; no occurrence observed in real Empire saves so far.
    SizedBlock8C = 0x8c,
}

impl EsfValueType {
    /// Element type when this tag is a typed array (0x41-0x50).
    pub fn array_elem(self) -> Option<ArrayElem> {
        ArrayElem::from_array_tag(self as u8)
    }
}

impl TryFrom<u8> for EsfValueType {
    type Error = String;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        Ok(match value {
            0x00 => EsfValueType::LegacyShort00,
            0x01 => EsfValueType::Bool,
            0x02 => EsfValueType::I8,
            0x03 => EsfValueType::I16,
            0x04 => EsfValueType::I32,
            0x05 => EsfValueType::I64,
            0x06 => EsfValueType::U8,
            0x07 => EsfValueType::U16,
            0x08 => EsfValueType::U32,
            0x09 => EsfValueType::U64,
            0x0a => EsfValueType::F32,
            0x0b => EsfValueType::F64,
            0x0c => EsfValueType::Coord2D,
            0x0d => EsfValueType::Coord3D,
            0x0e => EsfValueType::Utf16,
            0x0f => EsfValueType::Ascii,
            0x10 => EsfValueType::Angle,
            0x41 => EsfValueType::BoolArray,
            0x42 => EsfValueType::I8Array,
            0x43 => EsfValueType::I16Array,
            0x44 => EsfValueType::I32Array,
            0x45 => EsfValueType::I64Array,
            0x46 => EsfValueType::U8Array,
            0x47 => EsfValueType::U16Array,
            0x48 => EsfValueType::U32Array,
            0x49 => EsfValueType::U64Array,
            0x4a => EsfValueType::F32Array,
            0x4b => EsfValueType::F64Array,
            0x4c => EsfValueType::Coord2DArray,
            0x4d => EsfValueType::Coord3DArray,
            0x4e => EsfValueType::Utf16Array,
            0x4f => EsfValueType::AsciiArray,
            0x50 => EsfValueType::AngleArray,
            0x6d => EsfValueType::Unknown6D,
            0x80 => EsfValueType::SingleNode,
            0x81 => EsfValueType::PolyNode,
            0x8c => EsfValueType::SizedBlock8C,
            _ => return Err(format!("Unknown EsfValueType byte: 0x{:02X}", value)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_esf_type_try_from() {
        assert_eq!(EsfType::try_from(0xABCD), Ok(EsfType::ABCD));
        assert_eq!(EsfType::try_from(0xABCE), Ok(EsfType::ABCE));
        assert_eq!(EsfType::try_from(0xABCF), Ok(EsfType::ABCF));
        assert_eq!(EsfType::try_from(0xABCA), Ok(EsfType::ABCA));
        assert!(EsfType::try_from(0x1234).is_err());
    }

    #[test]
    fn test_esf_value_type_try_from() {
        assert_eq!(EsfValueType::try_from(0x0f), Ok(EsfValueType::Ascii));
        assert_eq!(EsfValueType::try_from(1), Ok(EsfValueType::Bool));
        assert_eq!(EsfValueType::try_from(0x10), Ok(EsfValueType::Angle));
        assert_eq!(EsfValueType::try_from(0x48), Ok(EsfValueType::U32Array));
        assert_eq!(EsfValueType::try_from(0x50), Ok(EsfValueType::AngleArray));
        assert!(EsfValueType::try_from(0xFF).is_err());
        assert!(EsfValueType::try_from(0x11).is_err());
        assert!(EsfValueType::try_from(0x51).is_err());
    }

    #[test]
    fn array_tags_map_to_elements() {
        assert_eq!(EsfValueType::U32Array.array_elem(), Some(ArrayElem::U32));
        assert_eq!(EsfValueType::BoolArray.array_elem(), Some(ArrayElem::Bool));
        assert_eq!(EsfValueType::AngleArray.array_elem(), Some(ArrayElem::Angle));
        assert_eq!(EsfValueType::U32.array_elem(), None);
        assert_eq!(ArrayElem::Coord3D.fixed_size(), Some(12));
        assert_eq!(ArrayElem::Utf16.fixed_size(), None);
    }
}
