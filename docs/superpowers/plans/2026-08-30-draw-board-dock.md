# Draw Board Dock Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the approved canvas dock to Draw a formula: pen/eraser, undo/redo with Excalidraw 1–4 badges, and cream Dots/Lines/Blank paper, without changing sheet chrome or the inktex ingest path.

**Architecture:** Keep stroke math in `DrawBoard` (`src/ui/draw.rs`) with unit tests: vector eraser, snapshot undo/redo, paper constants. Render the existing 40px sheet bar unchanged. Overlay a centered dock on the canvas (`absolute` + known width, same pattern as `orig_view` nav discs). Bind `1`/`2`/`3`/`4`/`ctrl-z`/`ctrl-shift-z` in a `DrawBoard` key context after focusing a new `draw_focus` handle. `traces()` still emits pen polylines only.

**Tech Stack:** Rust 2021, crates.io `gpui 0.2`, existing `icon_btn` / `segmented` / `seg_item`, bundled SVG via `src/icon.rs`.

## Global Constraints

- Single Cargo package, single process. No extra crates.
- Smallest coherent change. Do not replace the OCR pipeline, xcap capture, freeze-frame overlay, or inktex.
- Linux X11 is the verifiable path. Do not claim Windows or macOS runtime verification.
- Identifiers, comments, and commit messages in English. Do not commit unless the user asks.
- GPUI 0.2 only: `window, cx` order; `cx.notify()` after render-affecting state; `svg().path(...)` needs `text_color`; no second GPUI window.
- Render path: no I/O, model loading, or full-library scans.
- Dock mouse-down must `cx.stop_propagation()` so the canvas does not start a stroke.
- User-facing copy stays English in the app (“Draw a formula”, “Dots”, “Lines”, “Blank”).
- `DrawBoard::traces()` remains `Vec<Vec<[f32; 3]>>` after `inktex::deburst`. Eraser must mutate polylines; never append erase marks.

## Agreed UI (from `/tmp/localtex-copy-ui/draw-board.html`)

- Topbar 44px, sheet bar 40px, footer 28px unchanged.
- Canvas `m_4`, `rounded_md`, 1px `BORDER`. Fill cream `PAPER` (`#fbfaf7`) in every paper mode.
- Dock: 40px tall, 4px padding, `BG_RAISED`, `BORDER`, `rounded_md`, bottom 12px, horizontally centered.
- Tools: Pen `1`, Eraser `2`, 1px `TRACK_OFF` separator, Undo `3`, Redo `4`, then Dots/Lines/Blank segmented.
- Digit: 9px monospace, `MUTED`, `right 3px` / `bottom 2px`.
- Default paper: Dots. Default tool: Pen.

## File structure (end state)

```
assets/icons/eraser.svg      lucide-style eraser
assets/icons/undo.svg        curved undo arrow
assets/icons/redo.svg        curved redo arrow
src/icon.rs                  bundle the three SVGs
src/ui/theme.rs              PAPER / PAPER_DOT / PAPER_RULE
src/ui/widgets.rs            IconKind::{Eraser, Undo, Redo}; icon_btn_kbd; seg_item stop_propagation
src/ui/draw.rs               DrawTool, DrawPaper, eraser/undo, paper paint, dock, tests
src/actions.rs               DrawPen, DrawEraser, DrawUndo, DrawRedo
src/keymap.rs                DrawBoard-context bindings
src/ui/main_window.rs        draw_focus; toggle_draw focuses it; on_action handlers
```

Unchanged on purpose: `src/ocr/inktex.rs`, ingest, `IngestSource::Strokes`, sheet-bar labels, Recognize → `traces()`.

---

### Task 1: Eraser, undo snapshots, and paper helpers (tests first)

**Files:**
- Modify: `src/ui/theme.rs` (add three `u32` paper colors)
- Modify: `src/ui/draw.rs` (types + pure functions + tests; do not restyle the sheet yet)

**Interfaces:**
- Produces:

```rust
pub(crate) const PAPER: u32 = 0xfbfaf7;
pub(crate) const PAPER_DOT: u32 = 0xc8c4bc;
pub(crate) const PAPER_RULE: u32 = 0xc9d3e4;
pub(crate) const PAPER_PITCH: f32 = 20.0;
pub(crate) const PAPER_RULE_GAP: f32 = 28.0;
pub(crate) const ERASER_RADIUS: f32 = 7.0;
pub(crate) const INK_WIDTH: f32 = 2.4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DrawTool { Pen, Eraser }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DrawPaper { Dots, Lines, Blank }

pub(crate) fn dist_point_seg(px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32) -> f32;
pub(crate) fn erase_at(lines: &mut Vec<Vec<StrokePt>>, x: f32, y: f32, radius: f32) -> bool;
```

`StrokePt` stays in `draw.rs`. `DrawBoard` gains `tool`, `paper`, `undo: Vec<Vec<Vec<StrokePt>>>`, `redo: Vec<Vec<Vec<StrokePt>>>`, `gesture_before: Option<Vec<Vec<StrokePt>>>`. `new()` defaults `tool = Pen`, `paper = Dots`. `clear_ink()` clears lines + stacks + `t0` + `painting`, and keeps tool/paper.

- [ ] **Step 1: Write the failing tests at the bottom of `src/ui/draw.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{point, px};

    fn ink(points: &[[f32; 2]]) -> Vec<StrokePt> {
        points
            .iter()
            .enumerate()
            .map(|(i, [x, y])| StrokePt {
                pos: point(px(*x), px(*y)),
                t_ms: i as f32,
            })
            .collect()
    }

    #[test]
    fn dist_to_segment_hits_the_middle() {
        let d = dist_point_seg(5.0, 10.0, 0.0, 0.0, 10.0, 0.0);
        assert!((d - 10.0).abs() < 1e-4);
        assert!(dist_point_seg(5.0, 0.0, 0.0, 0.0, 10.0, 0.0) < 1e-4);
    }

    #[test]
    fn erase_splits_a_stroke_and_drops_short_fragments() {
        let mut lines = vec![ink(&[[0.0, 0.0], [10.0, 0.0], [20.0, 0.0], [30.0, 0.0]])];
        assert!(erase_at(&mut lines, 20.0, 0.0, 7.0));
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].len(), 2);
        assert!((f32::from(lines[0][0].pos.x) - 0.0).abs() < 1e-3);
        assert!((f32::from(lines[0][1].pos.x) - 10.0).abs() < 1e-3);
    }

    #[test]
    fn erase_misses_a_distant_stroke() {
        let mut lines = vec![ink(&[[0.0, 0.0], [10.0, 0.0]])];
        assert!(!erase_at(&mut lines, 80.0, 80.0, 7.0));
        assert_eq!(lines[0].len(), 2);
    }

    #[test]
    fn erase_uses_segment_distance_not_only_vertices() {
        let mut lines = vec![ink(&[[0.0, 0.0], [40.0, 0.0]])];
        assert!(erase_at(&mut lines, 20.0, 0.0, 7.0));
        assert!(lines.is_empty() || lines.iter().all(|l| l.len() < 2) || lines.len() == 2);
    }

    #[test]
    fn undo_redo_and_new_stroke_clears_redo() {
        let mut b = DrawBoard::new();
        b.lines = vec![ink(&[[0.0, 0.0], [4.0, 0.0]])];
        b.begin_gesture();
        b.lines.push(ink(&[[8.0, 0.0], [12.0, 0.0]]));
        b.end_gesture();
        assert!(b.can_undo());
        b.undo();
        assert_eq!(b.lines.len(), 1);
        assert!(b.can_redo());
        b.redo();
        assert_eq!(b.lines.len(), 2);
        b.undo();
        b.begin_gesture();
        b.lines.clear();
        b.end_gesture();
        assert!(!b.can_redo());
    }

    #[test]
    fn clear_ink_keeps_tool_and_paper() {
        let mut b = DrawBoard::new();
        b.tool = DrawTool::Eraser;
        b.paper = DrawPaper::Lines;
        b.lines = vec![ink(&[[0.0, 0.0], [4.0, 0.0]])];
        b.clear_ink();
        assert!(b.lines.is_empty());
        assert!(!b.has_ink());
        assert_eq!(b.tool, DrawTool::Eraser);
        assert_eq!(b.paper, DrawPaper::Lines);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib ui::draw::tests -- --nocapture`

Expected: compile error (`dist_point_seg` / `erase_at` / `begin_gesture` not found) or FAIL.

- [ ] **Step 3: Add theme colors and the minimal implementation**

In `src/ui/theme.rs`, after `BORDER`:

```rust
/// Draft-paper fill for the draw canvas (all paper modes).
pub const PAPER: u32 = 0xfbfaf7;
pub const PAPER_DOT: u32 = 0xc8c4bc;
pub const PAPER_RULE: u32 = 0xc9d3e4;
```

In `src/ui/draw.rs`, add `#[derive(Clone)]` on `StrokePt`. Expand `DrawBoard`:

```rust
pub(crate) const PAPER_PITCH: f32 = 20.0;
pub(crate) const PAPER_RULE_GAP: f32 = 28.0;
pub(crate) const ERASER_RADIUS: f32 = 7.0;
pub(crate) const INK_WIDTH: f32 = 2.4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DrawTool {
    Pen,
    Eraser,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DrawPaper {
    Dots,
    Lines,
    Blank,
}

pub(crate) fn dist_point_seg(px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    let vx = bx - ax;
    let vy = by - ay;
    let len2 = vx * vx + vy * vy;
    if len2 < 1e-8 {
        let dx = px - ax;
        let dy = py - ay;
        return (dx * dx + dy * dy).sqrt();
    }
    let t = ((px - ax) * vx + (py - ay) * vy) / len2;
    let t = t.clamp(0.0, 1.0);
    let dx = px - (ax + t * vx);
    let dy = py - (ay + t * vy);
    (dx * dx + dy * dy).sqrt()
}

fn pt_xy(p: &StrokePt) -> (f32, f32) {
    (f32::from(p.pos.x), f32::from(p.pos.y))
}

fn point_hit(p: &StrokePt, x: f32, y: f32, radius: f32) -> bool {
    let (px, py) = pt_xy(p);
    let dx = px - x;
    let dy = py - y;
    dx * dx + dy * dy <= radius * radius
}

fn seg_hit(a: &StrokePt, b: &StrokePt, x: f32, y: f32, radius: f32) -> bool {
    let (ax, ay) = pt_xy(a);
    let (bx, by) = pt_xy(b);
    dist_point_seg(x, y, ax, ay, bx, by) <= radius
}

/// Drop vertices (and split traces) whose point or adjoining segment
/// lies within `radius` of `(x, y)`. Fragments shorter than 2 points go away.
pub(crate) fn erase_at(lines: &mut Vec<Vec<StrokePt>>, x: f32, y: f32, radius: f32) -> bool {
    let old = lines.clone();
    let mut out: Vec<Vec<StrokePt>> = Vec::new();
    for line in lines.drain(..) {
        let n = line.len();
        if n < 2 {
            continue;
        }
        let drop: Vec<bool> = (0..n)
            .map(|i| {
                if point_hit(&line[i], x, y, radius) {
                    return true;
                }
                if i > 0 && seg_hit(&line[i - 1], &line[i], x, y, radius) {
                    return true;
                }
                if i + 1 < n && seg_hit(&line[i], &line[i + 1], x, y, radius) {
                    return true;
                }
                false
            })
            .collect();
        let mut cur: Vec<StrokePt> = Vec::new();
        for (i, pt) in line.into_iter().enumerate() {
            if drop[i] {
                if cur.len() >= 2 {
                    out.push(cur);
                }
                cur = Vec::new();
            } else {
                cur.push(pt);
            }
        }
        if cur.len() >= 2 {
            out.push(cur);
        }
    }
    let changed = out != old;
    *lines = out;
    changed
}
```

`StrokePt` must `#[derive(Clone, PartialEq)]` so `out != old` compiles.

`DrawBoard` fields and methods:

```rust
pub(crate) struct DrawBoard {
    lines: Vec<Vec<StrokePt>>,
    painting: bool,
    t0: Option<Instant>,
    pub(crate) tool: DrawTool,
    pub(crate) paper: DrawPaper,
    undo: Vec<Vec<Vec<StrokePt>>>,
    redo: Vec<Vec<Vec<StrokePt>>>,
    gesture_before: Option<Vec<Vec<StrokePt>>>,
}

impl DrawBoard {
    pub(crate) fn new() -> Self {
        Self {
            lines: Vec::new(),
            painting: false,
            t0: None,
            tool: DrawTool::Pen,
            paper: DrawPaper::Dots,
            undo: Vec::new(),
            redo: Vec::new(),
            gesture_before: None,
        }
    }

    pub(crate) fn clear_ink(&mut self) {
        self.lines.clear();
        self.undo.clear();
        self.redo.clear();
        self.gesture_before = None;
        self.painting = false;
        self.t0 = None;
    }

    pub(crate) fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub(crate) fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub(crate) fn begin_gesture(&mut self) {
        self.gesture_before = Some(self.lines.clone());
    }

    pub(crate) fn end_gesture(&mut self) {
        let Some(before) = self.gesture_before.take() else {
            return;
        };
        if before != self.lines {
            self.undo.push(before);
            self.redo.clear();
        }
        self.painting = false;
    }

    pub(crate) fn undo(&mut self) {
        let Some(prev) = self.undo.pop() else {
            return;
        };
        self.redo.push(self.lines.clone());
        self.lines = prev;
        self.painting = false;
        self.gesture_before = None;
    }

    pub(crate) fn redo(&mut self) {
        let Some(next) = self.redo.pop() else {
            return;
        };
        self.undo.push(self.lines.clone());
        self.lines = next;
        self.painting = false;
        self.gesture_before = None;
    }
    // has_ink, stamp, traces: unchanged
}
```

Change Clear in `render_draw` from `this.board = DrawBoard::new()` to `this.board.clear_ink()`. Recognize may still replace with `DrawBoard::new()` **after** `traces()`, or call `clear_ink()` then keep paper/tool — prefer `clear_ink()` so paper survives Recognize.

For `erase_uses_segment_distance`: with the `drop` rules above, both endpoints of `[0,0]–[40,0]` are 20px from x=20, which is **outside** radius 7, but `seg_hit` is true for both indices → both vertices drop → empty. Assert `!b.has_ink()` / `lines.iter().all(|l| l.len() < 2)` after filtering: `assert!(!lines.iter().any(|l| l.len() >= 2));`

If the third test is too strict, assert `!has_ink` equivalent: `assert!(!lines.iter().any(|l| l.len() >= 2));`

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib ui::draw::tests`

Expected: PASS (all tests in the module).

- [ ] **Step 5: Commit** (skip unless the user asked to commit)

```bash
git add src/ui/theme.rs src/ui/draw.rs
git commit -m "$(cat <<'EOF'
Add vector eraser and snapshot undo for the draw board.

EOF
)"
```

---

### Task 2: Eraser / undo / redo icons and keyed toolbar button

**Files:**
- Create: `assets/icons/eraser.svg`
- Create: `assets/icons/undo.svg`
- Create: `assets/icons/redo.svg`
- Modify: `src/icon.rs` (`BUNDLED`)
- Modify: `src/ui/widgets.rs` (`IconKind`, `icon_btn_kbd`, `seg_item` mouse-down)

**Interfaces:**
- Produces: `IconKind::{Eraser, Undo, Redo}`; `icon_btn_kbd(id, kind, hint, digit: char, active, enabled, on_click)`; existing `every_toolbar_kind_has_an_embedded_asset` still passes.
- Consumes: `icon_btn_sized` with `hit: 32`, `glyph: 16`.

- [ ] **Step 1: Write the three SVGs** (same stroke language as `assets/icons/draw.svg`)

`assets/icons/eraser.svg`:

```svg
<svg xmlns="http://www.w3.org/2000/svg" width="96" height="96" fill="none" stroke="#1a1a1a" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" viewBox="0 0 24 24">
  <path d="m7 15 8-8 3 3-6 6H7z"/>
  <path d="M14 10 11 13"/>
</svg>
```

`assets/icons/undo.svg`:

```svg
<svg xmlns="http://www.w3.org/2000/svg" width="96" height="96" fill="none" stroke="#1a1a1a" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" viewBox="0 0 24 24">
  <path d="M3 7v6h6"/>
  <path d="M3 13a9 9 0 1 0 3-7.7L3 13"/>
</svg>
```

`assets/icons/redo.svg`:

```svg
<svg xmlns="http://www.w3.org/2000/svg" width="96" height="96" fill="none" stroke="#1a1a1a" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" viewBox="0 0 24 24">
  <path d="M21 7v6h-6"/>
  <path d="M21 13a9 9 0 1 1-3-7.7L21 13"/>
</svg>
```

- [ ] **Step 2: Bundle them**

In `src/icon.rs` `bundled!` list, add `"icons/eraser.svg"`, `"icons/undo.svg"`, `"icons/redo.svg"` next to `"icons/draw.svg"`.

- [ ] **Step 3: Extend `IconKind` and add `icon_btn_kbd`**

Add variants to the enum, `asset_path`, and `ALL` (length becomes 17).

After `icon_btn_sized`, add:

```rust
pub fn icon_btn_kbd(
    id: impl Into<SharedString>,
    kind: IconKind,
    hint: impl Into<SharedString>,
    digit: char,
    active: bool,
    enabled: bool,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .relative()
        .size(px(32.))
        .child(icon_btn_sized(
            id,
            kind,
            hint,
            active,
            enabled,
            IconBtnSize {
                hit: px(32.),
                glyph: px(16.),
            },
            on_click,
        ))
        .child(
            div()
                .absolute()
                .right(px(3.))
                .bottom(px(2.))
                .text_size(px(9.))
                .font_family("monospace")
                .text_color(rgb(theme::MUTED))
                .child(digit.to_string()),
        )
}
```

On `seg_item` inner click target, add the same mouse-down stop as `icon_btn`:

```rust
.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
.on_click(move |_, window, cx| on_click(window, cx))
```

- [ ] **Step 4: Run the icon embed test**

Run: `cargo test --lib ui::widgets::tests::every_toolbar_kind_has_an_embedded_asset`

Expected: PASS.

- [ ] **Step 5: Commit** (skip unless asked)

```bash
git add assets/icons/eraser.svg assets/icons/undo.svg assets/icons/redo.svg src/icon.rs src/ui/widgets.rs
git commit -m "$(cat <<'EOF'
Add draw-board tool icons and keyed icon buttons.

EOF
)"
```

---

### Task 3: Canvas cream paper + pen/eraser mouse + dock chrome

**Files:**
- Modify: `src/ui/draw.rs` (`render_draw` only; math already exists)
- Modify: `src/ui/main_window.rs` (mouse-up ends an in-progress gesture if the pointer is released outside the pad)

**Interfaces:**
- Consumes: `DrawTool`, `DrawPaper`, `erase_at`, `ERASER_RADIUS`, `INK_WIDTH`, `icon_btn_kbd`, `segmented`, `seg_item`, `theme::PAPER*`
- Produces: visible dock; cream paper; eraser mutates `lines`; Clear uses `clear_ink`

**Layout (do not use a full-width overlay that eats hits):**

```rust
pub(crate) const DOCK_W: f32 = 304.0;
```

Pad wrapper: `.id("draw-canvas").relative().flex_1().min_h_0().m_4()`.

Inner pad: `.size_full().rounded_md().border_1().border_color(BORDER).overflow_hidden().cursor(...)` with `cursor` = `CursorStyle::Crosshair` for pen, `CursorStyle::Arrow` for eraser.

Child 1: `canvas` `size_full()` — paint paper then ink.

Child 2: dock

```rust
div()
    .id("draw-dock")
    .absolute()
    .bottom(px(12.))
    .left(gpui::relative(0.5))
    .ml(px(-DOCK_W / 2.0))
    .w(px(DOCK_W))
    .h(px(40.))
    .px(px(4.))
    .flex()
    .items_center()
    .gap(px(2.))
    .rounded_md()
    .bg(rgb(theme::BG_RAISED))
    .border_1()
    .border_color(rgb(theme::BORDER))
    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
```

Paper segmented: wrap `segmented([...])` in `div().w(px(148.)).ml(px(4.))` with `seg_item("draw-paper-dots", "Dots", ...)`, `"Lines"`, `"Blank"`.

- [ ] **Step 1: Paint paper in the canvas callback using `bounds`**

Change `canvas` to the gpui 0.2 pair `(prepaint, paint)` where paint receives `bounds: Bounds<Pixels>` (same as `orig_view.rs` prepaint). Fill `bounds` with `theme::PAPER` via `window.paint_quad(fill(bounds, rgb(theme::PAPER)))`. Then:

- `DrawPaper::Dots`: 2×2 quads at `origin + 10 + n * PAPER_PITCH` in both axes, color `PAPER_DOT`, while inside `bounds`.
- `DrawPaper::Lines`: 1px-tall quads, y = origin.y + 27 + n * `PAPER_RULE_GAP`, color `PAPER_RULE`, full width of bounds.
- `DrawPaper::Blank`: fill only.

Then stroke remaining polylines with `PathBuilder::stroke(px(INK_WIDTH))` and `theme::TEXT` as today.

Need `use gpui::{fill, Bounds, ...}` (or `gpui::fill`). Capture `paper` and `lines` in the paint closure (clone `paper` and point lists before the canvas, like today’s `lines`).

- [ ] **Step 2: Route mouse by tool**

On pad (not dock) left down:

```rust
this.board.begin_gesture();
this.board.painting = true;
let x = f32::from(ev.position.x);
let y = f32::from(ev.position.y);
match this.board.tool {
    DrawTool::Pen => {
        let pt = this.board.stamp(ev.position);
        this.board.lines.push(vec![pt]);
    }
    DrawTool::Eraser => {
        erase_at(&mut this.board.lines, x, y, ERASER_RADIUS);
    }
}
cx.notify();
```

Move: if `!painting` return. Pen: `stamp` + push to last line. Eraser: `erase_at`.

Up: `this.board.end_gesture(); cx.notify();`

In `MainWindow` existing `on_mouse_up` (the window-level one), if `this.board.painting` or `gesture_before.is_some()`, call `this.board.end_gesture()` so a release outside the pad still commits undo.

- [ ] **Step 3: Build the four keyed buttons + paper seg**

Tooltips: `"Pen  1"`, `"Eraser  2"`, `"Undo  3"`, `"Redo  4"`.

Pen/Eraser `active` from `board.tool`. Undo/Redo `enabled` from `can_undo` / `can_redo`.

Clicks: set `tool`; or `undo()` / `redo()`; or `paper = DrawPaper::*`. Always `cx.notify()`.

- [ ] **Step 4: Compile**

Run: `cargo test --lib ui::draw::tests && cargo build --profile dev-opt`

Expected: tests PASS; build succeeds.

- [ ] **Step 5: Commit** (skip unless asked)

```bash
git add src/ui/draw.rs src/ui/main_window.rs
git commit -m "$(cat <<'EOF'
Paint draft paper and overlay the draw-board dock.

EOF
)"
```

---

### Task 4: DrawBoard key context (`1`–`4`, undo chords) and focus

**Files:**
- Modify: `src/actions.rs`
- Modify: `src/keymap.rs` (`apply`)
- Modify: `src/ui/main_window.rs`
- Modify: `src/ui/draw.rs` (`track_focus` + `key_context`)

**Interfaces:**
- Produces: actions `DrawPen`, `DrawEraser`, `DrawUndo`, `DrawRedo`
- Bindings (only `Some("DrawBoard")`): `"1"`, `"2"`, `"3"`, `"4"`, `"ctrl-z"`, `"ctrl-shift-z"`
- Do **not** add these to `keymap::CATALOG` / settings.

- [ ] **Step 1: Declare actions**

In `src/actions.rs` `actions!` list, append `DrawPen, DrawEraser, DrawUndo, DrawRedo`.

- [ ] **Step 2: Bind keys**

In `keymap::apply`, after the existing `bind_keys` array, add:

```rust
cx.bind_keys([
    KeyBinding::new("1", DrawPen, Some("DrawBoard")),
    KeyBinding::new("2", DrawEraser, Some("DrawBoard")),
    KeyBinding::new("3", DrawUndo, Some("DrawBoard")),
    KeyBinding::new("4", DrawRedo, Some("DrawBoard")),
    KeyBinding::new("ctrl-z", DrawUndo, Some("DrawBoard")),
    KeyBinding::new("ctrl-shift-z", DrawRedo, Some("DrawBoard")),
]);
```

Import the four actions in `keymap.rs`.

- [ ] **Step 3: Focus handle + handlers**

`MainWindow`: add `pub(crate) draw_focus: FocusHandle`. In `new`, `let draw_focus = cx.focus_handle();`.

`toggle_draw(&mut self, window: &mut Window, cx: &mut Context<Self>)`: when entering Draw, `window.focus(&self.draw_focus)`; when leaving via toggle, `window.focus(&self.snip_list_focus)`.

Update `start_draw` to pass `window`. Update the topbar draw button:

```rust
move |window, cx| {
    entity.update(cx, |this, cx| this.toggle_draw(window, cx));
}
```

`dismiss_sheet`: if leaving Draw, `window` is not always available. Keep `dismiss_sheet(&mut self, cx)` as today; in `Render`, if `view == Draw` and `!self.draw_focus.is_focused(window)`, `window.focus(&self.draw_focus)` (mirror `orig_focus`).

Handlers:

```rust
fn draw_pen(&mut self, _: &DrawPen, _: &mut Window, cx: &mut Context<Self>) {
    if !matches!(self.view, View::Draw) {
        return;
    }
    self.board.tool = DrawTool::Pen;
    cx.notify();
}
fn draw_eraser(...) { self.board.tool = DrawTool::Eraser; cx.notify(); }
fn draw_undo(...) {
    if !matches!(self.view, View::Draw) { return; }
    self.board.undo();
    cx.notify();
}
fn draw_redo(...) { self.board.redo(); cx.notify(); }
```

Register `.on_action(cx.listener(Self::draw_pen))` (and the other three) on the main `div` next to existing actions.

On the draw root in `render_draw`:

```rust
.id("draw")
.track_focus(&self.draw_focus)
.key_context("DrawBoard")
```

`render_draw` needs `&self.draw_focus` — it already has `&mut self`. Use `self.draw_focus.clone()`.

- [ ] **Step 4: Run tests + build**

Run:

```bash
cargo fmt --all -- --check
cargo test
cargo build --profile dev-opt
```

Expected: fmt clean; all tests PASS; build succeeds.

- [ ] **Step 5: Commit** (skip unless asked)

```bash
git add src/actions.rs src/keymap.rs src/ui/main_window.rs src/ui/draw.rs
git commit -m "$(cat <<'EOF'
Bind draw-board tools to 1-4 and ctrl-z in DrawBoard context.

EOF
)"
```

---

### Task 5: Linux X11 visual check

**Files:** none (run scripts only)

- [ ] **Step 1: Launch on the desktop session**

```bash
./scripts/run-on-desktop.sh --stop
./scripts/run-on-desktop.sh
```

Expected: unit starts; window title `LocalTeX`.

- [ ] **Step 2: Open Draw a formula and screenshot**

```bash
source ./scripts/desktop-env.sh
./scripts/desktop-ctl.sh focus LocalTeX
# Draw icon is the 4th 32px tool after the brand (approx x=12+brand+3*36)
./scripts/desktop-ctl.sh shot /tmp/localtex-draw-dock.png
```

Click the draw tool (or send Ctrl+D if the helper can), then shot again.

Check the PNG:

- Cream canvas, not pure white.
- Dock centered on the canvas, not in the 40px sheet bar.
- Dots visible by default.
- Digit `1`–`4` on the four buttons.
- Sheet bar still `Draw a formula` / Clear / Cancel / Recognize.
- Footer still `Ready when you are` (or current status).

- [ ] **Step 3: Exercise tools (manual on the desktop)**

Pen draws; `2` erases nearby ink without adding a gray stroke; `3`/`ctrl-z` restores; `4` reapplies; Dots/Lines/Blank keep cream; Recognize still produces a formula snip (inktex path unchanged).

- [ ] **Step 4: Commit** (skip unless asked) — none if no file changes.

---

## Self-review

**Spec coverage**

| Spec item | Task |
| --- | --- |
| Dock on canvas, sheet chrome unchanged | 3 |
| Pen/Eraser/Undo/Redo + Dots/Lines/Blank | 3 |
| Cream paper all modes; grid not in inktex | 1, 3 |
| Vector eraser radius 7 | 1 |
| Snapshot undo/redo | 1 |
| 1–4 badges | 2, 3 |
| Keys in DrawBoard context only | 4 |
| Clear keeps tool/paper | 1 |
| No OCR/ingest change | (explicit non-goals) |
| Linux X11 verify | 5 |

**Placeholder scan:** none.

**Type consistency:** `DrawTool` / `DrawPaper` / `erase_at` / `begin_gesture` / `end_gesture` / `clear_ink` / `icon_btn_kbd` / `DOCK_W` / `DrawPen` match across tasks.
