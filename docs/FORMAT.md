# The ESF format (ABCD / ABCE)

ESF ("Empire Save Format") is Creative Assembly's binary object-serialization
format, used for campaign saves (`.empire_save`, `.empire_save_multiplayer`),
`startpos.esf`, `trade_routes.esf`, `pathfinding.esf`, and more. Structurally
it is a binary XML: a tree of named, versioned **record nodes** whose leaves
are typed **values**.

This document specifies the two variants twedit supports — **ABCD** and
**ABCE** (Empire and Napoleon) — and notes where later variants differ.
Everything here is cross-confirmed by:

- taw's specification: <https://t-a-w.blogspot.com/2012/03/esf-empire-total-war-object.html>
  and the etwng tools (<https://github.com/taw/etwng>)
- RPFM's implementation (`rpfm_lib/src/files/esf/`, on docs.rs)
- The original C# EsfEditor 1.4.4 (2009), where it agreed with the above
- Empirical scans of a real 104 MB Empire multiplayer campaign save
  ([scan_report.md](scan_report.md), regenerate with the `schema_scan` tool)

All integers are little-endian. All offsets are absolute u32 file positions.

## File layout

```
┌────────────────────────────┐
│ header                     │
│ root node (0x80)           │  the entire tree
│ node-name table ("footer") │  pointed to by the header
└────────────────────────────┘
```

### Header

| Variant | Layout |
|---|---|
| ABCD | u32 magic `0x0000ABCD` · u32 name-table offset |
| ABCE | u32 magic `0x0000ABCE` · u32 (always 0) · u32 Unix timestamp · u32 name-table offset |

The root node begins immediately after the header and must be a record node
(tag 0x80 or 0x81).

Later variants: **ABCF** (Shogun 2) moves strings into footer string tables;
**ABCA** (Rome 2+) additionally uses variable-length sizes, compact record
tags with inline version bits, and optimized primitive tags 0x12–0x1D.
twedit recognizes both and rejects them with a clear error.

### Node-name table

At the header's name-table offset:

```
u16 count
count × { u16 length, ascii[length] }   -- node names, indexed by records
```

ABCF/ABCA append UTF-16 and ASCII string tables after the names; ABCD/ABCE
store all strings inline, so the name table is the whole footer.

## Records (structural nodes)

**Single record — tag 0x80**

```
0x80 · u16 name_index · u8 version · u32 end_offset · [children...]
```

`end_offset` is the absolute position one past the node's content. Children
are any sequence of values and nested records, in file order.

**Record array — tag 0x81** ("poly node" in twedit)

```
0x81 · u16 name_index · u8 version · u32 end_offset · u32 count ·
count × { u32 record_end_offset · [children...] }
```

Each entry ("record" in twedit's arena) is delimited by its own end offset
and inherits the array's name and version. The count is redundant with the
offsets but both must be consistent.

## Value tags

### Primitives (fixed size)

| Tag | Name | Payload |
|---|---|---|
| 0x01 | Bool | 1 byte: 0 / 1 |
| 0x02 | I8 | 1-byte signed |
| 0x03 | I16 | 2-byte signed |
| 0x04 | I32 | 4-byte signed |
| 0x05 | I64 | 8-byte signed |
| 0x06 | U8 | 1-byte unsigned |
| 0x07 | U16 | 2-byte unsigned |
| 0x08 | U32 | 4-byte unsigned |
| 0x09 | U64 | 8-byte unsigned |
| 0x0A | F32 | IEEE 754 single |
| 0x0B | F64 | IEEE 754 double |
| 0x0C | Coord2D | 2 × f32 (x, y) |
| 0x0D | Coord3D | 3 × f32 (x, y, z) |
| 0x10 | Angle | u16 degrees (0–360) |

### Strings (variable size, inline in ABCD/ABCE)

| Tag | Name | Payload |
|---|---|---|
| 0x0E | Utf16 | u16 code-unit count · UTF-16LE data |
| 0x0F | Ascii | u16 byte length · ASCII data |

In ABCF/ABCA these instead hold a u32 index into the footer string tables.

### Typed arrays — tags 0x41–0x50

Array tag = **0x40 + element tag**. Layout:

```
tag · u32 end_offset · packed elements (no per-element tags)
```

So 0x41 = bool[], 0x44 = i32[], 0x48 = u32[], 0x4A = f32[], 0x4C = coord2d[],
0x4E = utf16[] (each element keeps its u16 count prefix), 0x4F = ascii[],
0x50 = angle[]. Empire's campaign saves are array-heavy: the scanned save
holds 1.38 M u32 arrays.

### Empire value-type census

From the scanned save (10,002,564 values):

| Type | Count |
|---|---|
| U32 | 4,156,030 |
| Bool | 2,003,863 |
| F32 | 1,521,138 |
| U32 array | 1,382,725 |
| I32 | 620,214 |
| Utf16 | 231,247 |
| Coord2D | 49,457 |
| Ascii | 10,151 |
| U8 | 7,224 |
| I32 array | 7,207 |
| F32 array | 7,198 |
| U16 | 3,288 |
| Angle | 1,956 |
| U16 array | 560 |
| Bool array | 246 |
| U8 array | 57 |
| U64 | 2 |
| Coord3D | 1 |

### Legacy compatibility tags (not in the spec)

The 2009 C# EsfEditor handled three tags that appear in **no** community
specification, and **zero** of them occur in the scanned save. twedit keeps
parsing them for compatibility, under honest names:

| Tag | twedit name | C# behavior |
|---|---|---|
| 0x00 | LegacyShort00 | read as 2-byte "Short" |
| 0x6D | Unknown6D | read as opaque 4 bytes |
| 0x8C | SizedBlock8C | u32 end offset + opaque bytes |

If `schema_scan` ever reports a non-zero count for these, that file is the
evidence needed to identify what they really are.

## Editing and offset fixups

Fixed-size values can be patched in place: the payload starts one byte after
the tag, and nothing else moves.

Anything that changes a value's byte length (string edits) shifts the rest
of the file, so every stored absolute offset after the splice must be
adjusted by the length delta:

- the header's name-table offset (position 4 in ABCD, 12 in ABCE),
- every record's `end_offset` (at node offset + 4 for 0x80/0x81 headers;
  a record-array entry's own start IS its end-offset field),
- every array/sized-block `end_offset` (at value offset + 1).

A stored end-offset shifts iff the splice lies strictly inside the region it
closes; a field's own position shifts iff the splice lies before it. This is
exactly what `EsfDocument::bytes_with_edits` implements; it is verified by a
grow-then-shrink round trip on a real 104 MB save reproducing the input
byte-for-byte.

## Worked example: diplomacy

Wars and treaties live per faction, symmetrically — each side holds a
mirrored record and both must agree:

```
FACTION/DIPLOMACY_MANAGER/DIPLOMACY_RELATIONSHIPS_ARRAY/
  DIPLOMACY_RELATIONSHIP        (one per other faction)
    values: target faction id (I32), trade agreement (Bool),
            military access (I32, -1 = indefinite),
            state (Utf16: war/neutral/allied/patron/protectorate),
            ... payment/standing counters, turns at war ...
    DIPLOMACY_RELATIONSHIP_ATTITUDES_ARRAY
      22 fixed slots (gift received, alliance broken, ..., territorial
      expansion, religion, ...) of 6 values each:
      drift, current, limit, active, secondary value, secondary active
```

Field-by-field labels — including the full 22-slot attitude table — are in
[esf_schema.toml](../twedit-ui/assets/esf_schema.toml), which drives the
Description column and node doc line in the editor.

## Extending the documentation

1. Run `cargo run --release -p esf-parser --bin schema_scan -- <save> out.md`.
2. Find an undocumented node; the per-field ranges, samples, and observed
   strings usually suggest meanings (IDs look like faction/region ids,
   turn counters max out at the current turn, keys match db tables).
3. Cross-reference etwng's `esfxml/lib/esf_semantic_converter.rb` — it has
   annotations for ~150 node types, addressed as nth-occurrence-of-type,
   which maps directly onto `typed` keys in esf_schema.toml.
4. Add the labels to `twedit-ui/assets/esf_schema.toml` and a test if the
   node matters (see `real_assets_parse_and_resolve_diplomacy`).
