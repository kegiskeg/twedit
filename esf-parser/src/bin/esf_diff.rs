use esf_parser::objects::{EsfDocument, EsfItem, EsfValue, NodeId};
use std::env;
use std::process;

/// Checks if two values are semantically equal, ignoring their absolute file offsets.
fn values_equal(doc1: &EsfDocument, v1: &EsfValue, doc2: &EsfDocument, v2: &EsfValue) -> bool {
    if !v1.same_variant(v2) {
        return false;
    }
    match (v1, v2) {
        (EsfValue::Utf16 { .. }, EsfValue::Utf16 { .. })
        | (EsfValue::Ascii { .. }, EsfValue::Ascii { .. }) => {
            doc1.decode_string(v1) == doc2.decode_string(v2)
        }
        (EsfValue::Array { .. }, EsfValue::Array { .. })
        | (EsfValue::SizedBlock { .. }, EsfValue::SizedBlock { .. }) => {
            doc1.raw_bytes(v1) == doc2.raw_bytes(v2)
        }
        _ => v1 == v2, // All other variants are absolute-offset free and implement PartialEq correctly
    }
}

/// Recursively compare the trees starting from node1 and node2
fn compare_nodes(
    doc1: &EsfDocument,
    id1: NodeId,
    doc2: &EsfDocument,
    id2: NodeId,
    diffs: &mut usize,
) {
    let node1 = doc1.node(id1);
    let node2 = doc2.node(id2);

    if node1.kind != node2.kind || doc1.node_name(id1) != doc2.node_name(id2) {
        println!(
            "[{}] <Structural difference> Node name/kind mismatch: {:?} vs {:?}",
            doc1.node_path(id1),
            doc1.node_name(id1),
            doc2.node_name(id2)
        );
        *diffs += 1;
        return;
    }

    let items1 = doc1.node_items(id1);
    let items2 = doc2.node_items(id2);

    if items1.len() != items2.len() {
        println!(
            "[{}] <Structural difference> Child count mismatch: {} items vs {} items",
            doc1.node_path(id1),
            items1.len(),
            items2.len()
        );
        *diffs += 1;
        return;
    }

    let mut value_index = 0; // To label which value we are looking at in the array of values

    for (item1, item2) in items1.iter().zip(items2.iter()) {
        match (item1, item2) {
            (EsfItem::Node(child_id1), EsfItem::Node(child_id2)) => {
                compare_nodes(doc1, *child_id1, doc2, *child_id2, diffs);
            }
            (EsfItem::Value(val_id1), EsfItem::Value(val_id2)) => {
                let v1 = &doc1.values[*val_id1 as usize].value;
                let v2 = &doc2.values[*val_id2 as usize].value;

                if !values_equal(doc1, v1, doc2, v2) {
                    println!(
                        "[{}] Value {} changed:",
                        doc1.node_path(id1),
                        value_index
                    );
                    let f1 = doc1.format_value(v1);
                    let f2 = doc2.format_value(v2);
                    println!("  - Before: {}", f1);
                    println!("  + After:  {}", f2);
                    *diffs += 1;
                }
                value_index += 1;
            }
            _ => {
                println!(
                    "[{}] <Structural difference> Item type mismatch (Node vs Value) at position",
                    doc1.node_path(id1)
                );
                *diffs += 1;
                return; // Structural mismatch, abort deeper inspection
            }
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: esf_diff <save_before> <save_after>");
        process::exit(1);
    }

    let path1 = &args[1];
    let path2 = &args[2];

    println!("Loading {}...", path1);
    let doc1 = esf_parser::parser::load_file(path1)?;
    println!("Loading {}...", path2);
    let doc2 = esf_parser::parser::load_file(path2)?;

    println!("\nComparing trees...\n");
    let mut diffs = 0;
    
    // Compare roots
    compare_nodes(&doc1, doc1.root, &doc2, doc2.root, &mut diffs);

    if diffs == 0 {
        println!("No differences found!");
    } else {
        println!("\nFound {} differences.", diffs);
    }

    Ok(())
}
