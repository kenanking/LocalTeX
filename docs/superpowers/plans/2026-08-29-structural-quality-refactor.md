# Structural Quality Refactor Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove duplicated document-kind heuristics and closed-set exporter boilerplate, then split the two god objects (`AppState`, `MainWindow`) so later features do not grow spaghetti — without changing user-visible snip / OCR / copy / settings behavior.

**Architecture:** Keep one crate and one GPUI `Entity<AppState>`. Canonicalize `BlockKind` at OCR and SQLite load so consumers trust `block.kind`. Replace the `Exporter` trait with methods on the existing `CopyKind` enum. Nested data types (`CaptureSession`, `IngestPump`, `SearchFilter`) plus a `src/state/` module own orchestration; a `SettingsPane` entity owns settings/sysmon so `Render` only paints prepared state.

**Tech Stack:** Rust 2021, crates.io `gpui 0.2`, rusqlite, existing `cargo test` / `cargo fmt` / `cargo clippy --profile dev-opt --all-targets` / `./scripts/run-on-desktop.sh` (Linux X11 only).

## Global Constraints

- Single Cargo package, single process. No workspace, no extra crates.
- Smallest coherent change per task. No OCR pipeline replacement; `to_doc_block` kind promotion is allowed.
- User-visible copy, preview, snip overlay, and settings behavior stay the same unless a task explicitly documents a tradeoff.
- `export_fmt` (session) vs `prefs.default_fmt` (persisted) stay two fields: View-menu toggle does **not** persist; Settings writes both. Add a comment, do not merge.
- Linux X11 is the verifiable desktop path. Do not claim Windows or macOS runtime verification.
- Identifiers, comments, and commit messages in English. Do not commit unless the user asked to execute this plan (each task still has a commit step for when they do).
- After every task: `cargo fmt --all -- --check`, `cargo test`, `cargo clippy --profile dev-opt --all-targets`. Clippy must not introduce new warnings.
- GPUI: `window, cx` parameter order; `cx.notify()` after render-affecting state; OCR/disk/image in `cx.background_spawn`; do not update an entity already being updated.

## Stop points

| After task | Tree is | Safe to ship |
|---|---|---|
| 1–2 | Kind is canonical; search/math helpers cleaned | Yes |
| 3 | Copy chips still work, no `Exporter` trait | Yes |
| 4–5 | Files smaller, same behavior | Yes |
| 6–8 | `AppState` split, Library encapsulated | Yes, plus one Linux snip smoke |
| 9–10 | Settings entity + Render no longer loads | Yes, plus settings + preview smoke |

Do not start Task N+1 until Task N tests are green. Do not combine Tasks 7 and 9 in one diff.

## File structure (end state)

```
src/doc.rs                 Block::promote_html_table; CopyKind::{ALL,id,applies_to}; CopyRow { kind, text }
src/math.rs                pub(crate) split_braced
src/office.rs              uses math::split_braced; while-let unwrap_boxed
src/export.rs              impl CopyKind { render }; no Exporter trait
src/ocr/pipeline.rs        to_doc_block calls promote_html_table
src/preview/mod.rs         types, re-exports (was src/preview.rs)
src/preview/table_layout.rs
src/library.rs             private fields; select / set_visible / set_date_preset
src/state/mod.rs           AppState, new(prefs), prefs, desktop pump, bind_keys
src/state/session.rs       CaptureSession, IngestPump, SearchFilter + unit tests
src/state/capture.rs       impl AppState capture / hide / paste
src/state/ingest.rs        impl AppState ingest / OCR / persist / thumbs / png
src/state/search.rs        impl AppState filter
src/ui/settings/mod.rs     SettingsPane + page chrome
src/ui/settings/{general,formatting,shortcuts,system}.rs
src/ui/main_window.rs      no sysmon/settings fields; Render does not schedule loads
src/ui/preview_doc.rs      TextRunSpec instead of 8-arg helper
src/main.rs                Prefs::load once; AppState::new(prefs)
```

Unchanged on purpose: `ocr/{layout,unirec,text,imgops}.rs` internals, `desktop/**` overlays, `capture.rs`, `store.rs` schema.

---

### Task 1: Canonicalize `BlockKind` at the trust boundary

**Files:**
- Modify: `src/doc.rs` (`Block`, `decode_blocks_json`, `snip_kind`, tests)
- Modify: `src/ocr/pipeline.rs` (`to_doc_block`)
- Modify: `src/export.rs` (delete `effective_kind`; use `block.kind`)
- Modify: `src/office.rs` (delete local `effective_kind`)
- Modify: `src/preview.rs` (`document_preview_with_dpr` table branch)
- Keep: `src/table.rs` `looks_like_html_table` as the single detector used by `promote_html_table`

**Interfaces:**
- Consumes: `table::looks_like_html_table(&str) -> bool` (unchanged)
- Produces:

```rust
impl Block {
    /// If this is not already a table but the payload is HTML `<table`, set kind to Table.
    pub fn promote_html_table(&mut self) {
        if self.kind != BlockKind::Table && table::looks_like_html_table(&self.text) {
            self.kind = BlockKind::Table;
        }
    }
}

pub fn decode_blocks_json(json: &str) -> Result<Vec<Block>, serde_json::Error> {
    // existing serde, then:
    // for b in &mut blocks { b.promote_html_table(); }
    // Ok(blocks)
}
```

`to_doc_block` must call `block.promote_html_table()` before `Some(block)`.

After this task, `grep looks_like_html_table src` must only hit `table.rs` and `doc.rs`.

- [ ] **Step 1: Write the failing tests** in `src/doc.rs` `mod tests` (reuse existing `rect` helper):

```rust
#[test]
fn html_payload_in_text_block_promotes_to_table() {
    let mut b = Block::new(
        BlockKind::Text,
        rect(0),
        "<table><tr><td>a</td></tr></table>",
    );
    b.promote_html_table();
    assert_eq!(b.kind, BlockKind::Table);
}

#[test]
fn decode_promotes_legacy_html_text_rows() {
    let json = r#"[{"kind":"Text","bbox":{"x":0,"y":0,"w":1,"h":1},"text":"<table><tr><td>a</td></tr></table>"}]"#;
    let blocks = decode_blocks_json(json).expect("json");
    assert_eq!(blocks[0].kind, BlockKind::Table);
    assert_eq!(snip_kind(&blocks), SnipKind::Table);
}
```

In `src/ocr/mod.rs` tests, add:

```rust
#[test]
fn text_region_with_html_table_becomes_table_kind() {
    let block = to_doc_block("text", [0.0, 0.0, 10.0, 10.0], "<table><tr><td>a</td></tr></table>")
        .expect("block");
    assert_eq!(block.kind, BlockKind::Table);
}
```

(`to_doc_block` is already imported via `pipeline::{..., to_doc_block}` in that test module.)

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test html_payload_in_text_block_promotes_to_table decode_promotes_legacy_html_text_rows text_region_with_html_table_becomes_table_kind
```

Expected: compile error (`promote_html_table` missing) or FAIL.

- [ ] **Step 3: Implement promotion and delete consumer heuristics**

`Block::promote_html_table` as above. Call it from `decode_blocks_json` after deserialize (all three JSON shapes). Call it at the end of `to_doc_block` before `Some(block)`.

`snip_kind`: `BlockKind::Text` arm must **not** call `looks_like_html_table`. Count tables only via `BlockKind::Table`. Captions still skip `n_text` via `role.interrupts_prose()`.

`export.rs`:
- Delete `fn effective_kind`.
- `export_block` / `row_is_spaced` / `table_html` / `group_rows` / `render_latex_table_snip` match on `block.kind` only.
- `group_rows` table/formula solo-row condition: `matches!(block.kind, BlockKind::Table | BlockKind::Formula) && (block.kind == BlockKind::Table || block.text.contains('\n') || block.text.len() > 48)`.
- Delete the `looks_like_html_table` branch inside `emit_text_block`. After promotion, a Text block is not an HTML table. Mixed `$` splitting stays.

`office.rs`: delete `effective_kind`; `document_xml` matches `block.kind`.

`preview.rs` `document_preview_with_dpr`: delete the `BlockKind::Text` + `looks_like_html_table` branch. HTML tables arrive as `BlockKind::Table` and already hit `push_table_block`.

- [ ] **Step 4: Run tests**

```bash
cargo test
rg -n 'looks_like_html_table' src
```

Expected: all tests PASS. `looks_like_html_table` only in `src/table.rs` and `src/doc.rs`. Existing tests `table_plus_caption_is_table_snip`, `recs_map_formula_and_skip_empty`, office/export markdown heading tests still pass.

- [ ] **Step 5: Commit**

```bash
git add src/doc.rs src/ocr/pipeline.rs src/ocr/mod.rs src/export.rs src/office.rs src/preview.rs
git commit -m "$(cat <<'EOF'
Promote HTML tables to BlockKind at ingest and load.

Consumers can trust block.kind instead of re-detecting <table> in export, office, preview, and snip_kind.
EOF
)"
```

---

### Task 2: Dedup helpers, search blob, leftover aliases, clippy nits

**Files:**
- Modify: `src/math.rs` (export `split_braced`)
- Modify: `src/office.rs` (use it; `while let` in `unwrap_boxed`)
- Modify: `src/doc.rs` (`search_text_for_blocks`)
- Modify: `src/ocr/mod.rs` (`default_intra`)
- Modify: `src/ocr/pipeline.rs` (`env_flag`)
- Modify: `src/store.rs` (`SNIPS_COLUMNS.contains`)

**Interfaces:**
- Consumes: Task 1 `BlockKind` canonical
- Produces:

```rust
// math.rs
pub(crate) fn split_braced(s: &str) -> Option<(String, &str)> { /* move existing body */ }

// doc.rs
pub fn search_text_for_blocks(blocks: &[Block]) -> String {
    let mut out = String::new();
    for b in blocks {
        out.push_str(&b.text);
        out.push('\n');
    }
    out
}
```

Tradeoff (documented here, not a product prompt): FTS no longer indexes Markdown wrapped with **default** delimiters. Raw `block.text` (including TeX) remains. Store tests that query `frac` / `\frac` must still pass.

- [ ] **Step 1: Write the failing test** in `src/math.rs` tests:

```rust
#[test]
fn split_braced_finds_matching_close() {
    let (inner, rest) = super::split_braced("foo} bar").expect("close");
    assert_eq!(inner, "foo");
    assert_eq!(rest, " bar");
    assert!(super::split_braced("no close").is_none());
}
```

In `src/doc.rs` tests:

```rust
#[test]
fn search_blob_is_raw_block_text() {
    let blocks = vec![Block::new(BlockKind::Formula, rect(0), r"\frac{1}{2}")];
    let blob = Document::search_text_for_blocks(&blocks);
    assert!(blob.contains(r"\frac{1}{2}"));
    assert!(!blob.contains("$$"), "must not depend on Prefs::default wrap");
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test split_braced_finds_matching_close search_blob_is_raw_block_text
```

Expected: FAIL or private `split_braced`; `search_blob` FAIL while `$$` still present.

- [ ] **Step 3: Implement**

1. `pub(crate) fn split_braced` in `math.rs`. Delete the copy in `office.rs`. `unwrap_boxed` becomes:

```rust
fn unwrap_boxed(tex: &str) -> String {
    let mut t = tex.to_string();
    let needle = r"\boxed{";
    while let Some(at) = t.find(needle) {
        let rest = &t[at + needle.len()..];
        let Some((inner, after)) = crate::math::split_braced(rest) else {
            break;
        };
        t = format!("{}{}{}", &t[..at], inner, after);
    }
    t
}
```

2. Strip the `export_blocks(..., Prefs::default())` half of `search_text_for_blocks`.
3. `default_intra`: only `LOCALTEX_INTRA_THREADS`. Update the comment. `env_flag("LOCALTEX_SPINNING", "OPENDOC_SPINNING")` → read only `LOCALTEX_SPINNING` (keep a one-arg helper or inline `std::env::var("LOCALTEX_SPINNING").is_ok_and(|v| v != "0")`).
4. `src/store.rs` around line 413: `SNIPS_COLUMNS.contains(&n.as_str())` per clippy.

- [ ] **Step 4: Run tests**

```bash
cargo test
cargo clippy --profile dev-opt --all-targets --message-format=short
```

Expected: PASS. No `office.rs` while-let warning. No `store.rs` contains warning. `OPENDOC_` gone from `src`.

- [ ] **Step 5: Commit**

```bash
git add src/math.rs src/office.rs src/doc.rs src/ocr/mod.rs src/ocr/pipeline.rs src/store.rs
git commit -m "$(cat <<'EOF'
Share split_braced and stop indexing default Markdown in FTS.

Search blobs are raw block text; leftover OPENDOC_ env aliases are removed.
EOF
)"
```

---

### Task 3: Collapse `Exporter` into `CopyKind`

**Files:**
- Modify: `src/doc.rs` (`CopyKind`, `CopyRow`, tests that read `.label` / `.exporter_id`)
- Modify: `src/export.rs` (delete trait + 10 ZSTs; `impl CopyKind { render }`; `copy_rows`)
- Modify: `src/ui/detail.rs` (chip id / label from `CopyKind`)

**Interfaces:**
- Consumes: existing `formula_body`, `table_html`, `export_blocks`, `render_latex_table_snip`, `table_tsv` (stay private in `export.rs`)
- Produces:

```rust
// doc.rs — next to label()/symbol()
impl CopyKind {
    pub const ALL: [Self; 10] = [
        Self::MsWord,
        Self::Latex,
        Self::MdInline,
        Self::MdDisplay,
        Self::Equation,
        Self::LatexTable,
        Self::MdTable,
        Self::Tsv,
        Self::Markdown,
        Self::LatexDoc,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::MsWord => "ms_word",
            Self::Latex => "latex",
            Self::MdInline => "md_inline",
            Self::MdDisplay => "md_display",
            Self::Equation => "equation",
            Self::LatexTable => "latex_table",
            Self::MdTable => "md_table",
            Self::Tsv => "tsv",
            Self::Markdown => "markdown",
            Self::LatexDoc => "latex_doc",
        }
    }

    pub fn applies_to(self, kind: SnipKind) -> bool {
        match self {
            Self::MsWord | Self::Latex | Self::MdInline | Self::MdDisplay | Self::Equation => {
                kind == SnipKind::Formula
            }
            Self::LatexTable | Self::MdTable | Self::Tsv => kind == SnipKind::Table,
            Self::Markdown | Self::LatexDoc => kind == SnipKind::Mixed,
        }
    }
}

pub struct CopyRow {
    pub kind: CopyKind,
    pub text: String,
}
```

```rust
// export.rs
impl CopyKind {
    pub fn render(self, blocks: &[Block], prefs: &Prefs) -> String {
        match self {
            CopyKind::MsWord => crate::office::formula_mathml(&formula_body(blocks)),
            CopyKind::Latex => formula_body(blocks),
            CopyKind::MdInline => prefs.wrap_inline(&formula_body(blocks)),
            CopyKind::MdDisplay => {
                let body = formula_body(blocks);
                format!("$$ {body} $$")
            }
            CopyKind::Equation => {
                let body = formula_body(blocks);
                format!("\\begin{{equation}}\n{body}\n\\end{{equation}}")
            }
            CopyKind::LatexTable => render_latex_table_snip(blocks),
            CopyKind::MdTable => export_blocks(blocks, ExportFmt::Markdown, prefs),
            CopyKind::Tsv => table_tsv(blocks).unwrap_or_default(),
            CopyKind::Markdown => export_blocks(blocks, ExportFmt::Markdown, prefs),
            CopyKind::LatexDoc => export_blocks(blocks, ExportFmt::Latex, prefs),
        }
    }
}

pub fn copy_rows(blocks: &[Block], prefs: &Prefs) -> Vec<CopyRow> {
    let kind = crate::doc::snip_kind(blocks);
    match kind {
        SnipKind::Formula if formula_body(blocks).is_empty() => return Vec::new(),
        SnipKind::Table if table_html(blocks).is_none() => return Vec::new(),
        _ => {}
    }
    CopyKind::ALL
        .into_iter()
        .filter(|k| k.applies_to(kind))
        .filter_map(|k| {
            let text = k.render(blocks, prefs);
            if k == CopyKind::Tsv && text.is_empty() {
                None
            } else {
                Some(CopyRow { kind: k, text })
            }
        })
        .collect()
}
```

Delete `pub trait Exporter`, all ZST structs, `EXPORTERS`, `pub fn exporters()`. Module docs: `//! Copy / document export keyed by CopyKind.`

`detail.rs` chip:

```rust
copy_chip(
    SharedString::from(format!("copy-{}", kind.id())),
    kind.label(),
    kind.symbol(),
    hint,
    ...
)
```

Doc tests that used `rows[0].label` → `rows[0].kind.label()`.

- [ ] **Step 1: Write the failing test** replacing `exporters_have_unique_ids` in `src/export.rs`:

```rust
#[test]
fn copy_kinds_have_unique_ids_and_cover_all() {
    let mut ids = std::collections::HashSet::new();
    for kind in CopyKind::ALL {
        assert!(ids.insert(kind.id()), "duplicate {}", kind.id());
        assert!(!kind.label().is_empty());
        assert!(!kind.symbol().is_empty());
    }
    assert_eq!(CopyKind::ALL.len(), 10);
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test copy_kinds_have_unique_ids_and_cover_all
```

Expected: `ALL` / `id` missing.

- [ ] **Step 3: Implement** the types and `copy_rows` as above. Keep `visible_copy_rows` / `copy_payload_eq` unchanged. Chip order must stay `CopyKind::ALL` order (same as old `EXPORTERS`).

- [ ] **Step 4: Run tests**

```bash
cargo test
rg -n 'trait Exporter|fn exporters\(' src
```

Expected: PASS. No `Exporter` / `exporters(`. Existing `copy_rows` tests in `doc.rs` still match kinds and visibility.

- [ ] **Step 5: Commit**

```bash
git add src/doc.rs src/export.rs src/ui/detail.rs
git commit -m "$(cat <<'EOF'
Key copy formats on CopyKind instead of an Exporter trait.

The catalog is closed; chip ids and render dispatch live on the enum.
EOF
)"
```

---

### Task 4: Split preview table layout; shrink `text_run_el`

**Files:**
- Create: `src/preview/mod.rs` (move from `src/preview.rs`)
- Create: `src/preview/table_layout.rs`
- Delete: `src/preview.rs` (after move)
- Modify: `src/ui/preview_doc.rs` (`TextRunSpec`)
- Modify: `src/main.rs` — `mod preview;` stays valid with `preview/mod.rs`

**Interfaces:**
- Consumes: `segs_from_cell`, `InlineSeg`, `PreviewLayout`, `PlacedCell` (stay in `preview/mod.rs` or re-exported)
- Produces:

```rust
// preview/table_layout.rs
pub(crate) fn table_preview_layout(table: &crate::table::Table, dpr: f64) -> crate::preview::PreviewLayout
```

Move `table_preview_layout`, `estimate_text_width`, `looks_numeric` into `table_layout.rs`. `push_table_block` stays in assemble path (`mod.rs`) and calls `table_layout::table_preview_layout`.

```rust
// preview_doc.rs
struct TextRunSpec {
    run_id: String,
    block_id: String,
    tok: String,
    para_text: String,
    start: usize,
    paragraph: bool,
}

fn text_run_el(&self, spec: TextRunSpec, sel: Rc<RefCell<PreviewSel>>) -> impl IntoElement
```

Call sites pass `TextRunSpec { ... }`. This clears clippy `too_many_arguments` on `text_run_el`.

Do **not** unify `document_preview_with_dpr` and `export::group_rows`. They remain two assemblers (flow vs spatial rows).

- [ ] **Step 1: Move file with git** so history survives:

```bash
mkdir -p src/preview
git mv src/preview.rs src/preview/mod.rs
```

Then cut the three functions into `src/preview/table_layout.rs` with `mod table_layout;` in `mod.rs`. Fix imports (`use super::{...}`).

- [ ] **Step 2: Run existing preview tests** (no new behavior)

```bash
cargo test mixed_text_preview_has_paragraph title_and_body_are_separate_preview_blocks
```

Expected: PASS. Other tests live under `preview::tests`; `cargo test` in Step 4 covers them.

- [ ] **Step 3: Introduce `TextRunSpec`** and update every `text_run_el(` call in `preview_doc.rs`.

- [ ] **Step 4: Verify**

```bash
cargo test
cargo clippy --profile dev-opt --all-targets --message-format=short
wc -l src/preview/*.rs
```

Expected: no `too_many_arguments` for `text_run_el`. `mod.rs` well under 1000 lines; `table_layout.rs` holds the grid math.

- [ ] **Step 5: Commit**

```bash
git add src/preview src/ui/preview_doc.rs
git commit -m "$(cat <<'EOF'
Move preview table geometry into its own module.

RaTeX SVG assembly no longer shares a file with grid layout, and preview text runs take a struct instead of eight arguments.
EOF
)"
```

---

### Task 5: Split settings pages into a module directory

**Files:**
- Create: `src/ui/settings/mod.rs` (from `src/ui/settings.rs`: `SettingsTab`, `SettingsScroll`, `page`, `TABS`, `picker`, `bool_row`, `patch_prefs`)
- Create: `src/ui/settings/general.rs` (`general_page`)
- Create: `src/ui/settings/formatting.rs` (`formatting_page`)
- Create: `src/ui/settings/shortcuts.rs` (`shortcuts_page`, `shortcut_group`, `shortcut_row`)
- Create: `src/ui/settings/system.rs` (`system_page` and spark/stat helpers)
- Delete: `src/ui/settings.rs`
- Modify: `src/ui/mod.rs` — `mod settings;` already works with a directory

**Interfaces:**
- Consumes: same `page(...)` signature as today (still 8 args; Task 9 removes that)
- Produces: `pub use` of `page`, `SettingsTab`, `SettingsScroll` from `settings/mod.rs` so `main_window.rs` imports do not change.

`general_page` / `formatting_page` / etc. become `pub(super)`.

- [ ] **Step 1: `git mv src/ui/settings.rs src/ui/settings/mod.rs`**, then split functions into the four files. Keep `page` body identical (still calls `general_page(state, &prefs)` etc.).

- [ ] **Step 2: Compile**

```bash
cargo test
cargo clippy --profile dev-opt --all-targets --message-format=short
```

Expected: PASS. `page` may still warn `too_many_arguments` until Task 9.

- [ ] **Step 3: Commit**

```bash
git add src/ui/settings src/ui/settings.rs
git commit -m "$(cat <<'EOF'
Split settings tabs into per-page modules.

The 8-argument page() signature stays until SettingsPane owns tab and listen state.
EOF
)"
```

---

### Task 6: Encapsulate `Library` selection and visible ids

**Files:**
- Modify: `src/library.rs`
- Modify: `src/state.rs` (or `src/state/*` if Task 7 already landed — this task **precedes** Task 7)
- Test: `src/library.rs` `mod tests`

**Interfaces:**
- Consumes: current `HashMap` + `order` + `visible_ids` + `selected`
- Produces:

```rust
impl Library {
    pub fn selected(&self) -> Option<Uuid> { self.selected }
    pub fn visible_ids(&self) -> &[Uuid] { &self.visible_ids }
    pub fn date_preset(&self) -> DatePreset { self.date_preset }

    pub fn select(&mut self, id: Uuid) {
        if self.docs.contains_key(&id) {
            self.selected = Some(id);
            self.touch_lru(id);
        }
    }

    /// Replace the filtered id list. If the current selection is not visible,
    /// select the first visible id. Returns true when selection changed.
    pub fn set_visible(&mut self, ids: Vec<Uuid>) -> bool {
        self.visible_ids = ids;
        let gone = self.selected.is_none_or(|s| !self.visible_ids.contains(&s));
        if gone {
            self.selected = self.visible_ids.first().copied();
            true
        } else {
            false
        }
    }

    pub fn set_date_preset(&mut self, preset: DatePreset) -> bool {
        if self.date_preset == preset {
            return false;
        }
        self.date_preset = preset;
        true
    }
}
```

Fields `docs`, `order`, `visible_ids`, `selected`, `date_preset` become private. Keep `get` / `get_mut` / `insert_newest` / `remove` / `iter_all` / `visible_docs` / `touch_lru` / `pixels` / `gpu_full_ids`.

`AppState::selected` / `visible_ids` / `date_preset` forward to these methods. `AppState::select` calls `library.select`. Filter completion calls `library.set_visible(ids)` then `ensure_detail` if the returned `bool` is true.

Extract the inflight+persisted merge used in `schedule_filter` to a pure function in `library.rs` or `state.rs`:

```rust
pub fn merge_visible(inflight_hits: Vec<Uuid>, mut persisted: Vec<Uuid>) -> Vec<Uuid> {
    persisted.retain(|id| !inflight_hits.contains(id));
    let mut out = inflight_hits;
    out.append(&mut persisted);
    out
}
```

Use it in `schedule_filter` so Task 7 can test it without GPUI.

- [ ] **Step 1: Write failing tests** in `src/library.rs`:

```rust
#[test]
fn set_visible_moves_selection_when_filtered_out() {
    let mut lib = Library::new();
    let a = Document::pending(Arc::new(RgbaImage::new(4, 4)));
    let b = Document::pending(Arc::new(RgbaImage::new(4, 4)));
    let id_a = a.id;
    let id_b = b.id;
    lib.insert_newest(a);
    lib.insert_newest(b); // selected = b
    assert_eq!(lib.selected(), Some(id_b));
    let changed = lib.set_visible(vec![id_a]);
    assert!(changed);
    assert_eq!(lib.selected(), Some(id_a));
}

#[test]
fn merge_visible_puts_inflight_first_without_dup() {
    let a = Uuid::new_v4();
    let b = Uuid::new_v4();
    let c = Uuid::new_v4();
    let out = merge_visible(vec![a], vec![a, b, c]);
    assert_eq!(out, vec![a, b, c]);
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test set_visible_moves_selection_when_filtered_out merge_visible_puts_inflight_first_without_dup
```

- [ ] **Step 3: Implement methods, privatize fields, update `state.rs` and any `library.selected` / `library.visible_ids` writes.** UI must go through `AppState` accessors (already mostly does). `main_window.rs` `state.library.get` may stay if `get` remains public.

- [ ] **Step 4: Run tests**

```bash
cargo test
```

- [ ] **Step 5: Commit**

```bash
git add src/library.rs src/state.rs src/ui/main_window.rs
git commit -m "$(cat <<'EOF'
Keep Library selection and visible ids behind methods.

Filter results cannot poke HashMap fields; merge_visible is a pure helper for the search pump.
EOF
)"
```

---

### Task 7: Nested session types and `src/state/` module

**Files:**
- Create: `src/state/session.rs`
- Create: `src/state/mod.rs` (from current `src/state.rs` struct + `new` + prefs + desktop + key bind)
- Create: `src/state/capture.rs` (`request_capture`, hide/reveal, paste, upload)
- Create: `src/state/ingest.rs` (pixel ingest, OCR pump, persist, thumbs, png/blocks load, delete, retry, copy, docx)
- Create: `src/state/search.rs` (`schedule_filter`, `set_search_query`, `set_date_preset`)
- Delete: `src/state.rs`
- Modify: `src/main.rs` — `mod state;` still works

**Interfaces:**
- Consumes: Task 6 `Library` methods, `merge_visible`, existing `OcrQueue`
- Produces:

```rust
// state/session.rs
pub(crate) enum Capture {
    Idle,
    Grabbing,
    Failed(String),
}

pub(crate) struct CaptureSession {
    status: Capture,
    gen: u64,
    hide_depth: u32,
    reveal_on_main: bool,
}

impl CaptureSession {
    pub fn new() -> Self { /* Idle, 0, 0, false */ }
    pub fn is_grabbing(&self) -> bool { matches!(self.status, Capture::Grabbing) }
    pub fn error(&self) -> Option<&str> { /* Failed */ }
    pub fn set(&mut self, next: Capture) {
        self.status = next;
        self.gen = self.gen.wrapping_add(1);
    }
    pub fn gen(&self) -> u64 { self.gen }
    pub fn should_clear_flash(&self, gen: u64) -> bool {
        self.gen == gen && matches!(self.status, Capture::Failed(_))
    }
    pub fn push_hide(&mut self) { self.hide_depth = self.hide_depth.saturating_add(1); }
    /// Decrement; true when the window should restore (depth reached 0).
    pub fn pop_hide(&mut self) -> bool {
        if self.hide_depth == 0 {
            return false;
        }
        self.hide_depth -= 1;
        self.hide_depth == 0
    }
    pub fn force_show(&mut self) { self.hide_depth = 0; }
    pub fn set_reveal_on_main(&mut self, v: bool) { self.reveal_on_main = v; }
    pub fn take_reveal_on_main(&mut self) -> bool {
        let v = self.reveal_on_main;
        self.reveal_on_main = false;
        v
    }
}

pub(crate) struct IngestPump {
    pub ocr: crate::ocr_queue::OcrQueue,
    pub file_queue: std::collections::VecDeque<std::path::PathBuf>,
    pub file_loading: bool,
    pub thumb_inflight: std::collections::HashSet<uuid::Uuid>,
}

impl IngestPump {
    pub fn new() -> Self { /* defaults */ }
}

pub(crate) struct SearchFilter {
    pub query: String,
    pub gen: u64,
}

impl SearchFilter {
    pub fn bump(&mut self) -> u64 {
        self.gen = self.gen.wrapping_add(1);
        self.gen
    }
}
```

```rust
// AppState fields
pub struct AppState {
    pub library: Library,
    export_fmt: ExportFmt,
    pub prefs: Prefs,
    engine: Arc<Engine>,
    store: Option<Arc<Store>>,
    search: SearchFilter,
    ingest: IngestPump,
    capture: CaptureSession,
    pub main_window: Option<WindowHandle<MainWindow>>,
}
```

GPUI spawn closures still live in `impl AppState` in the split files. `flash_capture_error` uses `capture.set(Failed)` + `capture.gen()` + `should_clear_flash`. Do not put `cx` on `CaptureSession`.

- [ ] **Step 1: Write failing tests** in `src/state/session.rs`:

```rust
#[test]
fn stale_flash_gen_does_not_clear() {
    let mut c = CaptureSession::new();
    c.set(Capture::Failed("x".into()));
    let old = c.gen();
    c.set(Capture::Idle);
    c.set(Capture::Failed("y".into()));
    assert!(!c.should_clear_flash(old));
    assert!(c.should_clear_flash(c.gen()));
}

#[test]
fn hide_depth_restores_only_at_zero() {
    let mut c = CaptureSession::new();
    c.push_hide();
    c.push_hide();
    assert!(!c.pop_hide());
    assert!(c.pop_hide());
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test stale_flash_gen_does_not_clear hide_depth_restores_only_at_zero
```

- [ ] **Step 3: Add types, then `git mv src/state.rs src/state/mod.rs` and split `impl AppState` across capture/ingest/search.** Replace field access (`self.ocr_queue` → `self.ingest.ocr`, `self.search_query` → `self.search.query`, `self.hide_depth` → `self.capture` methods). Public API of `AppState` (`request_capture`, `ingest`, `set_search_query`, …) stays so UI does not churn.

- [ ] **Step 4: Run tests**

```bash
cargo test
cargo fmt --all -- --check
```

Expected: PASS. `src/state/mod.rs` well under 1000 lines.

- [ ] **Step 5: Commit**

```bash
git add src/state src/state.rs
git commit -m "$(cat <<'EOF'
Split AppState into capture, ingest, and search modules.

Session flags get a tested CaptureSession; GPUI tasks stay on AppState methods.
EOF
)"
```

---

### Task 8: Load prefs once; document session format

**Files:**
- Modify: `src/state/mod.rs` (`AppState::new`)
- Modify: `src/main.rs`

**Interfaces:**
- Consumes: `Prefs::load() -> Prefs`
- Produces:

```rust
impl AppState {
    pub fn new(prefs: Prefs) -> Self { /* use prefs; do not call Prefs::load */ }
}
```

```rust
// main.rs
let prefs = crate::prefs::Prefs::load();
crate::state::bind_keys(cx, &prefs.shortcuts);
let state = cx.new(|_| AppState::new(prefs));
```

On `AppState`:

```rust
export_fmt: ExportFmt, // Session copy format. Initialized from prefs.default_fmt.
                       // toggle_format does not persist; Settings writes both fields.
```

- [ ] **Step 1: Change the signature and the one call site.** No new test (loading prefs is IO). Grep `AppState::new` — only `main.rs`.

- [ ] **Step 2: Compile**

```bash
cargo test
rg -n 'Prefs::load' src
```

Expected: `Prefs::load` only in `prefs.rs` and `main.rs`.

- [ ] **Step 3: Commit**

```bash
git add src/state/mod.rs src/main.rs
git commit -m "$(cat <<'EOF'
Load preferences once at startup and pass them into AppState.

Avoids a second disk read and documents that export_fmt is session-only.
EOF
)"
```

---

### Task 9: `SettingsPane` entity

**Files:**
- Modify: `src/ui/settings/mod.rs` (struct + `impl Render` or `page(&mut self, ...)`)
- Modify: `src/ui/main_window.rs` (drop settings/sysmon fields; hold `Entity<SettingsPane>`)
- Modify: shortcut intercept in `MainWindow::new` to read listen state from the pane

**Interfaces:**
- Consumes: Task 5 module split
- Produces:

```rust
pub struct SettingsPane {
    tab: SettingsTab,
    listen: Option<ShortcutId>,
    scroll: ScrollHandle,
    thumb: Rc<RefCell<Option<ScrollThumbDrag>>>,
    sysmon: crate::sysmon::SysMon,
    sys_snap: crate::sysmon::SysSnapshot,
    sysmon_on: bool,
}

impl SettingsPane {
    pub fn new() -> Self { /* General, no listen, empty snap */ }

    pub fn listening(&self) -> Option<ShortcutId> { self.listen }

    pub fn set_listen(&mut self, id: Option<ShortcutId>) { self.listen = id; }

    pub fn dismiss_listen(&mut self) { self.listen = None; }

    /// Build the settings page. Uses self.tab / self.listen / self.sys_snap / self.scroll.
    pub fn view(
        &mut self,
        state: Entity<AppState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement
}
```

`view` has ~3 arguments besides `cx` (state, maybe unused window). Tab change, listen, and `kick_sysmon` live **inside** `SettingsPane`. Move `kick_sysmon` from `MainWindow`.

`MainWindow`:

```rust
pub(crate) settings: Entity<SettingsPane>,
```

In `new`: `let settings = cx.new(|_| SettingsPane::new());`

When `View::Settings`, child is `self.settings.update(cx, |pane, cx| pane.view(state, window, cx))` — **do not** call `settings.update` from inside another `settings.update`. `MainWindow::render` updates MainWindow; nested `self.settings.update` is a different entity (allowed).

Keystroke intercept: `this.read(cx).settings.read(cx).listening()` then `settings.update` to clear/set chord. Binding still calls `AppState::bind_shortcut`.

`dismiss_sheet` / open settings: `settings.update` to `dismiss_listen` and reset scroll (move `reset_settings_scroll` onto the pane).

- [ ] **Step 1: Add `SettingsPane` with the current field values moved over.** Keep painting identical. Compile after each field move if the diff is large.

- [ ] **Step 2: Run unit tests + clippy**

```bash
cargo test
cargo clippy --profile dev-opt --all-targets --message-format=short
```

Expected: `page` 8-arg warning gone. No `too_many_arguments` on `view` (if it still warns, pass a `SettingsChrome` struct with scroll handles only).

- [ ] **Step 3: Linux smoke (when executing, not during plan-only)**

```bash
./scripts/run-on-desktop.sh
./scripts/desktop-ctl.sh shot /tmp/localtex-settings.png
```

Open Settings, switch all four tabs, bind one shortcut, Esc cancels listen. System tab shows CPU numbers updating.

- [ ] **Step 4: Commit**

```bash
git add src/ui/settings src/ui/main_window.rs
git commit -m "$(cat <<'EOF'
Give settings its own GPUI entity.

Tab, shortcut capture, and the system sampler no longer live on MainWindow.
EOF
)"
```

---

### Task 10: Stop scheduling loads from `Render`

**Files:**
- Modify: `src/ui/main_window.rs` (`render`, `new`, `schedule_derived`, `ensure_selected_full`, `schedule_media_gc`)
- Modify: `src/ui/history.rs` (thumb request)

**Interfaces:**
- Consumes: existing `cx.observe(&state, ...)`, `observe_window_bounds`
- Produces: `Render::render` does **not** call `ensure_selected_full`, `schedule_derived`, `schedule_media_gc`, or drain `thumb_need`.

Wiring in `MainWindow::new` after `observe(&state)`:

```rust
cx.observe(&state, |this, _, cx| {
    this.ensure_selected_full(cx);
    this.schedule_derived_from_app(cx); // read scale from stored last_scale or Window via cx
    this.schedule_media_gc(cx);
    cx.notify();
})
.detach();
```

GPUI 0.2 `observe` callback is `Fn(this, entity, cx)`. Window scale: keep last `f32` on `MainWindow` (`last_scale`). Update it from `observe_window_bounds` **and** from `render` **read-only** (`window.scale_factor()` assigned to `last_scale` if changed, then `schedule_derived` via `cx.defer` — not spawn from the paint path). Prefer: `observe_window_bounds` already fires on resize; also compare scale there:

```rust
cx.observe_window_bounds(window, |this, window, cx| {
    let scale = window.scale_factor();
    if (scale - this.last_scale).abs() > f32::EPSILON {
        this.last_scale = scale;
        this.schedule_derived(window, cx);
    }
    // existing sidebar fold...
});
```

On first frame, `new` already has `window`: call `schedule_derived(window, cx)` once at the end of `new`.

Thumbs: in `history.rs` uniform_list callback, instead of `thumb_need.extend(need_thumbs)` + Render drain:

```rust
let state = self.state.clone();
cx.defer(move |cx| {
    for id in need_thumbs {
        state.update(cx, |s, cx| s.request_thumb(id, cx));
    }
});
```

Drop `thumb_need`. Keep `thumb_keep` for GC (still written from the list callback; `schedule_media_gc` reads it from observe/defer, not from Render).

`ensure_selected_full` needs `&App` or `cx: &mut Context<Self>` — today `&self, cx: &App`. Calling from observe with `cx` is fine.

- [ ] **Step 1: Move the three calls out of `render`. Confirm `render` no longer contains those names.**

```bash
rg -n 'ensure_selected_full|schedule_derived|schedule_media_gc|thumb_need' src/ui/main_window.rs
```

`render` must not match. Other methods may.

- [ ] **Step 2: `cargo test` + clippy**

- [ ] **Step 3: Linux smoke**

```bash
./scripts/run-on-desktop.sh
```

Select a snip: preview appears, copy chips appear, history thumbs load while scrolling, zoom original still works. Switch snips quickly: no stuck "Loading…" and no panic.

- [ ] **Step 4: Commit**

```bash
git add src/ui/main_window.rs src/ui/history.rs
git commit -m "$(cat <<'EOF'
Schedule preview and media work from observers, not Render.

Paint uses prepared derived state; thumbs request via defer from the list callback.
EOF
)"
```

---

## Out of scope (do not do in this plan)

- Unifying preview assembly with `export::group_rows`
- Merging `export_fmt` into `prefs.default_fmt`
- Replacing OCR models or `RgbImg`
- Merging X11/Win32 overlays
- Rewriting `ocr/text.rs` `re_static` to `OnceLock` (optional follow-up; not required)
- macOS snip overlay

## Spec coverage (self-review)

| Review item | Task |
|---|---|
| Canonical `BlockKind` / delete scattered `looks_like_html_table` | 1 |
| `split_braced` dedup | 2 |
| Search blob without `Prefs::default` | 2 |
| Drop `OPENDOC_*` | 2 |
| Clippy `while let` / `contains` | 2 |
| Delete `Exporter` / `CopyRow` extras | 3 |
| `preview.rs` over 1k / table layout | 4 |
| `text_run_el` 8 args | 4 |
| `settings.rs` split before 1k | 5 |
| `Library` public mutation | 6 |
| `AppState` god object + tests | 7 |
| Double `Prefs::load` | 8 |
| Session vs default format comment | 8 |
| Settings 8-arg `page` / sysmon on MainWindow | 9 |
| Render-time loading | 10 |

No TBD placeholders. `CopyKind::ALL` order matches the old `EXPORTERS` array. `AppState::new(prefs)` is defined in Task 8; Task 7 still uses `Prefs::load` inside `new` until then — Task 7 implementer must keep `pub fn new() -> Self` until Task 8, or Task 8 is a one-line follow-up (preferred: Task 7 keeps `new()` loading prefs so UI compiles; Task 8 changes the signature).
