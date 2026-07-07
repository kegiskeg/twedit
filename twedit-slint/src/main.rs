#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Slint front-end for twedit. Shares the `esf-parser` engine (parsing,
//! editing, and `campaign` semantic extraction) with the WinUI build; only
//! the view layer differs. This first cut proves the pipeline: parse a real
//! save, feed a virtualized factions table and an Explorer tree, all in the
//! imperial-ledger theme with no dependency on WinUI or a patched windows-rs.

slint::include_modules!();

use esf_parser::campaign::extract_factions;
use esf_parser::objects::EsfDocument;
use i_slint_backend_winit::WinitWindowAccessor;
use slint::{Color, ModelRc, VecModel};
use std::rc::Rc;

const DEFAULT_SAVE_PATH: &str =
    r"C:\Projects\Rust\_old\esfeditor\saves\test_save.empire_save_multiplayer";

/// Fallback swatch for factions with no flag colour (panel umber).
const NO_SWATCH: Color = Color::from_rgb_u8(0x1A, 0x17, 0x11);

/// Build the factions table model from the shared semantic extraction.
fn faction_rows(doc: &EsfDocument) -> Vec<FactionRow> {
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
            treasury: f.treasury.map(|(_, amount)| amount).unwrap_or(0),
            swatch: f
                .color
                .map(|(r, g, b)| Color::from_rgb_u8(r, g, b))
                .unwrap_or(NO_SWATCH),
            major: f.is_major,
            destroyed: f.destroyed,
        })
        .collect()
}

/// Flatten the top of the arena (root + two levels) into tree rows. A future
/// pass will window this for the full 2M-node tree, since Slint's ListView
/// virtualizes but its scroll math gets shaky past a few million rows.
fn tree_rows(doc: &EsfDocument) -> Vec<TreeRow> {
    let mut out = Vec::new();
    let root = doc.root;
    out.push(TreeRow {
        label: doc.node_name(root).into(),
        depth: 0,
        node_id: root as i32,
    });
    for child in doc.children(root) {
        out.push(TreeRow {
            label: doc.node_name(child).into(),
            depth: 1,
            node_id: child as i32,
        });
        for grandchild in doc.children(child) {
            out.push(TreeRow {
                label: doc.node_name(grandchild).into(),
                depth: 2,
                node_id: grandchild as i32,
            });
        }
    }
    out
}

fn main() -> Result<(), slint::PlatformError> {
    let app = AppWindow::new()?;

    // Custom-title-bar plumbing: the window is `no-frame`, so dragging and
    // closing are ours. Dragging goes through the winit accessor's native
    // `drag_window` (keeps OS aero-snap); minimize/maximize are handled in
    // the .slint via the Window state props.
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

    if std::path::Path::new(DEFAULT_SAVE_PATH).exists() {
        match esf_parser::parser::load_file(DEFAULT_SAVE_PATH) {
            Ok(doc) => {
                let facs = faction_rows(&doc);
                let tree = tree_rows(&doc);
                app.set_doc_stats(
                    format!(
                        "{:?} · {:.1} MB · {} factions · {} nodes",
                        doc.header.magic,
                        doc.data.len() as f64 / 1_000_000.0,
                        facs.len(),
                        doc.nodes.len(),
                    )
                    .into(),
                );
                app.set_factions(ModelRc::from(Rc::new(VecModel::from(facs))));
                app.set_tree_rows(ModelRc::from(Rc::new(VecModel::from(tree))));
                app.set_status_text(format!("Opened {DEFAULT_SAVE_PATH}").into());
            }
            Err(e) => app.set_status_text(format!("Failed to open: {e}").into()),
        }
    } else {
        app.set_status_text("Use Open to load a save file.".into());
    }

    app.run()
}
