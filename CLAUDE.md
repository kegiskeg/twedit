# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this project is

`twedit` is a Rust reimplementation of the 2009 C# EsfEditor for Total War campaign saves (`.empire_save`, `.empire_save_multiplayer`, `startpos.esf`, etc.).

It supports editing **ABCD** and **ABCE** variants of the ESF format (Empire: Total War and Napoleon: Total War). ABCF (Shogun 2) and ABCA (Rome 2+) are recognized and rejected with a clear error.

**Key differences from the original EsfEditor:**
- Accurate type taxonomy per community specifications (taw/etwng, RPFM) rather than the flawed 2009 C# enum (angle type, typed arrays properly decoded).
- Variable-length string editing with automatic offset fixups (the C# editor's "SlowSave", done right).
- Fast native UI built on `windows-rs` via `windows-reactor` / `windows-core`.
- `schema_scan` tool for empirical field research against real save files.
- Semantic labels from `assets/esf_schema.toml` merged over the legacy XML.

## Read these before diving in

- `docs/FORMAT.md` — the canonical ESF ABCD/ABCE spec: every type tag with
  byte layout, record encodings, the offset-fixup rules for editing, and the
  workflow for extending field documentation. If you touch the parser or the
  editing path, read this first.
- `docs/scan_report.md` — generated per-node field statistics from a real
  104 MB campaign save (regenerate with `schema_scan`). This is the
  empirical reference when labeling fields or checking a hypothesis about
  what a value means.
- `twedit-ui/assets/esf_schema.toml` — the curated node docs / field labels
  and the comment at the top explaining its two addressing modes.

## Workspace layout

```text
esf-parser/                      # Core ESF parsing/editing engine (no UI deps)
  src/
    enums.rs                     # Magic headers + type tags, doc'd per spec
    objects.rs                   # EsfDocument arena, EsfValue, EsfEdit, editing
    parser.rs                    # Iterative frame-stack parser + tests
    pack_parser.rs               # Locates ETW via Steam library folders, reads
                                 #   local_en.pack (PFH0) → localisation.loc →
                                 #   key→English map (faction/region names);
                                 #   returns None gracefully without Steam/ETW
    bin/schema_scan.rs           # Field-statistics research tool
    bin/debug_parser.rs          # Quick tree dumper
    bin/esf_diff.rs              # Semantic save diff (offset-insensitive);
                                 #   prints node paths of changed values

twedit-ui/                       # Windows-native UI crate
  src/
    main.rs                      # App shell: Explorer/Factions/Regions views,
                                 #   tree, value grid, pending-edits drawer
    campaign.rs                  # Semantic extraction (factions, regions)
                                 #   from the generic tree; field positions
                                 #   follow the schema TOML + scan report
    descriptions.rs              # Merges legacy XML + esf_schema.toml labels
    theme.rs                     # "Imperial ledger" theme (umber/parchment/
                                 #   gold) via XAML overrides
  assets/
    esf_schema.toml              # Curated node docs + field labels (edit this)
    NodesDescriptions.xml        # Legacy 2009 descriptions (26/678 populated)
  wix/                           # MSI installer definition (cargo-wix)

docs/                            # Format spec and research (see above)
```

## Build & run

**Hard prerequisite:** `twedit-ui` depends on the user's **patched local
windows-rs clone** via path deps (`../../_lib/windows-rs/crates/libs/reactor`
etc.). The repo does not build without it. The patches (theme resources via
`App::theme_resources`, `set_window_icon`) must be reapplied if that clone is
ever re-cloned or hard-reset — the patch inventory is at the bottom of this
file.

```sh
cargo run -p twedit-ui             # run the editor (Windows only)
cargo check --workspace            # typecheck
cargo test --workspace             # all tests (parser + UI schema tests)
cargo clippy --workspace           # pedantic lints are warnings, not errors

./build_release.ps1                # portable zip + MSI (add -SkipMsi to skip)
cargo wix -p twedit-ui             # MSI only; works from any CWD because
                                   #   main.wxs uses $(sys.SOURCEFILEDIR)
```

The UI auto-loads `DEFAULT_SAVE_PATH` (a test save under
`C:\Projects\Rust\_old\esfeditor\saves\`) when present; the parser test
`parses_real_save_if_present` uses the same file and self-skips when absent.
The original C# source lives at `C:\Projects\Rust\_old\esfeditor\` for
reference — but its type enum is wrong in places; `docs/FORMAT.md` and the
community specs win.

### Running the UI from automation / agents

`cargo run -p twedit-ui` blocks until the window is closed — never run it
in a foreground shell from an automated flow. The smoke-test pattern:

```sh
cargo build --release -p twedit-ui
./target/release/twedit-ui.exe &   # background
sleep 12                           # long enough to parse the test save
kill -0 <pid> && echo alive        # still running = theme parsed, load OK
kill <pid>
```

Staying alive past load is the pass signal: theme XAML and control
templates are parsed at startup, so a bad `theme.rs` kills the process
before the window ever paints. For visual changes, prefer verifying with
real screenshots over guessing from code — several past bugs (dead flyout
handlers, mis-styled controls, a stale drawer ghost) were only visible live.

**Focus-free driving (works even while a game owns the foreground):**
computer-use's permission resolver cannot see this unpackaged dev exe, and
stealing focus/cursor is hostile if the user is mid-game. The proven
pattern instead: capture with `PrintWindow(hwnd, dc, 2)` (flag 2 =
PW_RENDERFULLCONTENT, required for WinUI composition surfaces), and drive
controls via **UI Automation** (`System.Windows.Automation` from
powershell.exe): InvokePattern for buttons (our custom Button template
exposes its content text as the UIA Name), TogglePattern for the Edit/View
switch, ValuePattern.SetValue for text boxes — SetValue fires TextChanged,
so edit staging is fully testable headlessly. Neither API moves focus or
the cursor.

## Architecture

### esf-parser

- **Flat arena, not a pointer tree.** `EsfDocument { data, nodes, items,
  values }`: `NodeId` is a u32 index, nodes are DFS pre-order (so sorted by
  file offset — `find_node_by_offset` binary-searches), and each node's
  children+values are a contiguous range in `items`. The whole file stays in
  memory (`data`); strings and arrays are **ranges into it**, decoded on
  demand — a 100 MB save parses in ~350 ms and does not duplicate payloads
  on the heap. Keep new value kinds lazy the same way.
- **Parsing is iterative** (explicit frame stack in `parse_tree`) so deep
  files can't overflow the thread stack. Poly (0x81) entries become
  `NodeKind::Record` nodes that inherit the poly's name/version.
- **Editing model:** edits are staged as `HashMap<value_id, EsfEdit>`.
  `EsfEdit::Value` = fixed-size in-place payload patch. `EsfEdit::Text` =
  string splice that rebuilds the file and fixes up every stored absolute
  offset. Arrays and opaque blocks are read-only today.

### twedit-ui

- `DocState` holds `Arc<EsfDocument>` with pointer-identity equality so
  state comparisons never deep-compare a 100 MB document.
- `descriptions::load(xml, toml)` builds the merged `Descriptions`; the
  value grid resolves labels via `label(name, &classes, pos)` where
  `classes` are per-value type classes (`descriptions::type_class`) —
  this implements the TOML's nth-occurrence-of-type addressing.
- Node IDs travel inside tree labels as invisible characters (U+2063 marker
  + U+200B/U+200C bits) because the tree widget only reports clicked label
  text (`encode_node_id` / `label_node_id` in `main.rs`).

## Conventions and gotchas

### Failure modes: strict parser, tolerant UI

When something is malformed, unknown, or unsupported, pick the failure
mode by layer:

- **Parser: hard `EsfError` with the absolute offset, never a skip.** An
  unknown type byte or inconsistent offset means either a real format
  discovery (research gold — see the 0x00/0x6D/0x8C note below) or a bug
  in our byte math that would corrupt every edit after it. Skipping bytes
  to "keep going" hides both. Errors, not panics: the `rejects_bad_input`
  tests assert truncated/garbage input returns `Err` cleanly.
- **Save path: refuse to write rather than write something broken.** The
  save flow re-parses the bytes it just produced; a re-parse failure is
  surfaced in the status bar instead of silently leaving a corrupt file.
- **UI metadata: degrade silently.** Missing field labels, descriptions,
  or schema entries fall back to raw names/values — an unlabeled field is
  normal (the schema is forever incomplete), not an error condition.

### Format / parser

- **The offset-fixup invariant (load-bearing):** any value kind whose file
  encoding stores an absolute offset (arrays, sized blocks) MUST be included
  in the fixup loop of `EsfDocument::bytes_with_edits`, or string edits will
  corrupt every such value after the splice. If you add one, extend the
  `string_edits_rewrite_and_fix_offsets` test — it deliberately places an
  array after two spliced strings.
- **Do not treat 0x00, 0x6D, or 0x8C as real ESF types.** They are C#
  EsfEditor artifacts (kept as `LegacyShort00` / `Unknown6D` /
  `SizedBlock8C` for compatibility) with zero observed occurrences across
  10M values. If `schema_scan` ever reports one, that file is research gold
  — don't paper over it.
- **Verification bar for parser/editing changes:** `cargo test --workspace`
  plus, for anything touching byte layout or fixups, a grow-then-shrink
  string-edit round trip on a real save must stay **byte-identical**. The
  synthetic sample in `parser.rs` tests (`build_sample`) should gain any new
  value type you add.
- All integers little-endian; all offsets absolute u32 positions.

### Schema / labels workflow

To label more fields (the main ongoing research task):

1. `cargo run --release -p esf-parser --bin schema_scan -- <save> out.md`
2. Pick an undocumented node; ranges/samples usually give the meaning away
   (faction/region IDs are huge u32s, turn counters cap at the current
   turn, keys match db table entries).
3. For a specific hypothesis, use the diff technique: change exactly one
   thing in-game (spend money, move an army), save again, then
   `cargo run --release -p esf-parser --bin esf_diff -- before after` —
   the changed paths name the field.
4. Cross-reference etwng's semantic converter — a local copy is vendored
   at `docs/esf_semantic_converter.rb` (origin: github.com/taw/etwng,
   `esfxml/lib/`) — ~150 node types annotated. Its `annotate_rec_nth`
   keys (`[:s, 0]`) map directly to `typed` keys (`s0`); its
   `annotate_rec` keys use **member indices that count child nodes**,
   so verify against scan samples before transcribing.
5. Add to `twedit-ui/assets/esf_schema.toml`; extend
   `real_assets_parse_and_resolve_diplomacy` if the node matters.

### windows-reactor UI gotchas

**The reactor source IS the documentation.** `windows-reactor` has no
published docs; the API reference is the crate source at
`C:\Projects\Rust\_lib\windows-rs\crates\libs\reactor\src\`:

- `widgets/*.rs` — every builder and its methods (check here before
  assuming a widget supports something; e.g. `tree_view` has no
  on_expanding, `text_block` has no wrapping).
- `element.rs` (`ElementExt`) + `style.rs` — modifiers, `ThemeRef`
  resource keys, animation configs, `PointerEventInfo`.
- `generated.rs` + `backend/winui/generated_attach_event.rs` +
  `backend/winui/mod.rs` — whether a callback is *actually wired* and
  when it re-attaches. Verify here before building on an event; two past
  bugs (stale flyout handlers, doubts about ToggleSwitch) were resolved
  by reading the attach code, not the widget API.

Known sharp edges:

- **Flyout/menu handlers are wired ONCE, at first mount.** If
  `MenuFlyoutItems` never change, `on_item_clicked` closures keep their
  first-render captures forever (stale doc snapshots, silent no-ops). The
  established fix: flyout callbacks only write into pending state
  (`set_pending_goto` / `set_pending_open`), and render drains the pending
  value and acts. Apply this pattern to ANY flyout/menu callback. Plain
  Button `Click` handlers re-attach every render and are safe.
- **`text_block` has no `text_wrapping`** in this reactor version (only
  `text_box` does) — long text must be truncated (see the node doc line,
  capped at 220 chars).
- **SelectorBar and AutoSuggestBox callbacks are NOT wired** (verified in
  `backend/winui/generated_attach_event.rs`): `on_selection_changed` /
  `on_query_submitted` exist on the builders but are never attached —
  silent no-ops. The view-switcher tabs are custom Buttons for this
  reason. BreadcrumbBar `ItemClicked` (payload = index) and NumberBox
  `ValueChanged` ARE wired.
- **Keep grid row structures stable across renders.** The reconciler
  diffs children positionally; conditionally removing a middle row (the
  pending-edits drawer) shifted every later sibling and left ghost
  visuals of the removed row. Render a zero-height placeholder in the
  slot instead of dropping the row (see `drawer_slot` in `main.rs`).
- `Thickness` only has `From<f64>`; use `Thickness::xy` or a literal.
- `ListBox` doesn't impl `ElementExt` — wrap in `Element::from` first.
- Bad theme XAML fails at app start, not compile time — smoke-launch after
  editing `theme.rs`.
- Setter types are `AsyncSetState<T>` / `SetState<T>`; controlled
  `text_box` works because the reconciler only pushes `Text` on def change.

### Licensing

GPL-3.0-or-later, matching the original 2009 EsfEditor. Crate metadata, the
LICENSE file, and the MSI's `License.rtf` must stay in agreement.

## When you change something

- **Add a value type:** variant in `enums.rs` (doc comment: byte layout +
  source citation) → `EsfValue` variant + `format_value` /
  `value_type_name` / `payload_bytes` / `parse_same_type` in `objects.rs` →
  parse arm in `parser.rs` → fixup loop if it stores an offset → extend
  `build_sample` + assertions → document in `docs/FORMAT.md` → add its
  class to `descriptions::type_class` if it should be label-addressable.
- **Add field labels:** follow the schema workflow above; no code changes
  needed unless a new type class is involved.
- **Change editing/save behavior:** keep the QuickSave fast path (no
  splices → in-place only); re-verify the byte-identical round trip.
- **Touch the reactor patch:** files are
  `crates/libs/reactor/src/app.rs` (theme_xaml field + builder + install in
  run/run_custom), `app_shim.rs` (`install_theme_resources`), and
  `bindings.rs`/`host.rs` (IAppWindow SetIcon + `set_window_icon` with
  PENDING_ICON applied post-render). Reapply after any re-clone/pull of
  `C:\Projects\Rust\_lib\windows-rs`.
- **Log the why, not just the what.** Format/byte-layout discoveries go
  in `docs/FORMAT.md` (with a source citation or scan evidence); field
  semantics go in `esf_schema.toml` next to the label they justify; new
  reactor workarounds and sharp edges go in this file's gotchas section.
  A workaround whose reason isn't written down gets "cleaned up" and
  reintroduces the bug.
