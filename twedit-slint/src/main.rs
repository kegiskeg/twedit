#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Slint front-end for twedit. Shares the `esf-parser` engine (parsing,
//! editing, and `campaign` semantic extraction) with the WinUI build; only
//! the view layer differs — no WinUI, no patched windows-rs clone.
//!
//! State (document, staged edits, tree expansion, selection) lives in a
//! single `Rc<RefCell<State>>`; callbacks mutate it and re-push the derived
//! models to the window through a weak handle. Edits stage into the same
//! `EsfEdit` map the parser's `bytes_with_edits` consumes, so saving is
//! byte-for-byte identical to the WinUI path.

slint::include_modules!();

use esf_parser::campaign::{extract_factions, find_first_node};
use esf_parser::objects::{EsfDocument, EsfEdit, NodeId, NO_PARENT};
use i_slint_backend_winit::WinitWindowAccessor;
use slint::{Color, ComponentHandle, ModelRc, SharedString, VecModel};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

const DEFAULT_SAVE_PATH: &str =
    r"C:\Projects\Rust\_old\esfeditor\saves\test_save.empire_save_multiplayer";

/// Cap on materialized rows per pane (Slint's ListView virtualizes, but the
/// tree is user-expanded so we still bound each node's children).
const MAX_TREE_CHILDREN: usize = 1000;
const MAX_VALUE_ROWS: usize = 500;

const NO_SWATCH: Color = Color::from_rgb_u8(0x1A, 0x17, 0x11);

type Edits = HashMap<u32, EsfEdit>;

#[derive(Default)]
struct State {
    doc: Option<Arc<EsfDocument>>,
    path: String,
    edits: Edits,
    expanded: HashSet<NodeId>,
    selected: Option<NodeId>,
}

/// Integer with thousands separators (`-1234567` -> `-1,234,567`).
fn fmt_int(n: i64) -> String {
    let neg = n < 0;
    let digits = n.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    if neg {
        format!("-{out}")
    } else {
        out
    }
}

fn tree_label(doc: &EsfDocument, id: NodeId) -> String {
    let count = doc.child_count(id);
    if count > 0 {
        format!("{} ({count})", doc.node_name(id))
    } else {
        doc.node_name(id).to_string()
    }
}

/// Flatten the arena into visible rows: DFS from root, descending only into
/// expanded nodes (so an unexpanded 2M-node tree materializes a handful of
/// rows). Each node carries its own id — no invisible-character hack.
fn build_tree(doc: &EsfDocument, st: &State) -> Vec<TreeRow> {
    let mut out = Vec::new();
    fn walk(doc: &EsfDocument, id: NodeId, depth: i32, st: &State, out: &mut Vec<TreeRow>) {
        let has_children = doc.child_count(id) > 0;
        let expanded = st.expanded.contains(&id);
        out.push(TreeRow {
            node_id: id as i32,
            label: tree_label(doc, id).into(),
            depth,
            has_children,
            expanded,
            selected: st.selected == Some(id),
        });
        if expanded {
            for child in doc.children(id).take(MAX_TREE_CHILDREN) {
                walk(doc, child, depth + 1, st, out);
            }
        }
    }
    walk(doc, doc.root, 0, st, &mut out);
    out
}

fn build_values(doc: &EsfDocument, id: NodeId, edits: &Edits) -> Vec<ValueRow> {
    doc.node_value_entries(id)
        .take(MAX_VALUE_ROWS)
        .map(|(vid, rec)| {
            let staged = edits.get(&vid);
            let value = match staged {
                Some(EsfEdit::Value(v)) => doc.format_value(v),
                Some(EsfEdit::Text(s)) => s.clone(),
                None => doc.format_value(&rec.value),
            };
            ValueRow {
                value_id: vid as i32,
                value: value.into(),
                original: doc.format_value(&rec.value).into(),
                type_name: EsfDocument::value_type_name(&rec.value).into(),
                label: SharedString::new(),
                edited: staged.is_some(),
                editable: rec.value.is_editable(),
            }
        })
        .collect()
}

fn build_factions(doc: &EsfDocument) -> Vec<FactionRow> {
    extract_factions(doc)
        .into_iter()
        .map(|f| FactionRow {
            name: f.name.as_str().into(),
            key: f.key.as_str().into(),
            status: if f.destroyed {
                "Destroyed"
            } else if f.is_major {
                "Major"
            } else {
                "Minor"
            }
            .into(),
            treasury: f
                .treasury
                .map(|(_, a)| fmt_int(a as i64))
                .unwrap_or_else(|| "—".into())
                .into(),
            swatch: f
                .color
                .map(|(r, g, b)| Color::from_rgb_u8(r, g, b))
                .unwrap_or(NO_SWATCH),
            major: f.is_major,
            destroyed: f.destroyed,
        })
        .collect()
}

fn model<T: Clone + 'static>(rows: Vec<T>) -> ModelRc<T> {
    ModelRc::from(Rc::new(VecModel::from(rows)))
}

/// Push tree + selection-dependent panels to the window.
fn refresh_tree(app: &AppWindow, st: &State) {
    if let Some(doc) = &st.doc {
        app.set_tree_rows(model(build_tree(doc, st)));
    } else {
        app.set_tree_rows(model(Vec::<TreeRow>::new()));
    }
}

fn refresh_values(app: &AppWindow, st: &State) {
    match (&st.doc, st.selected) {
        (Some(doc), Some(id)) if (id as usize) < doc.nodes.len() => {
            app.set_values(model(build_values(doc, id, &st.edits)));
            app.set_node_path(doc.node_path(id).into());
            let node = doc.node(id);
            app.set_node_doc(
                format!(
                    "{:?} v{} · offset 0x{:x}..0x{:x} · {} children · {} values",
                    node.kind,
                    node.version,
                    node.offset,
                    node.offset_end,
                    doc.child_count(id),
                    doc.node_values(id).count(),
                )
                .into(),
            );
        }
        _ => {
            app.set_values(model(Vec::<ValueRow>::new()));
            app.set_node_path(SharedString::new());
            app.set_node_doc(SharedString::new());
        }
    }
}

fn refresh_status(app: &AppWindow, st: &State) {
    let n = st.edits.len();
    app.set_pending_count(n as i32);
    app.set_save_label(if n == 0 {
        "Save".into()
    } else {
        format!("Save ({n})").into()
    });
    app.set_has_doc(st.doc.is_some());
    if let Some(doc) = &st.doc {
        app.set_doc_stats(
            format!(
                "{:?} · {:.1} MB · {} nodes · {} values",
                doc.header.magic,
                doc.data.len() as f64 / 1_000_000.0,
                fmt_int(doc.nodes.len() as i64),
                fmt_int(doc.values.len() as i64),
            )
            .into(),
        );
    } else {
        app.set_doc_stats(SharedString::new());
    }
}

fn refresh_all(app: &AppWindow, st: &State) {
    if let Some(doc) = &st.doc {
        app.set_factions(model(build_factions(doc)));
    }
    refresh_tree(app, st);
    refresh_values(app, st);
    refresh_status(app, st);
}

/// Load a save into `state` and repaint everything.
fn load_into(app: &AppWindow, state: &Rc<RefCell<State>>, path: &str) {
    app.set_status_text(format!("Loading {path}…").into());
    match esf_parser::parser::load_file(path) {
        Ok(doc) => {
            let root = doc.root;
            // Land on a value-rich node so the grid isn't empty on open.
            let mut expanded = HashSet::from([root]);
            let mut selected = None;
            if let Some(target) = find_first_node(&doc, "CAMPAIGN_SETUP_OPTIONS") {
                let mut cur = target;
                loop {
                    let parent = doc.node(cur).parent;
                    if parent == NO_PARENT {
                        break;
                    }
                    expanded.insert(parent);
                    cur = parent;
                }
                selected = Some(target);
            }
            {
                let mut st = state.borrow_mut();
                st.doc = Some(Arc::new(doc));
                st.path = path.to_string();
                st.edits.clear();
                st.expanded = expanded;
                st.selected = selected;
            }
            let st = state.borrow();
            refresh_all(app, &st);
            app.set_status_text(format!("Opened {path}").into());
        }
        Err(e) => {
            app.set_status_text(format!("Failed to open {path}: {e}").into());
            let _ = rfd::MessageDialog::new()
                .set_title("twedit — Load Error")
                .set_level(rfd::MessageLevel::Error)
                .set_description(format!("Failed to open {path}:\n\n{e}"))
                .show();
        }
    }
}

fn main() -> Result<(), slint::PlatformError> {
    let app = AppWindow::new()?;
    let state: Rc<RefCell<State>> = Rc::new(RefCell::new(State::default()));

    // --- Window chrome (frameless): native drag + close ---
    let weak = app.as_weak();
    app.on_start_drag(move || {
        if let Some(app) = weak.upgrade() {
            app.window().with_winit_window(|w| {
                let _ = w.drag_window();
            });
        }
    });
    let weak = app.as_weak();
    app.on_close_clicked(move || {
        if let Some(app) = weak.upgrade() {
            let _ = app.window().hide();
        }
    });

    // --- Open ---
    let weak = app.as_weak();
    let st_open = state.clone();
    app.on_open(move || {
        let Some(app) = weak.upgrade() else { return };
        let picked = rfd::FileDialog::new()
            .add_filter(
                "Total War saves / ESF",
                &["esf", "empire_save", "empire_save_multiplayer"],
            )
            .add_filter("All files", &["*"])
            .pick_file();
        if let Some(path) = picked {
            load_into(&app, &st_open, &path.display().to_string());
        }
    });

    // --- Select a tree node ---
    let weak = app.as_weak();
    let st_sel = state.clone();
    app.on_select_node(move |id| {
        let Some(app) = weak.upgrade() else { return };
        {
            let mut st = st_sel.borrow_mut();
            st.selected = Some(id as NodeId);
        }
        let st = st_sel.borrow();
        refresh_values(&app, &st);
        refresh_tree(&app, &st);
    });

    // --- Expand / collapse ---
    let weak = app.as_weak();
    let st_exp = state.clone();
    app.on_toggle_expand(move |id| {
        let Some(app) = weak.upgrade() else { return };
        {
            let mut st = st_exp.borrow_mut();
            let id = id as NodeId;
            if !st.expanded.insert(id) {
                st.expanded.remove(&id);
            }
        }
        let st = st_exp.borrow();
        refresh_tree(&app, &st);
    });

    // --- Toggle View / Edit (rebuild values so staged edits show) ---
    let weak = app.as_weak();
    let st_edit = state.clone();
    app.on_toggle_edit(move |on| {
        let Some(app) = weak.upgrade() else { return };
        app.set_edit_mode(on);
        let st = st_edit.borrow();
        refresh_values(&app, &st);
    });

    // --- Stage / unstage a value edit ---
    let weak = app.as_weak();
    let st_val = state.clone();
    app.on_edit_value(move |vid, text| {
        let Some(app) = weak.upgrade() else { return };
        let vid = vid as u32;
        {
            let mut st = st_val.borrow_mut();
            let Some(doc) = st.doc.clone() else { return };
            let Some(rec) = doc.values.get(vid as usize) else {
                return;
            };
            // Unparseable (a typing intermediate like "-"): leave staged
            // edits untouched rather than dropping the pending change.
            if let Some(edit) = rec.value.parse_edit(&text) {
                let unchanged = match &edit {
                    EsfEdit::Value(v) => *v == rec.value,
                    EsfEdit::Text(s) => {
                        Some(s.as_str()) == doc.decode_string(&rec.value).as_deref()
                    }
                };
                if unchanged {
                    st.edits.remove(&vid);
                } else {
                    st.edits.insert(vid, edit);
                }
            }
        }
        // Update pending count / Save label, but do NOT rebuild the values
        // model — that would reset the field the user is typing in.
        let st = st_val.borrow();
        refresh_status(&app, &st);
    });

    // --- Save (write, re-parse, reset staged edits) ---
    let weak = app.as_weak();
    let st_save = state.clone();
    app.on_save(move || {
        let Some(app) = weak.upgrade() else { return };
        let (doc, path, edits) = {
            let st = st_save.borrow();
            let Some(doc) = st.doc.clone() else { return };
            (doc, st.path.clone(), st.edits.clone())
        };
        if path.is_empty() {
            return;
        }
        app.set_status_text(format!("Saving {path}…").into());
        let (bytes, applied) = doc.bytes_with_edits(&edits);
        if let Err(e) = std::fs::write(&path, &bytes) {
            app.set_status_text(format!("Failed to save: {e}").into());
            return;
        }
        match esf_parser::parser::parse_bytes(bytes) {
            Ok(new_doc) => {
                {
                    let mut st = st_save.borrow_mut();
                    st.doc = Some(Arc::new(new_doc));
                    st.edits.clear();
                }
                let st = st_save.borrow();
                refresh_all(&app, &st);
                app.set_status_text(format!("Saved {applied} change(s) to {path}").into());
            }
            Err(e) => {
                app.set_status_text(format!("Saved, but failed to re-parse: {e}").into());
            }
        }
    });

    // --- Initial load ---
    if std::path::Path::new(DEFAULT_SAVE_PATH).exists() {
        load_into(&app, &state, DEFAULT_SAVE_PATH);
    } else {
        app.set_status_text("Use Open to load a save file.".into());
    }

    app.run()
}
