# Agent instructions

LocalTeX is a **single-crate GPUI 0.2 app**: screenshot → local OCR → Markdown / LaTeX.
Keep it that way. Do not introduce a Zed-style Cargo workspace or extra crates unless the user asks.

This file is the project rule set (same role as [Zed](https://github.com/zed-industries/zed)'s `.rules` / `AGENTS.md`). Prefer traps over architecture maps; `README.md` is the human overview.

## Commands

```bash
cargo build --profile dev-opt   # daily iteration (no LTO)
cargo build --release           # distribution: thin LTO + strip
./scripts/download-models.sh    # OpenDoc ship from GitHub release v0.0.0 → models dir
./scripts/run-on-desktop.sh           # launch on the physical display (survives SSH)
./scripts/run-on-desktop.sh --stop
./scripts/desktop-ctl.sh windows      # host helpers: focus / click / shot
```

Models are **not** in git (`*.onnx` is gitignored). Fetch them from the `v0.0.0` GitHub Release of [kenanking/LocalTeX](https://github.com/kenanking/LocalTeX) (`opendoc-0.1b-ship.tar.gz`, plus `manifest.json` / `SHA256SUMS`). `download-models.sh` prefers `gh release download` (needed while the repo is private); otherwise it tries the public `releases/download` URL. Override dest/tag with `LOCALTEX_MODELS`, `LOCALTEX_MODELS_REPO`, `LOCALTEX_MODELS_TAG`. Windows: extract the tarball into `%LOCALAPPDATA%\localtex\models`.

Agent shells often have **no `DISPLAY`**. Never `cargo run` and assume a window appeared. On this Linux host use `run-on-desktop.sh`, then `desktop-ctl.sh` / `screenshot-desktop.sh` to verify.

Windows: `cargo build --release` then run `localtex.exe` on that machine. This Linux host cannot verify the Win/mac path.

## Identity

`src/identity.rs` is the only source for product name, slug, app id, and the models directory.

- Display name: `LocalTeX`
- Crate / binary / XDG slug / `actions!` namespace / `key_context`: `localtex`
- App id: `com.localtex.app`
- Models: `$LOCALTEX_MODELS` or `dirs::data_local_dir()/localtex/models`
  - Linux: `~/.local/share/localtex/models`
  - Windows: `%LOCALAPPDATA%\localtex\models`

Do not scatter `"Texpix"` / `"texpix"` (legacy) or invent a new name.

## Target matrix

GPUI already owns windows / GPU / input. Only OS-owned services use `#[cfg(target_os)]`. Overlay and `AppState` must not `cfg`.

| Target | Capture | Global hotkey | Tray | Status |
|---|---|---|---|---|
| Linux X11 | xcap | `global-hotkey` | `ksni` | Supported (this host) |
| Windows | xcap (WGC) | `global-hotkey` on UI thread | `tray-icon` | Next test machine |
| macOS | xcap | same as Windows | `tray-icon` | Compile path only |
| Linux Wayland | incomplete in xcap | no standard API | — | **Out of scope** |

- Do not switch capture to GPUI `ScreenCaptureSource` / Zed `scap` (those are live streams). Keep **xcap** for a one-shot freeze-frame. Do not shell-out to ffmpeg from the app.
- `GlobalHotKeyManager` is created inside `Application::run` on the GPUI UI thread. Do not `thread::spawn` the manager (Windows needs that thread's win32 loop; macOS needs main). Event `recv` may still be forwarded off-thread onto `DesktopCmd`.
- Tray is best-effort: if it fails, log and continue (window + in-app Ctrl+Shift+S still work).
- Native menus (`cx.set_menus`) are the Win/mac discoverability path; Linux may not paint a menubar.
- `scripts/desktop-*.sh` are SSH/X11 agent tools, not part of the Windows product.

## GPUI

Follow Zed's GPUI discipline, adapted to **gpui 0.2** in this repo. For *what* a polished native GPUI client feels like (theme tokens, empty state, menus), [Waku](https://github.com/egoist/waku) is a useful peer. Do **not** copy Waku's daemon/workspace split — LocalTeX stays one process.

- `cx` is last (after `window` when present). Callbacks come after `cx`.
- Do not nest `entity.update` while that entity is already being updated (panic).
- After mutating view state, call `cx.notify()`.
- `cx.spawn` is the UI thread; OCR / capture go in `cx.background_spawn`. Store or `.detach()` tasks you intend to keep alive.
- Never block `render` with I/O. Image/SVG caches on `MainWindow` are the allowed pattern.
- Actions live in `src/actions.rs`. Overlay keys must stay scoped to `key_context("Overlay")`.
- Comments only for non-obvious *why*. Prefer extending an existing file over adding a tiny new one.
- Prefer `?` over `unwrap()`. New modules: `foo.rs`, not `foo/mod.rs` (except `desktop/` and `ocr/`, which already split backends).
- Accent color is for actions and selection, not chrome. Status is a dot **plus** a text label (never color alone).

UI / overlay traps (already paid for):

- Overlay is `WindowKind::Normal`. Linux fullscreen is `_NET_WM_STATE ADD` in `desktop::raise_overlay` (`overlay_window_bounds` is Windowed). Do **not** `toggle_fullscreen` after map — GPUI's X11 hint is TOGGLE, `is_fullscreen()` is stale, and a second toggle cancels the first. Close overlay **before** restoring the main window (`cx.defer`).
- Linux reuses one overlay GPUI window (unmap on dismiss, map on next snip). Destroying and opening a second X11 window in the same process often gets no XI2/focus. Session API: `arm_overlay` / `raise_overlay` / `park_overlay`. Raises no-op after park so in-flight retries cannot map a parked overlay back.
- Do **not** set `CursorStyle::Crosshair` (X11 can leak the cursor after destroy; cheap insurance on Win/mac too).
- Freeze-frame is **physical pixels, 1:1**. Never `ObjectFit::Fill` a full-monitor shot into `_NET_WORKAREA`. Windows DPI uses GPUI's `scale_factor()`, not a second manual scale.
- Tiny click on the overlay cancels (Mathpix-style). Do not early-return and leave the overlay up.

## OCR and binary size

- ONNX weights stay **on disk**, not in the ELF. Do not `include_bytes!` models.
- OCR is `Engine` in `src/ocr/mod.rs`. It is OpenDoc-0.1B: **PP-DocLayoutV2** (`src/ocr/layout.rs`) + **UniRec-0.1B** (`src/ocr/unirec.rs`), orchestrated by `src/ocr/pipeline.rs` which emits `doc::Block`s (formula labels → `BlockKind::Formula` with `$`/`$$` stripped so prefs/preview still wrap; tables stay `Text` HTML). Decoder **must** be KV-v2 (`cross_kt_0` input). Intra-op threads default **8**. Dark images invert if mean luma < 128 inside the pipeline — do **not** also invert in `imgutil`. Page-chrome labels are skipped before UniRec. Layout already has reading order. Missing ship files: app still starts, status shows missing, snip OCR returns `Err` (no dummy blocks). Do not add RapidOCR / FormulaNet / SLANet / PP-OCRv6 back, and do not reintroduce RecItem/markdown-file assembly.
- Keep `ort` `download-binaries` (static ONNX Runtime). Do not switch to a system `libonnxruntime.so` for “smaller binary.” Need **ort 2.0.0-rc.13** for MatMulNBits + the KV-v2 decoder. Do not drop below rc.13.
- UniRec / DocLayoutV2 inference is **CPU only**.
- Do **not** eager-load ONNX at startup. Ship weights are ~244 MB on disk (`layout.onnx`, `encoder.onnx`, `decoder.onnx`, `unirec_tokenizer_mapping.json`) and become large anonymous RSS once `Session` is created. Install with `./scripts/download-models.sh` (GitHub release `v0.0.0`, asset `opendoc-0.1b-ship.tar.gz`) into `$LOCALTEX_MODELS` or `dirs::data_local_dir()/localtex/models`. `manifest.json` in that tarball records the ship strategy (int8/WOQ + decoder KV-v2). Sessions are created on the first snip (`Engine` + `Mutex<Option<Pipeline>>`). Do not `commit_from_file` in `Engine::load`.
- Release profile already uses thin LTO + strip. Do not add UPX unless asked.

## Host-only files

- `.cargo/config.toml` is **machine-local** (gitignored). Copy `.cargo/config.toml.example` if `-lgbm` fails.
- `scripts/desktop-*.sh` are for driving a GNOME/X11 desktop from SSH. Do not bake `1920x1080` or `/home/yan/...` into app code.

## Hygiene

- User-facing replies: Simplified Chinese. Code, comments, and commit messages: English.
- Do not commit unless asked. Do not force-push. Do not skip hooks.
- Do not expand scope (new crates, themes, packaging) unless requested.
