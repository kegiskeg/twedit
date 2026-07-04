//! Schema inference / research tool: walks a parsed ESF file and reports,
//! per node name, how many instances exist, what value-type signatures they
//! carry, and per-field observed values. The output is the raw material for
//! labeling fields in twedit's schema (assets/esf_schema.toml) and for
//! verifying documentation claims empirically.
//!
//! Usage: schema_scan <save> [output.md]

use esf_parser::objects::{EsfDocument, EsfValue, NodeKind};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;

/// Cap on distinct sample values kept per field position.
const MAX_SAMPLES: usize = 8;
/// Cap on field positions detailed per node name.
const MAX_FIELDS: usize = 40;
/// Cap on distinct type signatures listed per node name.
const MAX_SIGNATURES: usize = 4;
/// Sample strings longer than this are truncated in the report.
const MAX_SAMPLE_LEN: usize = 48;

#[derive(Default)]
struct FieldStats {
    types: BTreeSet<&'static str>,
    /// Distinct formatted samples; one extra slot marks overflow.
    samples: BTreeSet<String>,
    overflow: bool,
    min: Option<f64>,
    max: Option<f64>,
}

impl FieldStats {
    fn record(&mut self, doc: &EsfDocument, value: &EsfValue) {
        self.types.insert(EsfDocument::value_type_name(value));
        if let Some(n) = numeric(value) {
            self.min = Some(self.min.map_or(n, |m| m.min(n)));
            self.max = Some(self.max.map_or(n, |m| m.max(n)));
        }
        if self.samples.len() < MAX_SAMPLES {
            let mut s = doc.format_value(value);
            if s.chars().count() > MAX_SAMPLE_LEN {
                s = s.chars().take(MAX_SAMPLE_LEN).collect::<String>() + "…";
            }
            self.samples.insert(s);
        } else {
            self.overflow = true;
        }
    }
}

fn numeric(value: &EsfValue) -> Option<f64> {
    Some(match value {
        EsfValue::I8(v) => *v as f64,
        EsfValue::I16(v) | EsfValue::LegacyShort(v) => *v as f64,
        EsfValue::I32(v) => *v as f64,
        EsfValue::I64(v) => *v as f64,
        EsfValue::U8(v) => *v as f64,
        EsfValue::U16(v) | EsfValue::Angle(v) => *v as f64,
        EsfValue::U32(v) => *v as f64,
        EsfValue::U64(v) => *v as f64,
        EsfValue::F32(v) => *v as f64,
        EsfValue::F64(v) => *v,
        _ => return None,
    })
}

#[derive(Default)]
struct NodeStats {
    count: u64,
    /// Distinct value-type signatures with occurrence counts.
    signatures: HashMap<Vec<&'static str>, u64>,
    fields: Vec<FieldStats>,
    child_names: BTreeSet<String>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: schema_scan <save> [output.md]");
    let out_path = args.next();

    let started = std::time::Instant::now();
    let doc = esf_parser::parser::load_file(&path)?;
    eprintln!(
        "parsed {} nodes / {} values in {:.2?}",
        doc.nodes.len(),
        doc.values.len(),
        started.elapsed()
    );

    // Global type histogram + legacy tag census.
    let mut type_histogram: BTreeMap<&'static str, u64> = BTreeMap::new();
    for rec in &doc.values {
        *type_histogram
            .entry(EsfDocument::value_type_name(&rec.value))
            .or_default() += 1;
    }

    // Per node-name stats. Records are grouped under "NAME[]".
    let mut stats: BTreeMap<String, NodeStats> = BTreeMap::new();
    for (idx, node) in doc.nodes.iter().enumerate() {
        let id = idx as u32;
        let key = match node.kind {
            NodeKind::Record => format!("{}[]", doc.node_name(id)),
            _ => doc.node_name(id).to_string(),
        };
        let entry = stats.entry(key).or_default();
        entry.count += 1;

        let signature: Vec<&'static str> = doc
            .node_values(id)
            .map(|r| EsfDocument::value_type_name(&r.value))
            .collect();
        for (pos, rec) in doc.node_values(id).enumerate() {
            if pos >= MAX_FIELDS {
                break;
            }
            if entry.fields.len() <= pos {
                entry.fields.push(FieldStats::default());
            }
            entry.fields[pos].record(&doc, &rec.value);
        }
        if entry.signatures.len() < 64 {
            *entry.signatures.entry(signature).or_default() += 1;
        }
        for child in doc.children(id) {
            let name = match doc.node(child).kind {
                NodeKind::Record => continue, // records share the poly name
                _ => doc.node_name(child).to_string(),
            };
            entry.child_names.insert(name);
        }
    }

    // ---- Report ----
    let mut md = String::new();
    let _ = writeln!(md, "# ESF schema scan\n");
    let _ = writeln!(md, "- File: `{path}`");
    let _ = writeln!(
        md,
        "- Magic: {:?}; {} nodes, {} values, {} node names\n",
        doc.header.magic,
        doc.nodes.len(),
        doc.values.len(),
        doc.node_names.len()
    );

    let _ = writeln!(md, "## Global value-type histogram\n");
    let _ = writeln!(md, "| Type | Count |");
    let _ = writeln!(md, "|---|---|");
    let mut hist: Vec<_> = type_histogram.iter().collect();
    hist.sort_by(|a, b| b.1.cmp(a.1));
    for (name, count) in &hist {
        let _ = writeln!(md, "| {name} | {count} |");
    }

    let legacy: u64 = ["Int16 (legacy 0x00)", "Unknown 0x6D", "Sized Block 0x8C"]
        .iter()
        .map(|n| type_histogram.get(*n).copied().unwrap_or(0))
        .sum();
    let _ = writeln!(
        md,
        "\nLegacy C#-compat tags (0x00 / 0x6D / 0x8C) observed: **{legacy}**{}",
        if legacy == 0 {
            " — consistent with them not being real ESF types."
        } else {
            " — they DO occur; investigate."
        }
    );

    let _ = writeln!(md, "\n## Node types ({})\n", stats.len());
    let _ = writeln!(md, "Sorted by instance count. `NAME[]` = records inside a poly node.\n");

    let mut names: Vec<_> = stats.keys().cloned().collect();
    names.sort_by_key(|n| std::cmp::Reverse(stats[n].count));

    for name in &names {
        let s = &stats[name];
        let _ = writeln!(md, "### {name}\n");
        let _ = writeln!(md, "- Instances: {}", s.count);
        if !s.child_names.is_empty() {
            let children: Vec<_> = s.child_names.iter().cloned().collect();
            let _ = writeln!(md, "- Child nodes: {}", children.join(", "));
        }
        if s.signatures.len() > 1 {
            let mut sigs: Vec<_> = s.signatures.iter().collect();
            sigs.sort_by_key(|(_, c)| std::cmp::Reverse(**c));
            let _ = writeln!(md, "- {} distinct signatures; most common:", s.signatures.len());
            for (sig, count) in sigs.iter().take(MAX_SIGNATURES) {
                let _ = writeln!(md, "  - ({count}×) {}", sig.join(", "));
            }
        }
        if !s.fields.is_empty() {
            let _ = writeln!(md, "\n| # | Type | Range | Samples |");
            let _ = writeln!(md, "|---|---|---|---|");
            for (pos, f) in s.fields.iter().enumerate() {
                let types: Vec<_> = f.types.iter().copied().collect();
                let range = match (f.min, f.max) {
                    (Some(a), Some(b)) if a != b => format!("{a} … {b}"),
                    (Some(a), _) => format!("{a}"),
                    _ => String::new(),
                };
                let mut samples: Vec<_> = f.samples.iter().cloned().collect();
                if f.overflow {
                    samples.push("…".to_string());
                }
                let _ = writeln!(
                    md,
                    "| {pos} | {} | {} | {} |",
                    types.join(" / "),
                    range,
                    samples.join("; ").replace('|', "\\|")
                );
            }
        }
        let _ = writeln!(md);
    }

    match out_path {
        Some(p) => {
            std::fs::write(&p, &md)?;
            eprintln!("report written to {p} ({} KB)", md.len() / 1024);
        }
        None => print!("{md}"),
    }

    // Console summary.
    eprintln!("node types: {}", stats.len());
    eprintln!("legacy-tag occurrences: {legacy}");
    Ok(())
}
