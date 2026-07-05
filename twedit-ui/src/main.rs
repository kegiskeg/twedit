#![windows_subsystem = "windows"]

mod campaign;
mod descriptions;
mod theme;

use campaign::{extract_factions, extract_regions, FactionRow, RegionRow};
use descriptions::Descriptions;
use esf_parser::objects::{EsfDocument, EsfEdit, EsfValue, NodeId, NodeKind, NO_PARENT};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use windows_core::Result;
use windows_reactor::*;
use tracing::{info, error};
use tracing_subscriber::fmt::writer::MakeWriterExt;

const DEFAULT_SAVE_PATH: &str =
    r"C:\Projects\Rust\_old\esfeditor\saves\test_save.empire_save_multiplayer";

/// Cap on materialized children per tree node; huge poly nodes (region lists,
/// unit rosters) would otherwise stall the XAML tree view.
const MAX_TREE_CHILDREN: usize = 1000;
/// Cap on value rows in the grid; each row hosts a live XAML text box.
const MAX_VALUE_ROWS: usize = 200;
const SEARCH_RESULT_LIMIT: usize = 200;
/// Minimum query length before search-as-you-type kicks in.
const LIVE_SEARCH_MIN_CHARS: usize = 2;
/// Cap on rows in the pending-edits drawer.
const MAX_EDIT_ROWS: usize = 100;
/// Array elements shown in a value tooltip before truncating.
const ARRAY_TOOLTIP_LIMIT: usize = 64;

/// Monotonic ticket for search requests; a finished search only publishes
/// its results if no newer search has started (search-as-you-type races).
static SEARCH_GEN: AtomicU64 = AtomicU64::new(0);

/// Which main content the body shows. Explorer is the raw tree; Factions
/// and Regions are semantic campaign views built on `campaign`.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum MainView {
    #[default]
    Explorer,
    Factions,
    Regions,
}

/// Arc-backed document handle with pointer-identity equality, so state
/// comparisons never deep-compare a 100MB document.
#[derive(Clone, Default)]
struct DocState {
    doc: Option<Arc<EsfDocument>>,
    /// Path the document was loaded from / last saved to.
    path: String,
}

impl PartialEq for DocState {
    fn eq(&self, other: &Self) -> bool {
        let same_doc = match (&self.doc, &other.doc) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        same_doc && self.path == other.path
    }
}

#[derive(Clone, Default)]
struct DescState(Option<Arc<Descriptions>>);

impl PartialEq for DescState {
    fn eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        }
    }
}

/// Edits staged in the UI, keyed by global value id.
type Edits = HashMap<u32, EsfEdit>;

// The tree widget only reports the clicked label text, so the arena node id
// must travel inside the label. To keep labels visually clean (like the old
// editor), the id is appended as invisible characters: a U+2063 marker
// followed by 32 bits encoded as U+200B (0) / U+200C (1).
const ID_MARK: char = '\u{2063}';
const ID_ZERO: char = '\u{200B}';
const ID_ONE: char = '\u{200C}';

fn encode_node_id(id: NodeId) -> String {
    let mut out = String::with_capacity(33);
    out.push(ID_MARK);
    for bit in (0..32).rev() {
        out.push(if (id >> bit) & 1 == 1 { ID_ONE } else { ID_ZERO });
    }
    out
}

/// Extract the invisible arena node id from a tree label.
fn label_node_id(label: &str) -> Option<NodeId> {
    let start = label.rfind(ID_MARK)?;
    let mut id: NodeId = 0;
    let mut bits = 0;
    for c in label[start..].chars().skip(1) {
        match c {
            ID_ZERO => id <<= 1,
            ID_ONE => id = (id << 1) | 1,
            _ => break,
        }
        bits += 1;
        if bits == 32 {
            return Some(id);
        }
    }
    None
}

fn node_label(doc: &EsfDocument, id: NodeId, record_index: usize) -> String {
    let node = doc.node(id);
    let visible = match node.kind {
        NodeKind::Record => format!("{} ({})", doc.node_name(id), record_index),
        NodeKind::Poly => format!("{} ({})", doc.node_name(id), doc.child_count(id)),
        NodeKind::Single => doc.node_name(id).to_string(),
    };
    format!("{visible}{}", encode_node_id(id))
}

/// Materialize the visible portion of the arena into TreeNodeDefs.
///
/// The tree_view widget is declarative with no on_expanding callback, so we
/// materialize children of every expanded node plus one extra level (so
/// collapsed-but-visible nodes still show an expander chevron). Invoking a
/// node toggles it in the expanded set, which rebuilds the defs one level
/// deeper — click-to-drill-down lazy loading.
fn build_node_def(
    doc: &EsfDocument,
    id: NodeId,
    record_index: usize,
    expanded: &HashSet<NodeId>,
    materialize_children: bool,
) -> TreeNodeDef {
    let mut def = tree_node(node_label(doc, id, record_index));
    let is_expanded = expanded.contains(&id);
    if is_expanded {
        def = def.expanded();
    }
    if materialize_children || is_expanded {
        let total = doc.child_count(id);
        for (index, child) in doc.children(id).take(MAX_TREE_CHILDREN).enumerate() {
            def = def.child(build_node_def(doc, child, index, expanded, is_expanded));
        }
        if total > MAX_TREE_CHILDREN {
            def = def.child(tree_node(format!(
                "… {} more entries not shown",
                total - MAX_TREE_CHILDREN
            )));
        }
    }
    def
}

fn build_tree(doc: &EsfDocument, expanded: &HashSet<NodeId>) -> Vec<TreeNodeDef> {
    vec![build_node_def(doc, doc.root, 0, expanded, true)]
}

/// Expand every ancestor of `id` so it becomes visible in the tree.
fn expand_path_to(doc: &EsfDocument, id: NodeId, expanded: &mut HashSet<NodeId>) {
    let mut current = id;
    loop {
        let parent = doc.node(current).parent;
        if parent == NO_PARENT {
            break;
        }
        expanded.insert(parent);
        current = parent;
    }
}

// Column widths for the value grid (Value | Original | Type | Description).
const COL_VALUE: f64 = 230.0;
const COL_ORIGINAL: f64 = 200.0;
const COL_TYPE: f64 = 110.0;

/// Shared horizontal inset so header and rows line up column-for-column.
const ROW_INSET: f64 = 8.0;
/// Width of the draggable gap between the tree pane and the content pane.
const SPLITTER_W: f64 = 10.0;

/// Wrap content in layered translucent borders that approximate a soft
/// drop shadow (reactor has no composition ThemeShadow support yet).
fn shadowed(content: impl Into<Element>) -> Element {
    let inner = border(content)
        .border_brush(Color { a: 0x40, r: 0, g: 0, b: 0 })
        .border_thickness(Thickness {
            left: 0.0,
            top: 0.0,
            right: 1.0,
            bottom: 2.0,
        })
        .corner_radius(7.0);
    let mid = border(inner)
        .border_brush(Color { a: 0x20, r: 0, g: 0, b: 0 })
        .border_thickness(Thickness {
            left: 1.0,
            top: 0.0,
            right: 1.0,
            bottom: 2.0,
        })
        .corner_radius(8.0);
    border(mid)
        .border_brush(Color { a: 0x0E, r: 0, g: 0, b: 0 })
        .border_thickness(Thickness {
            left: 1.0,
            top: 1.0,
            right: 2.0,
            bottom: 2.0,
        })
        .corner_radius(9.0)
        .into()
}

/// A rounded, stroked, shadowed panel.
fn card(content: impl Into<Element>, background: Color) -> Element {
    shadowed(
        border(content)
            .background(background)
            .border_brush(theme::BORDER)
            .border_thickness(Thickness::uniform(1.0))
            .corner_radius(6.0),
    )
}

/// Thin vertical separator for toolbar groups.
fn toolbar_divider() -> Element {
    Element::from(border(text_block("")))
        .background(theme::BORDER)
        .width(1.0)
        .height(22.0)
        .margin(Thickness::xy(4.0, 0.0))
        .vertical_alignment(VerticalAlignment::Center)
}

/// Format an integer with thousands separators (2164695 -> "2,164,695").
fn fmt_count(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Signed variant of [`fmt_count`] for treasuries that can run negative.
fn fmt_i64(n: i64) -> String {
    if n < 0 {
        format!("-{}", fmt_count(n.unsigned_abs() as usize))
    } else {
        fmt_count(n as usize)
    }
}

// --- Recent files, persisted to %APPDATA%\twedit\recent.txt ---

const MAX_RECENT: usize = 8;

fn recent_file_path() -> Option<std::path::PathBuf> {
    let appdata = std::env::var_os("APPDATA")?;
    Some(std::path::PathBuf::from(appdata).join("twedit").join("recent.txt"))
}

fn load_recent() -> Vec<String> {
    let Some(path) = recent_file_path() else {
        return Vec::new();
    };
    std::fs::read_to_string(path)
        .map(|text| {
            text.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .take(MAX_RECENT)
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

/// Move `path` to the front of the recent list on disk; returns the new list.
fn push_recent(path: &str) -> Vec<String> {
    let mut list = load_recent();
    list.retain(|p| p != path);
    list.insert(0, path.to_string());
    list.truncate(MAX_RECENT);
    if let Some(file) = recent_file_path() {
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(file, list.join("\n"));
    }
    list
}

/// Well-known nodes worth one-click jumps in Total War saves. Each entry
/// just runs a node-name search, so names missing from a particular game's
/// saves simply return no results.
const GO_TO_TARGETS: &[&str] = &[
    "WORLD",
    "FACTION_ARRAY",
    "REGIONS_ARRAY",
    "PLAYERS_ARRAY",
    "CREATED_CHARACTER_ARRAY",
    "PENDING_BATTLE",
    "TRADE_ROUTES",
    "CAMPAIGN_MODEL",
];

fn grid_header() -> Element {
    border(
        hstack((
            text_block("Value").width(COL_VALUE).foreground(ThemeRef::SecondaryText),
            text_block("Original").width(COL_ORIGINAL).foreground(ThemeRef::SecondaryText),
            text_block("Type").width(COL_TYPE).foreground(ThemeRef::SecondaryText),
            text_block("Description").foreground(ThemeRef::SecondaryText),
        ))
        .spacing(8.0),
    )
    .background(theme::HEADER)
    .border_brush(theme::BORDER)
    .border_thickness(Thickness {
        left: 0.0,
        top: 0.0,
        right: 0.0,
        bottom: 1.0,
    })
    .padding(Thickness {
        left: ROW_INSET,
        top: 6.0,
        right: ROW_INSET,
        bottom: 6.0,
    })
    .into()
}

/// Tooltip text for array/block values: the first elements, wrapped in
/// short lines so the tooltip stays readable.
fn array_tooltip(doc: &EsfDocument, value: &EsfValue) -> Option<String> {
    let (elems, total) = doc.array_element_strings(value, ARRAY_TOOLTIP_LIMIT)?;
    let mut out = format!("{} elements", fmt_count(total));
    for chunk in elems.chunks(8) {
        out.push('\n');
        out.push_str(&chunk.join(", "));
    }
    if total > elems.len() {
        out.push_str("\n…");
    }
    Some(out)
}

/// One row of the value grid.
#[allow(clippy::too_many_arguments)]
fn value_row(
    doc: &EsfDocument,
    value_id: u32,
    original: &EsfValue,
    current_text: String,
    description: Option<&str>,
    edits: &Edits,
    set_edits: &AsyncSetState<Edits>,
    edit_mode: bool,
    row_index: usize,
) -> Element {
    let is_edited = edits.contains_key(&value_id);

    let value_cell: Element = if edit_mode && original.is_editable() {
        // For strings, the "matches the original again" check compares
        // text, decoded up front so the closure stays document-free.
        let original_text = doc.decode_string(original);
        let original = original.clone();
        let edits = edits.clone();
        let set_edits = set_edits.clone();
        let mut cell = text_box(current_text).width(COL_VALUE).on_text_changed(
            move |text: String| {
                let mut next = edits.clone();
                // Unparseable (often a typing intermediate like "-"):
                // leave staged edits untouched.
                let Some(edit) = original.parse_edit(&text) else {
                    return;
                };
                let unchanged = match &edit {
                    EsfEdit::Value(parsed) => *parsed == original,
                    EsfEdit::Text(s) => Some(s.as_str()) == original_text.as_deref(),
                };
                if unchanged {
                    // Text matches the original again: unstage.
                    next.remove(&value_id);
                } else {
                    next.insert(value_id, edit);
                }
                set_edits.call(next);
            },
        );
        if is_edited {
            cell = cell.foreground(ThemeRef::AccentText);
        }
        cell.into()
    } else {
        let mut cell = text_block(current_text).width(COL_VALUE);
        if is_edited {
            cell = cell.foreground(ThemeRef::AccentText);
        }
        // Arrays and blocks are read-only; surface their contents on hover.
        if let Some(tip) = array_tooltip(doc, original) {
            cell = cell.tooltip(tip);
        }
        cell.into()
    };

    border(
        hstack((
            value_cell,
            text_block(doc.format_value(original))
                .width(COL_ORIGINAL)
                .foreground(if is_edited {
                    ThemeRef::PrimaryText
                } else {
                    ThemeRef::SecondaryText
                }),
            text_block(EsfDocument::value_type_name(original))
                .width(COL_TYPE)
                .foreground(ThemeRef::SecondaryText),
            text_block(description.unwrap_or_default()).foreground(ThemeRef::SecondaryText),
        ))
        .spacing(8.0),
    )
    .background(if row_index % 2 == 1 {
        theme::ROW_ALT
    } else {
        theme::PANEL
    })
    .border_brush(theme::BORDER)
    .border_thickness(Thickness {
        left: 0.0,
        top: 0.0,
        right: 0.0,
        bottom: 1.0,
    })
    .padding(Thickness {
        left: ROW_INSET,
        top: 3.0,
        right: ROW_INSET,
        bottom: 3.0,
    })
    .into()
}

fn value_rows(
    doc: &EsfDocument,
    id: NodeId,
    descs: &DescState,
    edits: &Edits,
    set_edits: &AsyncSetState<Edits>,
    edit_mode: bool,
) -> Vec<Element> {
    let name = doc.node_name(id);
    let entries: Vec<_> = doc.node_value_entries(id).collect();
    // Type class of every value, for typed ("s0"/"i1") label resolution.
    let classes: Vec<&'static str> = entries
        .iter()
        .map(|(_, record)| descriptions::type_class(&record.value))
        .collect();

    let mut rows = Vec::new();
    for (index, (value_id, record)) in entries.iter().copied().enumerate() {
        if index >= MAX_VALUE_ROWS {
            rows.push(
                text_block(format!("… {} more values not shown", entries.len() - MAX_VALUE_ROWS))
                    .foreground(ThemeRef::SecondaryText)
                    .into(),
            );
            break;
        }
        let current_text = match edits.get(&value_id) {
            Some(EsfEdit::Value(v)) => doc.format_value(v),
            Some(EsfEdit::Text(s)) => s.clone(),
            None => doc.format_value(&record.value),
        };
        let description = descs
            .0
            .as_ref()
            .and_then(|d| d.label(name, &classes, index, &current_text));
        rows.push(value_row(
            doc,
            value_id,
            &record.value,
            current_text,
            description.as_deref(),
            edits,
            set_edits,
            edit_mode,
            index,
        ));
    }
    if rows.is_empty() {
        rows.push(
            text_block("This node has no values.")
                .foreground(ThemeRef::SecondaryText)
                .into(),
        );
    }
    rows
}

// --- Campaign views -------------------------------------------------------

/// Small square colour chip for a faction's primary flag colour.
fn colour_swatch(color: Option<(u8, u8, u8)>) -> Element {
    let fill = match color {
        Some((r, g, b)) => Color { a: 0xFF, r, g, b },
        None => theme::PANEL,
    };
    Element::from(
        border(text_block(""))
            .border_brush(theme::SWATCH_BORDER)
            .border_thickness(Thickness::uniform(1.0))
            .corner_radius(2.0),
    )
    .background(fill)
    .width(14.0)
    .height(14.0)
    .vertical_alignment(VerticalAlignment::Center)
}

fn campaign_header_cell(label: &str, width: Option<f64>) -> Element {
    let cell = text_block(label).foreground(ThemeRef::SecondaryText);
    match width {
        Some(w) => cell.width(w).into(),
        None => cell.into(),
    }
}

fn campaign_row_border(row_index: usize, content: impl Into<Element>) -> Element {
    border(content)
        .background(if row_index % 2 == 1 {
            theme::ROW_ALT
        } else {
            theme::PANEL
        })
        .border_brush(theme::BORDER)
        .border_thickness(Thickness {
            left: 0.0,
            top: 0.0,
            right: 0.0,
            bottom: 1.0,
        })
        .padding(Thickness {
            left: ROW_INSET,
            top: 4.0,
            right: ROW_INSET,
            bottom: 4.0,
        })
        .into()
}

/// Editable numeric cell staging an in-place scalar edit for `value_id`.
/// Falls back to a plain text cell outside edit mode.
fn numeric_cell(
    doc: &EsfDocument,
    value_id: u32,
    original: EsfValue,
    width: f64,
    edits: &Edits,
    set_edits: &AsyncSetState<Edits>,
    edit_mode: bool,
) -> Element {
    let current_text = match edits.get(&value_id) {
        Some(EsfEdit::Value(v)) => doc.format_value(v),
        Some(EsfEdit::Text(s)) => s.clone(),
        None => doc.format_value(&original),
    };
    let is_edited = edits.contains_key(&value_id);
    if edit_mode {
        let edits = edits.clone();
        let set_edits = set_edits.clone();
        let mut cell = text_box(current_text).width(width).on_text_changed(
            move |text: String| {
                let mut next = edits.clone();
                let Some(edit) = original.parse_edit(&text) else {
                    return;
                };
                let unchanged = matches!(&edit, EsfEdit::Value(parsed) if *parsed == original);
                if unchanged {
                    next.remove(&value_id);
                } else {
                    next.insert(value_id, edit);
                }
                set_edits.call(next);
            },
        );
        if is_edited {
            cell = cell.foreground(ThemeRef::AccentText);
        }
        cell.into()
    } else {
        // Pretty-print integers with separators in view mode.
        let display = match &original {
            EsfValue::I32(v) => fmt_i64(*v as i64),
            EsfValue::U32(v) => fmt_count(*v as usize),
            other => doc.format_value(other),
        };
        let mut cell = text_block(display).width(width);
        if is_edited {
            cell = cell.foreground(ThemeRef::AccentText);
        }
        cell.into()
    }
}

// Campaign table column widths.
const FCOL_NAME: f64 = 230.0;
const FCOL_KEY: f64 = 170.0;
const FCOL_STATUS: f64 = 110.0;
const FCOL_MONEY: f64 = 130.0;
const RCOL_KEY: f64 = 190.0;
const RCOL_THEATRE: f64 = 100.0;
const RCOL_OWNER: f64 = 210.0;
const RCOL_POP: f64 = 110.0;

struct CampaignCtx<'a> {
    doc: &'a EsfDocument,
    edits: &'a Edits,
    set_edits: &'a AsyncSetState<Edits>,
    edit_mode: bool,
    filter: &'a str,
    set_filter: &'a AsyncSetState<String>,
    // Locate: reveal a node in the Explorer tree.
    set_view: &'a AsyncSetState<MainView>,
    set_selected: &'a AsyncSetState<Option<NodeId>>,
    expanded: &'a HashSet<NodeId>,
    set_expanded: &'a AsyncSetState<HashSet<NodeId>>,
}

/// "Locate in tree" button shared by campaign rows.
fn locate_button(ctx: &CampaignCtx, node: NodeId) -> Element {
    let doc_nodes = ctx.doc.nodes.len();
    let set_view = ctx.set_view.clone();
    let set_selected = ctx.set_selected.clone();
    let set_expanded = ctx.set_expanded.clone();
    let expanded = ctx.expanded.clone();
    // Ancestor chain is precomputed so the click closure never touches the
    // document.
    let mut chain = Vec::new();
    if (node as usize) < doc_nodes {
        let mut current = node;
        loop {
            let parent = ctx.doc.node(current).parent;
            if parent == NO_PARENT {
                break;
            }
            chain.push(parent);
            current = parent;
        }
    }
    button("Locate")
        .icon(Symbol::Go)
        .on_click(move || {
            let mut next = expanded.clone();
            next.extend(chain.iter().copied());
            set_expanded.call(next);
            set_selected.call(Some(node));
            set_view.call(MainView::Explorer);
        })
        .tooltip("Show this record in the Explorer tree")
        .into()
}

/// Shared header strip for campaign views: title, count, filter box.
fn campaign_toolbar(title: String, ctx: &CampaignCtx) -> Element {
    let set_filter = ctx.set_filter.clone();
    let filter_box = text_box(ctx.filter.to_string())
        .placeholder_text("Filter…")
        .width(220.0)
        .on_text_changed(move |text: String| set_filter.call(text.to_lowercase()));
    grid((
        Element::from(
            text_block(title)
                .font_size(14.0)
                .bold()
                .foreground(theme::TEXT)
                .vertical_alignment(VerticalAlignment::Center),
        )
        .grid_column(0),
        Element::from(filter_box).grid_column(2),
    ))
    .columns([GridLength::Auto, GridLength::Star(1.0), GridLength::Auto])
    .padding(Thickness::xy(ROW_INSET, 8.0))
    .into()
}

fn factions_view(ctx: &CampaignCtx) -> Element {
    let mut factions = extract_factions(ctx.doc);
    // Majors first, then alphabetical — the order a player expects.
    factions.sort_by(|a, b| b.is_major.cmp(&a.is_major).then(a.name.cmp(&b.name)));
    let total = factions.len();
    let majors = factions.iter().filter(|f| f.is_major).count();
    if !ctx.filter.is_empty() {
        factions.retain(|f| {
            f.name.to_lowercase().contains(ctx.filter) || f.key.to_lowercase().contains(ctx.filter)
        });
    }

    let header = border(
        hstack((
            campaign_header_cell("", Some(14.0)),
            campaign_header_cell("Faction", Some(FCOL_NAME)),
            campaign_header_cell("Key", Some(FCOL_KEY)),
            campaign_header_cell("Status", Some(FCOL_STATUS)),
            campaign_header_cell("Treasury", Some(FCOL_MONEY)),
            campaign_header_cell("", None),
        ))
        .spacing(10.0),
    )
    .background(theme::HEADER)
    .border_brush(theme::BORDER)
    .border_thickness(Thickness { left: 0.0, top: 0.0, right: 0.0, bottom: 1.0 })
    .padding(Thickness { left: ROW_INSET, top: 6.0, right: ROW_INSET, bottom: 6.0 });

    let mut rows: Vec<Element> = Vec::with_capacity(factions.len() + 1);
    for (index, f) in factions.iter().enumerate() {
        rows.push(faction_row(ctx, f, index));
    }
    if rows.is_empty() {
        rows.push(
            text_block(if total == 0 {
                "No campaign factions found in this file."
            } else {
                "No faction matches the filter."
            })
            .foreground(ThemeRef::SecondaryText)
            .padding(12.0)
            .into(),
        );
    }

    grid((
        campaign_toolbar(
            format!("Factions — {total} ({majors} major)"),
            ctx,
        )
        .grid_row(0),
        Element::from(header).grid_row(1),
        Element::from(scroll_viewer(vstack(rows).spacing(0.0)))
            .with_key("factions-list")
            .grid_row(2),
    ))
    .rows([GridLength::Auto, GridLength::Auto, GridLength::Star(1.0)])
    .into()
}

fn faction_row(ctx: &CampaignCtx, f: &FactionRow, index: usize) -> Element {
    let status: Element = if f.destroyed {
        text_block("Destroyed")
            .width(FCOL_STATUS)
            .foreground(theme::CRIMSON)
            .into()
    } else if f.is_major {
        text_block("Major").width(FCOL_STATUS).foreground(ThemeRef::PrimaryText).into()
    } else {
        text_block("Minor").width(FCOL_STATUS).foreground(ThemeRef::SecondaryText).into()
    };

    let treasury: Element = match f.treasury {
        Some((value_id, amount)) => numeric_cell(
            ctx.doc,
            value_id,
            EsfValue::I32(amount),
            FCOL_MONEY,
            ctx.edits,
            ctx.set_edits,
            ctx.edit_mode,
        ),
        None => text_block("—")
            .width(FCOL_MONEY)
            .foreground(ThemeRef::SecondaryText)
            .into(),
    };

    Element::from(campaign_row_border(
        index,
        hstack((
            colour_swatch(f.color),
            text_block(f.name.clone()).width(FCOL_NAME).foreground(if f.destroyed {
                ThemeRef::SecondaryText
            } else {
                ThemeRef::PrimaryText
            }),
            text_block(f.key.clone())
                .width(FCOL_KEY)
                .foreground(ThemeRef::SecondaryText),
            status,
            treasury,
            locate_button(ctx, f.node),
        ))
        .spacing(10.0),
    ))
    .with_key(format!("fac-{}", f.node))
}

fn regions_view(ctx: &CampaignCtx) -> Element {
    let factions = extract_factions(ctx.doc);
    let owner_names: HashMap<u32, &str> =
        factions.iter().map(|f| (f.id, f.name.as_str())).collect();
    let mut regions = extract_regions(ctx.doc);
    regions.sort_by(|a, b| a.theatre.cmp(&b.theatre).then(a.key.cmp(&b.key)));
    let total = regions.len();
    if !ctx.filter.is_empty() {
        regions.retain(|r| {
            r.key.to_lowercase().contains(ctx.filter)
                || r.theatre.to_lowercase().contains(ctx.filter)
                || owner_names
                    .get(&r.owner_faction)
                    .is_some_and(|n| n.to_lowercase().contains(ctx.filter))
        });
    }

    let header = border(
        hstack((
            campaign_header_cell("Region", Some(RCOL_KEY)),
            campaign_header_cell("Theatre", Some(RCOL_THEATRE)),
            campaign_header_cell("Owner", Some(RCOL_OWNER)),
            campaign_header_cell("Population", Some(RCOL_POP)),
            campaign_header_cell("Town wealth", Some(FCOL_MONEY)),
            campaign_header_cell("", None),
        ))
        .spacing(10.0),
    )
    .background(theme::HEADER)
    .border_brush(theme::BORDER)
    .border_thickness(Thickness { left: 0.0, top: 0.0, right: 0.0, bottom: 1.0 })
    .padding(Thickness { left: ROW_INSET, top: 6.0, right: ROW_INSET, bottom: 6.0 });

    let mut rows: Vec<Element> = Vec::with_capacity(regions.len());
    for (index, r) in regions.iter().enumerate() {
        rows.push(region_row(ctx, r, &owner_names, index));
    }
    if rows.is_empty() {
        rows.push(
            text_block(if total == 0 {
                "No campaign regions found in this file."
            } else {
                "No region matches the filter."
            })
            .foreground(ThemeRef::SecondaryText)
            .padding(12.0)
            .into(),
        );
    }

    grid((
        campaign_toolbar(format!("Regions — {total}"), ctx).grid_row(0),
        Element::from(header).grid_row(1),
        Element::from(scroll_viewer(vstack(rows).spacing(0.0)))
            .with_key("regions-list")
            .grid_row(2),
    ))
    .rows([GridLength::Auto, GridLength::Auto, GridLength::Star(1.0)])
    .into()
}

fn region_row(
    ctx: &CampaignCtx,
    r: &RegionRow,
    owner_names: &HashMap<u32, &str>,
    index: usize,
) -> Element {
    let owner = owner_names
        .get(&r.owner_faction)
        .map(|n| (*n).to_string())
        .unwrap_or_else(|| "—".to_string());
    let wealth: Element = match r.town_wealth {
        Some((value_id, amount)) => numeric_cell(
            ctx.doc,
            value_id,
            EsfValue::U32(amount),
            FCOL_MONEY,
            ctx.edits,
            ctx.set_edits,
            ctx.edit_mode,
        ),
        None => text_block("—")
            .width(FCOL_MONEY)
            .foreground(ThemeRef::SecondaryText)
            .into(),
    };

    Element::from(campaign_row_border(
        index,
        hstack((
            text_block(r.key.clone()).width(RCOL_KEY),
            text_block(r.theatre.clone())
                .width(RCOL_THEATRE)
                .foreground(ThemeRef::SecondaryText),
            text_block(owner).width(RCOL_OWNER).foreground(ThemeRef::SecondaryText),
            text_block(
                r.population
                    .map(|p| fmt_count(p as usize))
                    .unwrap_or_else(|| "—".to_string()),
            )
            .width(RCOL_POP)
            .foreground(ThemeRef::SecondaryText),
            wealth,
            locate_button(ctx, r.node),
        ))
        .spacing(10.0),
    ))
    .with_key(format!("reg-{}", r.node))
}

// --- Pending edits drawer --------------------------------------------------

/// Bottom drawer listing every staged edit with its node path and a revert
/// action. Shown whenever edits exist, in every view.
#[allow(clippy::too_many_arguments)]
fn pending_edits_drawer(
    doc: &EsfDocument,
    descs: &DescState,
    edits: &Edits,
    set_edits: &AsyncSetState<Edits>,
    set_view: &AsyncSetState<MainView>,
    set_selected: &AsyncSetState<Option<NodeId>>,
    expanded: &HashSet<NodeId>,
    set_expanded: &AsyncSetState<HashSet<NodeId>>,
) -> Element {
    // Sort by file offset for a stable, file-ordered list.
    let mut entries: Vec<(u32, &EsfEdit)> = edits.iter().map(|(k, v)| (*k, v)).collect();
    entries.sort_by_key(|(vid, _)| doc.values.get(*vid as usize).map_or(0, |r| r.offset));

    let discard_all = {
        let set_edits = set_edits.clone();
        button("Discard all")
            .on_click(move || set_edits.call(Edits::new()))
            .tooltip("Revert every staged change")
    };
    let header = grid((
        Element::from(
            text_block(format!(
                "Pending changes ({}) — Ctrl+S to save",
                entries.len()
            ))
            .bold()
            .font_size(12.0)
            .foreground(ThemeRef::AccentText)
            .vertical_alignment(VerticalAlignment::Center),
        )
        .grid_column(0),
        Element::from(discard_all).grid_column(2),
    ))
    .columns([GridLength::Auto, GridLength::Star(1.0), GridLength::Auto])
    .padding(Thickness::xy(ROW_INSET, 4.0));

    let mut rows: Vec<Element> = Vec::new();
    for (index, (value_id, edit)) in entries.iter().enumerate() {
        if index >= MAX_EDIT_ROWS {
            rows.push(
                text_block(format!("… {} more", entries.len() - MAX_EDIT_ROWS))
                    .foreground(ThemeRef::SecondaryText)
                    .into(),
            );
            break;
        }
        let Some(record) = doc.values.get(*value_id as usize) else {
            continue;
        };
        let owner = doc.find_owning_node(record.offset);
        let (path, label) = match owner {
            Some(node) => {
                let value_entries: Vec<_> = doc.node_value_entries(node).collect();
                let classes: Vec<&'static str> = value_entries
                    .iter()
                    .map(|(_, rec)| descriptions::type_class(&rec.value))
                    .collect();
                let pos = value_entries.iter().position(|(vid, _)| vid == value_id);
                let label = pos.and_then(|p| {
                    descs.0.as_ref().and_then(|d| {
                        d.label(doc.node_name(node), &classes, p, "")
                    })
                });
                (doc.node_path(node), label)
            }
            None => (String::from("?"), None),
        };
        let old_text = doc.format_value(&record.value);
        let new_text = match edit {
            EsfEdit::Value(v) => doc.format_value(v),
            EsfEdit::Text(s) => s.clone(),
        };
        let revert = {
            let set_edits = set_edits.clone();
            let edits = edits.clone();
            let value_id = *value_id;
            button("Revert").on_click(move || {
                let mut next = edits.clone();
                next.remove(&value_id);
                set_edits.call(next);
            })
        };
        let locate = {
            let set_view = set_view.clone();
            let set_selected = set_selected.clone();
            let set_expanded = set_expanded.clone();
            let expanded = expanded.clone();
            let chain: Vec<NodeId> = owner
                .map(|node| {
                    let mut chain = Vec::new();
                    let mut current = node;
                    loop {
                        let parent = doc.node(current).parent;
                        if parent == NO_PARENT {
                            break;
                        }
                        chain.push(parent);
                        current = parent;
                    }
                    chain
                })
                .unwrap_or_default();
            button("Locate").on_click(move || {
                if let Some(node) = owner {
                    let mut next = expanded.clone();
                    next.extend(chain.iter().copied());
                    set_expanded.call(next);
                    set_selected.call(Some(node));
                    set_view.call(MainView::Explorer);
                }
            })
        };
        let what = match label {
            Some(l) => format!("{path}  ·  {l}"),
            None => path,
        };
        rows.push(
            Element::from(
                border(
                    hstack((
                        text_block(what).foreground(ThemeRef::SecondaryText).font_size(12.0),
                        text_block(format!("{old_text}  →  {new_text}"))
                            .foreground(ThemeRef::AccentText)
                            .font_size(12.0),
                        Element::from(revert),
                        Element::from(locate),
                    ))
                    .spacing(14.0),
                )
                .border_brush(theme::BORDER)
                .border_thickness(Thickness { left: 0.0, top: 1.0, right: 0.0, bottom: 0.0 })
                .padding(Thickness::xy(ROW_INSET, 2.0)),
            )
            .with_key(format!("edit-{value_id}")),
        );
    }

    Element::from(
        border(
            grid((
                Element::from(header).grid_row(0),
                Element::from(scroll_viewer(vstack(rows).spacing(0.0)))
                    .max_height(150.0)
                    .grid_row(1),
            ))
            .rows([GridLength::Auto, GridLength::Auto]),
        )
        .background(theme::HEADER)
        .border_brush(theme::BORDER)
        .border_thickness(Thickness { left: 0.0, top: 1.0, right: 0.0, bottom: 0.0 }),
    )
}

// --- App shell --------------------------------------------------------------

/// One tab of the main view switcher: label + gold underline when active.
fn nav_tab(label: &str, active: bool, on_click: impl Fn() + 'static) -> Element {
    let indicator = Element::from(border(text_block("")))
        .background(if active {
            theme::ACCENT
        } else {
            Color { a: 0, r: 0, g: 0, b: 0 }
        })
        .height(2.0);
    let tab = Element::from(
        button(label)
            .on_click(on_click),
    )
    .background(if active { theme::HEADER } else { theme::BASE })
    .foreground(if active { theme::ACCENT_BRIGHT } else { theme::TEXT_DIM });
    vstack((tab, indicator)).spacing(0.0).into()
}

fn app_shell(cx: &mut RenderCx) -> Element {
    let (doc_state, set_doc_state) = cx.use_async_state(DocState::default());
    let (descs, set_descs) = cx.use_async_state(DescState::default());
    let (is_busy, set_is_busy) = cx.use_async_state(false);
    let (status, set_status) = cx.use_async_state(String::new());
    let (has_autoloaded, set_has_autoloaded) = cx.use_state(false);

    let (expanded, set_expanded) = cx.use_async_state(HashSet::<NodeId>::from([0]));
    let (selected, set_selected) = cx.use_async_state(None::<NodeId>);
    let (edits, set_edits) = cx.use_async_state(Edits::new());

    let (search_query, set_search_query) = cx.use_state(String::new());
    let (search_results, set_search_results) = cx.use_async_state(Vec::<(NodeId, String)>::new());
    let (is_searching, set_is_searching) = cx.use_async_state(false);

    let (tree_width, set_tree_width) = cx.use_state(320.0_f64);
    let (dragging, set_dragging) = cx.use_state(false);
    let (edit_mode, set_edit_mode) = cx.use_state(false);
    let (recent, set_recent) = cx.use_async_state(Vec::<String>::new());
    let (view, set_view) = cx.use_async_state(MainView::default());
    let (campaign_filter, set_campaign_filter) = cx.use_async_state(String::new());
    // Flyout item clicks fire closures captured when the flyout was first
    // built (menu items don't change, so the backend never re-wires them).
    // Those stale closures must not touch document snapshots directly —
    // instead they write the request into state, and render acts on it.
    let (pending_goto, set_pending_goto) = cx.use_state(None::<String>);
    let (pending_open, set_pending_open) = cx.use_state(None::<String>);

    // --- File loading ---
    let start_load = {
        let set_doc = set_doc_state.clone();
        let set_busy = set_is_busy.clone();
        let set_status = set_status.clone();
        let set_expanded = set_expanded.clone();
        let set_selected = set_selected.clone();
        let set_results = set_search_results.clone();
        let set_edits = set_edits.clone();
        let set_recent = set_recent.clone();
        move |path: String| {
            set_busy.call(true);
            set_status.call(format!("Loading {path}…"));
            set_expanded.call(HashSet::from([0]));
            set_selected.call(None);
            set_results.call(Vec::new());
            set_edits.call(Edits::new());
            let set_doc = set_doc.clone();
            let set_busy = set_busy.clone();
            let set_status = set_status.clone();
            let set_recent = set_recent.clone();
            std::thread::spawn(move || {
                let started = std::time::Instant::now();
                info!("Starting to load file: {}", path);
                match esf_parser::parser::load_file(&path) {
                    Ok(doc) => {
                        let elapsed = started.elapsed();
                        info!("Successfully loaded {} in {:?}", path, elapsed);
                        set_status.call(format!(
                            "Opened {path} in {:.2?}",
                            elapsed
                        ));
                        set_recent.call(push_recent(&path));
                        set_doc.call(DocState {
                            doc: Some(Arc::new(doc)),
                            path,
                        });
                    }
                    Err(e) => {
                        error!("Failed to open {}: {}", path, e);
                        set_status.call(format!("Failed to open {path}: {e}"));
                        set_doc.call(DocState::default());

                        let _ = rfd::MessageDialog::new()
                            .set_title("twedit - Load Error")
                            .set_level(rfd::MessageLevel::Error)
                            .set_description(&format!("Failed to open {}:\n\n{}", path, e))
                            .show();
                    }
                }
                set_busy.call(false);
            });
        }
    };

    if !has_autoloaded {
        set_has_autoloaded.call(true);
        // Pin the app to its own theme, independent of the Windows setting.
        set_requested_theme(RequestedTheme::Dark);
        // Window/taskbar icon: AppWindow.SetIcon wants a file path, so
        // materialize the embedded .ico into the temp dir.
        let icon_path = std::env::temp_dir().join("twedit-icon.ico");
        if std::fs::write(&icon_path, include_bytes!("../assets/icon.ico") as &[u8]).is_ok() {
            set_window_icon(icon_path.to_string_lossy().into_owned());
        }
        // Node descriptions embedded in the executable: legacy XML plus
        // twedit's curated schema (which wins on conflicts).
        let set_descs = set_descs.clone();
        std::thread::spawn(move || {
            let xml_str = include_str!("../assets/NodesDescriptions.xml");
            let toml_str = include_str!("../assets/esf_schema.toml");
            let mut d = descriptions::load(xml_str, toml_str);
            if let Some(locs) = esf_parser::pack_parser::get_etw_localisation() {
                d.loc_map = locs;
            }
            set_descs.call(DescState(Some(Arc::new(d))));
        });
        set_recent.call(load_recent());
        if std::path::Path::new(DEFAULT_SAVE_PATH).exists() {
            start_load(DEFAULT_SAVE_PATH.to_string());
        } else {
            set_status.call("Use Open to load a save file.".to_string());
        }
    }

    // --- Open / Save / Save As ---
    let on_open = {
        let start_load = start_load.clone();
        move || {
            let start_load = start_load.clone();
            std::thread::spawn(move || {
                let picked = rfd::FileDialog::new()
                    .add_filter("Total War saves / ESF", &["esf", "empire_save", "empire_save_multiplayer"])
                    .add_filter("All files", &["*"])
                    .pick_file();
                if let Some(path) = picked {
                    start_load(path.display().to_string());
                }
            });
        }
    };

    let save_to = {
        let doc_state = doc_state.clone();
        let edits = edits.clone();
        let set_doc = set_doc_state.clone();
        let set_edits = set_edits.clone();
        let set_status = set_status.clone();
        let set_busy = set_is_busy.clone();
        move |path: String| {
            let Some(doc) = doc_state.doc.clone() else {
                return;
            };
            let edits = edits.clone();
            let set_doc = set_doc.clone();
            let set_edits = set_edits.clone();
            let set_status = set_status.clone();
            let set_busy = set_busy.clone();
            set_busy.call(true);
            set_status.call(format!("Saving {path}…"));
            std::thread::spawn(move || {
                info!("Starting to save file: {}", path);
                let (bytes, applied) = doc.bytes_with_edits(&edits);
                match std::fs::write(&path, &bytes) {
                    Ok(()) => {
                        info!("Successfully wrote file {}. Applied {} edits. Re-parsing...", path, applied);
                        // Refresh the document from the just-written bytes so
                        // Original columns and future edits see current data.
                        match esf_parser::parser::parse_bytes(bytes) {
                            Ok(new_doc) => {
                                info!("Successfully re-parsed file {}", path);
                                set_doc.call(DocState {
                                    doc: Some(Arc::new(new_doc)),
                                    path: path.clone(),
                                });
                                set_edits.call(Edits::new());
                                set_status
                                    .call(format!("Saved {applied} change(s) to {path}"));
                            }
                            Err(e) => {
                                error!("Saved, but failed to re-parse {}: {}", path, e);
                                set_status.call(format!("Saved, but failed to re-parse: {e}"));
                                let _ = rfd::MessageDialog::new()
                                    .set_title("twedit - Re-parse Error")
                                    .set_level(rfd::MessageLevel::Warning)
                                    .set_description(&format!("The file was saved, but twedit failed to re-parse it:\n\n{}", e))
                                    .show();
                            }
                        }
                    }
                    Err(e) => {
                        error!("Failed to save {}: {}", path, e);
                        set_status.call(format!("Failed to save {path}: {e}"));
                        let _ = rfd::MessageDialog::new()
                            .set_title("twedit - Save Error")
                            .set_level(rfd::MessageLevel::Error)
                            .set_description(&format!("Failed to save {}:\n\n{}", path, e))
                            .show();
                    }
                }
                set_busy.call(false);
            });
        }
    };

    let on_save = {
        let save_to = save_to.clone();
        let path = doc_state.path.clone();
        move || {
            if !path.is_empty() {
                save_to(path.clone());
            }
        }
    };

    let on_save_as = {
        let save_to = save_to.clone();
        move || {
            let save_to = save_to.clone();
            std::thread::spawn(move || {
                let picked = rfd::FileDialog::new()
                    .add_filter("Total War saves / ESF", &["esf", "empire_save", "empire_save_multiplayer"])
                    .add_filter("All files", &["*"])
                    .save_file();
                if let Some(path) = picked {
                    save_to(path.display().to_string());
                }
            });
        }
    };

    // --- Search (shared by the search box, live typing, and Go To menu) ---
    let run_search = {
        let doc_state = doc_state.clone();
        let set_results = set_search_results.clone();
        let set_searching = set_is_searching.clone();
        move |query: String| {
            let Some(doc) = doc_state.doc.clone() else {
                return;
            };
            let set_results = set_results.clone();
            let set_searching = set_searching.clone();
            let ticket = SEARCH_GEN.fetch_add(1, Ordering::Relaxed) + 1;
            set_searching.call(true);
            std::thread::spawn(move || {
                let hits = doc.search_nodes(&query, SEARCH_RESULT_LIMIT);
                // A newer search superseded this one: drop the results.
                if SEARCH_GEN.load(Ordering::Relaxed) != ticket {
                    return;
                }
                let results: Vec<(NodeId, String)> = hits
                    .into_iter()
                    .map(|id| (id, doc.node_path(id)))
                    .collect();
                set_results.call(results);
                set_searching.call(false);
            });
        }
    };
    let on_search = {
        let run_search = run_search.clone();
        let query = search_query.clone();
        move || run_search(query.clone())
    };

    // Execute requests queued by flyout menu clicks (see state comment above).
    if let Some(query) = pending_goto.clone() {
        set_pending_goto.call(None);
        run_search(query);
    }
    if let Some(path) = pending_open.clone() {
        set_pending_open.call(None);
        start_load(path);
    }

    // --- Toolbar ---
    let has_doc = doc_state.doc.is_some();
    let save_label = if edits.is_empty() {
        "Save".to_string()
    } else {
        format!("Save ({})", edits.len())
    };

    let recent_button = {
        let set_pending_open = set_pending_open.clone();
        button("Recent")
            .menu_flyout(recent.iter().map(|p| menu_item(p.clone())).collect())
            .on_item_clicked(move |path: String| set_pending_open.call(Some(path)))
            .enabled(!recent.is_empty())
    };
    let goto_button = {
        let set_pending_goto = set_pending_goto.clone();
        button("Go To")
            .icon(Symbol::Go)
            .menu_flyout(GO_TO_TARGETS.iter().map(|t| menu_item(*t)).collect())
            .on_item_clicked(move |name: String| set_pending_goto.call(Some(name)))
            .enabled(has_doc)
    };
    let collapse_button = {
        let set_expanded = set_expanded.clone();
        button("Collapse")
            .icon(Symbol::Back)
            .on_click(move || set_expanded.call(HashSet::from([0])))
            .tooltip("Collapse the tree back to the root")
            .enabled(has_doc)
    };
    let logs_button = button("Logs")
        .icon(Symbol::Document)
        .on_click(|| {
            let appdata = std::env::var_os("APPDATA")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| std::path::PathBuf::from("."));
            let log_dir = appdata.join("twedit").join("logs");
            let _ = std::process::Command::new("explorer").arg(log_dir).spawn();
        })
        .tooltip("Open the log folder");

    let wordmark = hstack((
        text_block("Twedit")
            .font_family("Georgia")
            .font_size(17.0)
            .bold()
            .foreground(theme::ACCENT)
            .vertical_alignment(VerticalAlignment::Center),
        text_block("TOTAL WAR SAVE EDITOR")
            .font_size(9.0)
            .foreground(theme::TEXT_DIM)
            .vertical_alignment(VerticalAlignment::Center),
    ))
    .spacing(8.0);

    let toolbar_left = hstack((
        Element::from(wordmark).margin(Thickness {
            left: 2.0,
            top: 0.0,
            right: 8.0,
            bottom: 0.0,
        }),
        button("Open")
            .icon(Symbol::OpenFile)
            .on_click(on_open.clone())
            .tooltip("Open a save file (Ctrl+O)"),
        recent_button,
        button(save_label)
            .icon(Symbol::Save)
            .on_click(on_save.clone())
            .tooltip("Save staged changes (Ctrl+S)")
            .enabled(has_doc),
        button("Save As")
            .icon(Symbol::SaveLocal)
            .on_click(on_save_as.clone())
            .tooltip("Save to a new file (Ctrl+Shift+S)")
            .enabled(has_doc),
        toolbar_divider(),
        goto_button,
        collapse_button,
        toolbar_divider(),
        logs_button,
    ))
    .spacing(6.0);

    let toolbar_right = hstack((
        text_box(search_query.clone())
            .placeholder_text("Search nodes…")
            .width(220.0)
            .on_text_changed({
                let set_query = set_search_query.clone();
                let run_search = run_search.clone();
                let set_results = set_search_results.clone();
                let set_searching = set_is_searching.clone();
                move |text: String| {
                    set_query.call(text.clone());
                    // Search-as-you-type; short/cleared queries clear results.
                    if text.trim().len() >= LIVE_SEARCH_MIN_CHARS {
                        run_search(text);
                    } else {
                        // Invalidate any in-flight search; it must not
                        // repopulate the just-cleared results.
                        SEARCH_GEN.fetch_add(1, Ordering::Relaxed);
                        set_results.call(Vec::new());
                        set_searching.call(false);
                    }
                }
            }),
        button(if is_searching { "Searching…" } else { "Search" })
            .icon(Symbol::Find)
            .on_click(on_search)
            .enabled(has_doc),
        toolbar_divider(),
        Element::from(
            ToggleSwitch::new(edit_mode)
                .on_content("Edit")
                .off_content("View")
                .on_toggled({
                    let set_edit_mode = set_edit_mode.clone();
                    move |on| set_edit_mode.call(on)
                }),
        )
        .vertical_alignment(VerticalAlignment::Center),
    ))
    .spacing(6.0);

    let toolbar = grid((
        toolbar_left.grid_column(0),
        toolbar_right.grid_column(2),
    ))
    .columns([GridLength::Auto, GridLength::Star(1.0), GridLength::Auto]);

    // --- View switcher strip ---
    let nav_strip: Element = {
        let mk = |label: &str, target: MainView| {
            let set_view = set_view.clone();
            nav_tab(label, view == target, move || set_view.call(target))
        };
        Element::from(
            border(
                hstack((
                    mk("Explorer", MainView::Explorer),
                    mk("Factions", MainView::Factions),
                    mk("Regions", MainView::Regions),
                ))
                .spacing(2.0),
            )
            .background(theme::BASE)
            .border_brush(theme::BORDER)
            .border_thickness(Thickness { left: 0.0, top: 0.0, right: 0.0, bottom: 1.0 })
            .padding(Thickness { left: 10.0, top: 4.0, right: 10.0, bottom: 0.0 }),
        )
    };

    // --- Tree pane ---
    let tree: Element = if is_busy && !has_doc {
        Element::from(
            vstack((
                ProgressRing::indeterminate(),
                text_block("Parsing save file…"),
            ))
            .spacing(8.0)
            .padding(12.0),
        )
        .transition(
            Some(AnimationConfig::fade_in(Duration::from_millis(200))),
            None,
        )
    } else if let Some(doc) = &doc_state.doc {
        let defs = build_tree(doc, &expanded);
        let on_invoked = {
            let doc = doc.clone();
            let expanded = expanded.clone();
            let set_expanded = set_expanded.clone();
            let set_selected = set_selected.clone();
            move |label: String| {
                let Some(id) = label_node_id(&label) else {
                    return;
                };
                if (id as usize) >= doc.nodes.len() {
                    return;
                }
                set_selected.call(Some(id));
                // Toggle drill-down on structural nodes with children.
                if doc.child_count(id) > 0 {
                    let mut next = expanded.clone();
                    if !next.insert(id) {
                        next.remove(&id);
                    }
                    set_expanded.call(next);
                }
            }
        };
        Element::from(tree_view(defs).on_item_invoked(on_invoked))
            .font_size(13.0)
            .padding(Thickness::xy(4.0, 4.0))
            .with_key("tree-loaded")
            .transition(
                Some(AnimationConfig::fade_in(Duration::from_millis(250))),
                None,
            )
    } else {
        text_block("No file loaded.").padding(12.0).into()
    };

    // --- Value grid ---
    let rows: Vec<Element> = match (&doc_state.doc, selected) {
        (Some(doc), Some(id)) if (id as usize) < doc.nodes.len() => {
            value_rows(doc, id, &descs, &edits, &set_edits, edit_mode)
        }
        (Some(_), None) => vec![text_block("Select a node to view its values.")
            .foreground(ThemeRef::SecondaryText)
            .into()],
        _ => Vec::new(),
    };

    // Breadcrumb path + node details for the selected node.
    let node_info: Element = match (&doc_state.doc, selected) {
        (Some(doc), Some(id)) if (id as usize) < doc.nodes.len() => {
            // Ancestor chain root → node, as breadcrumb segments.
            let mut chain = vec![id];
            let mut current = id;
            loop {
                let parent = doc.node(current).parent;
                if parent == NO_PARENT {
                    break;
                }
                chain.push(parent);
                current = parent;
            }
            chain.reverse();
            let segments: Vec<String> = chain
                .iter()
                .map(|&n| {
                    if doc.node(n).kind == NodeKind::Record {
                        format!("{}[]", doc.node_name(n))
                    } else {
                        doc.node_name(n).to_string()
                    }
                })
                .collect();
            let crumbs = {
                let set_selected = set_selected.clone();
                let chain = chain.clone();
                BreadcrumbBar::new(segments)
                    .on_item_clicked(move |index: i32| {
                        if let Some(&node) = chain.get(index as usize) {
                            set_selected.call(Some(node));
                        }
                    })
            };

            let node = doc.node(id);
            let info = text_block(format!(
                "{:?} v{} · offset 0x{:x}..0x{:x} · {} children · {} values",
                node.kind,
                node.version,
                node.offset,
                node.offset_end,
                fmt_count(doc.child_count(id)),
                fmt_count(doc.node_values(id).count()),
            ))
            .foreground(ThemeRef::SecondaryText)
            .font_size(12.0);
            // Schema doc line for the node type, when we have one.
            // (text_block can't wrap in this reactor version, so long docs
            // are truncated; the full text lives in esf_schema.toml.)
            let mut stack: Vec<Element> = vec![Element::from(crumbs), info.into()];
            if let Some(doc_line) = descs.0.as_ref().and_then(|d| d.doc(doc.node_name(id))) {
                let mut text: String = doc_line.chars().take(220).collect();
                if text.len() < doc_line.len() {
                    text.push('…');
                }
                stack.push(
                    text_block(text)
                        .foreground(ThemeRef::AccentText)
                        .font_size(12.0)
                        .into(),
                );
            }
            vstack(stack).spacing(2.0).into()
        }
        _ => text_block("").into(),
    };

    let mut right_rows: Vec<Element> = vec![
        node_info.padding(Thickness::xy(8.0, 6.0)).grid_row(0),
        grid_header().grid_row(1),
        Element::from(scroll_viewer(vstack(rows).spacing(0.0)))
            .with_key(format!("vals-{selected:?}"))
            .transition(
                Some(AnimationConfig::fade_in(Duration::from_millis(150))),
                None,
            )
            .grid_row(2),
    ];
    let mut right_row_defs = vec![
        GridLength::Auto,
        GridLength::Auto,
        GridLength::Star(1.0),
    ];
    if !search_results.is_empty() {
        let result_labels: Vec<String> = search_results
            .iter()
            .map(|(_, path)| path.clone())
            .collect();
        let on_result_selected = {
            let doc_state = doc_state.clone();
            let results = search_results.clone();
            let expanded = expanded.clone();
            let set_expanded = set_expanded.clone();
            let set_selected = set_selected.clone();
            move |index: i32| {
                let Some(doc) = doc_state.doc.as_ref() else {
                    return;
                };
                let Some((id, _)) = results.get(index as usize) else {
                    return;
                };
                set_selected.call(Some(*id));
                let mut next = expanded.clone();
                expand_path_to(doc, *id, &mut next);
                set_expanded.call(next);
            }
        };
        right_rows.push(
            text_block(format!("Search results ({})", search_results.len()))
                .bold()
                .grid_row(3)
                .into(),
        );
        right_rows.push(
            Element::from(
                list_box()
                    .items(result_labels)
                    .on_selection_changed(on_result_selected),
            )
            .max_height(220.0)
            .transition(
                Some(AnimationConfig::fade_in(Duration::from_millis(200))),
                None,
            )
            .grid_row(4),
        );
        right_row_defs.push(GridLength::Auto);
        right_row_defs.push(GridLength::Auto);
    }
    let right_pane = grid(right_rows).rows(right_row_defs);

    // --- Status bar: muted dark strip with a small accent tick and
    // right-aligned document stats / pending-edit count ---
    let status_left = hstack((
        Element::from(border(text_block("")))
            .background(theme::STATUS_TICK)
            .width(3.0)
            .height(14.0)
            .vertical_alignment(VerticalAlignment::Center),
        text_block(status.clone())
            .font_size(12.0)
            .foreground(ThemeRef::SecondaryText)
            .vertical_alignment(VerticalAlignment::Center),
    ))
    .spacing(8.0);

    let mut status_right_items: Vec<Element> = Vec::new();
    if !edits.is_empty() {
        status_right_items.push(
            text_block(format!(
                "{} pending edit{}",
                edits.len(),
                if edits.len() == 1 { "" } else { "s" }
            ))
            .font_size(12.0)
            .foreground(ThemeRef::AccentText)
            .into(),
        );
    }
    if edit_mode {
        status_right_items.push(
            text_block("EDIT MODE")
                .font_size(11.0)
                .bold()
                .foreground(ThemeRef::AccentText)
                .into(),
        );
    }
    if let Some(doc) = &doc_state.doc {
        status_right_items.push(
            text_block(format!(
                "{:?} · {:.1} MB · {} nodes · {} values",
                doc.header.magic,
                doc.data.len() as f64 / 1_000_000.0,
                fmt_count(doc.nodes.len()),
                fmt_count(doc.values.len())
            ))
            .font_size(12.0)
            .foreground(ThemeRef::SecondaryText)
            .into(),
        );
    }
    let status_right = hstack(status_right_items).spacing(14.0);

    let status_bar = border(
        grid((
            status_left.grid_column(0),
            status_right.grid_column(1),
        ))
        .columns([GridLength::Star(1.0), GridLength::Auto]),
    )
    .background(theme::HEADER)
    .border_brush(theme::BORDER)
    .border_thickness(Thickness {
        left: 0.0,
        top: 1.0,
        right: 0.0,
        bottom: 0.0,
    })
    .padding(Thickness::xy(10.0, 5.0));

    // --- Body ---
    let body: Element = match (view, &doc_state.doc) {
        (MainView::Explorer, _) => {
            // Tree card | draggable gap | content card.
            let left_card = card(tree, theme::PANEL)
                .margin(Thickness {
                    left: 10.0,
                    top: 10.0,
                    right: 0.0,
                    bottom: 10.0,
                })
                .grid_column(0);

            let handle_line = Element::from(border(text_block("")))
                .background(if dragging { theme::ACCENT } else { theme::BORDER })
                .width(2.0)
                .height(48.0)
                .horizontal_alignment(HorizontalAlignment::Center)
                .vertical_alignment(VerticalAlignment::Center);
            let splitter = {
                let set_drag = set_dragging.clone();
                Element::from(border(handle_line))
                    .background(theme::BASE)
                    .on_pointer_pressed(move |_: PointerEventInfo| set_drag.call(true))
                    .grid_column(1)
            };

            let right_card = card(right_pane, theme::PANEL)
                .margin(Thickness {
                    left: 0.0,
                    top: 10.0,
                    right: 10.0,
                    bottom: 10.0,
                })
                .grid_column(2);

            let mut body_children: Vec<Element> = vec![left_card, splitter, right_card];
            if dragging {
                // Full-body transparent layer so the drag keeps tracking even
                // when the pointer leaves the thin splitter strip.
                let set_width = set_tree_width.clone();
                let stop_a = set_dragging.clone();
                let stop_b = set_dragging.clone();
                body_children.push(
                    Element::from(border(text_block("")))
                        .background(Color { a: 1, r: 0, g: 0, b: 0 })
                        .grid_column(0)
                        .grid_column_span(3)
                        .on_pointer_moved(move |info: PointerEventInfo| {
                            if info.is_left_button_pressed {
                                set_width.call((info.x - SPLITTER_W / 2.0).clamp(200.0, 680.0));
                            } else {
                                stop_a.call(false);
                            }
                        })
                        .on_pointer_released(move |_: PointerEventInfo| stop_b.call(false)),
                );
            }
            grid(body_children)
                .columns([
                    GridLength::Pixel(tree_width),
                    GridLength::Pixel(SPLITTER_W),
                    GridLength::Star(1.0),
                ])
                .into()
        }
        (campaign_view, Some(doc)) => {
            let ctx = CampaignCtx {
                doc,
                edits: &edits,
                set_edits: &set_edits,
                edit_mode,
                filter: campaign_filter.as_str(),
                set_filter: &set_campaign_filter,
                set_view: &set_view,
                set_selected: &set_selected,
                expanded: &expanded,
                set_expanded: &set_expanded,
            };
            let content = if campaign_view == MainView::Factions {
                factions_view(&ctx)
            } else {
                regions_view(&ctx)
            };
            card(content, theme::PANEL)
                .margin(Thickness::uniform(10.0))
                .with_key(if campaign_view == MainView::Factions {
                    "view-factions"
                } else {
                    "view-regions"
                })
                .transition(
                    Some(AnimationConfig::fade_in(Duration::from_millis(150))),
                    None,
                )
        }
        (_, None) => text_block("No file loaded.")
            .foreground(ThemeRef::SecondaryText)
            .padding(14.0)
            .into(),
    };

    let toolbar_strip = border(toolbar.padding(8.0))
        .border_brush(theme::BORDER)
        .border_thickness(Thickness {
            left: 0.0,
            top: 0.0,
            right: 0.0,
            bottom: 1.0,
        });

    // The drawer slot always exists (zero-height placeholder when empty) so
    // the shell keeps a stable row structure — removing a middle row shifts
    // every later element and confuses the positional reconciler diff.
    let drawer_slot: Element = match (&doc_state.doc, edits.is_empty()) {
        (Some(doc), false) => pending_edits_drawer(
            doc,
            &descs,
            &edits,
            &set_edits,
            &set_view,
            &set_selected,
            &expanded,
            &set_expanded,
        ),
        _ => Element::from(border(text_block(""))).height(0.0),
    };

    let shell_rows: Vec<Element> = vec![
        Element::from(toolbar_strip).grid_row(0),
        nav_strip.grid_row(1),
        body.grid_row(2),
        drawer_slot.with_key("edit-drawer").grid_row(3),
        Element::from(status_bar).grid_row(4),
    ];
    let shell_row_defs = vec![
        GridLength::Auto,
        GridLength::Auto,
        GridLength::Star(1.0),
        GridLength::Auto,
        GridLength::Auto,
    ];

    Element::from(grid(shell_rows).rows(shell_row_defs))
        .background(theme::BASE)
        .keyboard_accelerator(KeyboardAccelerator::new(
            VirtualKey::S,
            VirtualKeyModifiers::Control,
            move || on_save(),
        ))
        .keyboard_accelerator(KeyboardAccelerator::new(
            VirtualKey::S,
            VirtualKeyModifiers::Control | VirtualKeyModifiers::Shift,
            move || on_save_as(),
        ))
        .keyboard_accelerator(KeyboardAccelerator::new(
            VirtualKey::O,
            VirtualKeyModifiers::Control,
            move || on_open(),
        ))
}

fn setup_logging_and_panic() -> tracing_appender::non_blocking::WorkerGuard {
    let appdata = std::env::var_os("APPDATA")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let log_dir = appdata.join("twedit").join("logs");
    let _ = std::fs::create_dir_all(&log_dir);

    let file_appender = tracing_appender::rolling::daily(log_dir, "twedit.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    // Write to both console and file.
    tracing_subscriber::fmt()
        .with_writer(non_blocking.and(std::io::stdout))
        .with_env_filter("twedit_ui=debug,esf_parser=debug,info")
        .init();

    std::panic::set_hook(Box::new(|info| {
        let msg = match info.payload().downcast_ref::<&'static str>() {
            Some(s) => *s,
            None => match info.payload().downcast_ref::<String>() {
                Some(s) => &s[..],
                None => "Box<dyn Any>",
            },
        };
        let location = info
            .location()
            .map(|loc| format!("{}:{}:{}", loc.file(), loc.line(), loc.column()))
            .unwrap_or_else(|| "".to_string());

        error!("FATAL PANIC: {} at {}", msg, location);

        let _ = rfd::MessageDialog::new()
            .set_title("twedit - Fatal Error")
            .set_level(rfd::MessageLevel::Error)
            .set_description(&format!(
                "A fatal error occurred and the application must close.\n\nError: {}\nLocation: {}\n\nPlease check the logs in %APPDATA%\\twedit\\logs for more details.",
                msg, location
            ))
            .show();
    }));

    guard
}

fn main() -> Result<()> {
    let _log_guard = setup_logging_and_panic();
    info!("twedit application starting");

    App::new()
        .title("Twedit — Total War Save Editor")
        .inner_size(1280.0, 840.0)
        .theme_resources(theme::THEME_XAML)
        .render(app_shell)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_id_round_trips_invisibly() {
        for id in [0u32, 1, 42, 4711, 2_164_694, u32::MAX] {
            let label = format!("CAMPAIGN_ENV (3){}", encode_node_id(id));
            assert_eq!(label_node_id(&label), Some(id), "id {id}");
        }
        // The suffix must be invisible: no visible chars added.
        let encoded = encode_node_id(123);
        assert!(encoded.chars().all(|c| matches!(c, ID_MARK | ID_ZERO | ID_ONE)));
        // Labels without an encoded id (placeholder rows) decode to None.
        assert_eq!(label_node_id("… 500 more entries not shown"), None);
        assert_eq!(label_node_id("plain label"), None);
        // Truncated encoding decodes to None rather than a wrong id.
        let mut truncated = format!("x{}", encode_node_id(99));
        truncated.pop();
        assert_eq!(label_node_id(&truncated), None);
    }

    #[test]
    fn signed_thousands_formatting() {
        assert_eq!(fmt_i64(0), "0");
        assert_eq!(fmt_i64(20_000), "20,000");
        assert_eq!(fmt_i64(-1_234_567), "-1,234,567");
    }
}
