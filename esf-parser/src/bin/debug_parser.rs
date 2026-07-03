use esf_parser::objects::{EsfDocument, NodeId};

fn print_tree(doc: &EsfDocument, id: NodeId, depth: usize, max_depth: usize) {
    if depth > max_depth {
        return;
    }
    let node = doc.node(id);
    let values = doc.node_values(id).count();
    let children: Vec<NodeId> = doc.children(id).collect();
    println!(
        "{}{} [{:?} v{}] @0x{:x} ({} values, {} children)",
        "  ".repeat(depth),
        doc.node_name(id),
        node.kind,
        node.version,
        node.offset,
        values,
        children.len(),
    );
    for child in children {
        print_tree(doc, child, depth + 1, max_depth);
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).unwrap_or_else(|| {
        r"C:\Projects\Rust\_old\esfeditor\saves\test_save.empire_save_multiplayer".to_string()
    });
    let depth: usize = std::env::args()
        .nth(2)
        .and_then(|d| d.parse().ok())
        .unwrap_or(2);

    let start = std::time::Instant::now();
    let doc = esf_parser::parser::load_file(&path)?;
    let elapsed = start.elapsed();

    println!(
        "parsed {} in {:?}: {} nodes, {} values, {} names, magic {:?}",
        path,
        elapsed,
        doc.nodes.len(),
        doc.values.len(),
        doc.node_names.len(),
        doc.header.magic,
    );
    print_tree(&doc, doc.root, 0, depth);
    Ok(())
}
