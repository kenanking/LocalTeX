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

GPUI already owns windows / GPU / input. Only OS-owned services use `#[cfg(target_os)]`. Snip overlay lives in `desktop::select_region`; `AppState` must not `cfg`.

| Target | Capture | Snip UI | Global hotkey | Tray | Status |
|---|---|---|---|---|---|
| Linux X11 | xcap | x11rb override-redirect (`desktop/x11_snip.rs`) | `global-hotkey` | `ksni` | Supported (this host) |
| Windows | xcap (WGC) | **not implemented** | `global-hotkey` on UI thread | `tray-icon` | Next test machine |
| macOS | xcap | **not implemented** | same as Windows | `tray-icon` | Compile path only |
| Linux Wayland | incomplete in xcap | — | no standard API | — | **Out of scope** |

- Do not switch capture to GPUI `ScreenCaptureSource` / Zed `scap` (those are live streams). Keep **xcap** for a one-shot freeze-frame. Do not shell-out to ffmpeg from the app.
- `GlobalHotKeyManager` is created inside `Application::run` on the GPUI UI thread. Do not `thread::spawn` the manager (Windows needs that thread's win32 loop; macOS needs main). Event `recv` may still be forwarded off-thread onto `DesktopCmd`.
- Tray is best-effort: if it fails, log and continue (window + in-app Ctrl+Shift+S still work).
- Native menus (`cx.set_menus`) are the Win/mac discoverability path; Linux may not paint a menubar.
- `scripts/desktop-*.sh` are SSH/X11 agent tools, not part of the Windows product.

## GPUI

Follow Zed's GPUI discipline, adapted to **gpui 0.2** in this repo. For *how* a polished GPUI client writes UI (assets, icons, frame budget, theme tokens), [Waku](https://github.com/egoist/waku) is the peer to read — not its layout, and **not** its daemon/workspace split. LocalTeX stays one process on crates.io `gpui 0.2` (Waku tracks a Zed git fork; `WindowOptions.icon` and some `svg()` paths are newer than we have).

Waku icon traps (paid for here, do not cargo-cult):

- In-app chrome: Waku uses `svg().path("icons/foo.svg")` + `AssetSource` (`include_bytes!` map) so GPUI paints an **alpha mask at device DPI**, tinted with `text_color`. Polychrome file-type marks use `img(path)` so authored colors survive. Sources are Lucide-style `viewBox="0 0 24 24"` with **no** `width`/`height`.
- This NVIDIA/Vulkan host: `svg()` + `AssetSource` **loads** but does **not paint** (even a filled rect). Toolbar icons stay `img("icons/*.svg")` with a large declared size (96) so the 2× raster downscales into the 18px slot. Do not reintroduce `svg()` until a probe rect is visibly drawn. The empty-state app tile is polychrome: `img("icon.svg")` stays blank here — keep `Image::from_bytes(ImageFormat::Svg, APP_ICON_SVG)`.
- Toolbar hints: GPUI already owns hover delay/placement via `.tooltip()`. Do not swap the topbar brand label for a hint string. The tooltip view lives in `widgets.rs` (gpui 0.2 has no `shadow_md`).
- App/window icon: Waku embeds a PNG and passes `WindowOptions.icon` (Linux X11). gpui 0.2 has no that field. Linux identity stays `.desktop` + hicolor from `src/icon.rs`. Windows later: PE `.ico` via `build.rs` (Waku's `embed_resource` pattern), not a GPUI bump.

- `cx` is last (after `window` when present). Callbacks come after `cx`.
- Do not nest `entity.update` while that entity is already being updated (panic).
- After mutating view state, call `cx.notify()`.
- `cx.spawn` is the UI thread; OCR / capture go in `cx.background_spawn`. Store or `.detach()` tasks you intend to keep alive.
- Never block `render` with I/O. Image/SVG caches on `MainWindow` are the allowed pattern.
- Actions live in `src/actions.rs`.
- Comments only for non-obvious *why*. Prefer extending an existing file over adding a tiny new one.
- Prefer `?` over `unwrap()`. New modules: `foo.rs`, not `foo/mod.rs` (except `desktop/` and `ocr/`, which already split backends).
- Accent color is for actions and selection, not chrome. Status is a dot **plus** a text label (never color alone).

UI / overlay traps (already paid for):

- Do **not** `open_window` a second GPUI/Vulkan window for capture. On this NVIDIA host a second swapchain presents as Xid 31 (`FAULT_PDE` @ `0xC000`) and blade aborts.
- Linux snip UI is an **override-redirect X11 window** on its own x11rb connection (`desktop/x11_snip.rs`), not the GPUI main window. That overlay connection must not `_NET_ACTIVE_WINDOW` or `ConfigureWindow` GPUI's XID (races XI2, `RefCell already borrowed`). Hide-before-grab is a separate root ClientMessage: `_NET_WM_STATE ADD HIDDEN` (`linux::iconify_main_window`). GPUI's ICCCM `WM_CHANGE_STATE` iconify is not enough on GNOME; wait until unmapped/hidden before xcap.
- Do **not** paint the freeze-frame inside the decorated main window (GNOME `_NET_WORKAREA` excludes the panel/dock; 1:1 shot then shifts up and leaves a black corner).
- Do **not** set `CursorStyle::Crosshair` (X11 can leak the cursor after destroy; cheap insurance on Win/mac too).
- Freeze-frame is **physical pixels, 1:1** (RandR/root). `capture::stitch` builds the virtual desktop; the overlay sits at that origin. Library previews stay on `gpu_display_image` (long edge 1024). Crop from the original `RgbaImage`. Tiny click on the overlay cancels (Mathpix-style). Ungrab pointer/keyboard in `Drop`.

## OCR and binary size

- ONNX weights stay **on disk**, not in the ELF. Do not `include_bytes!` models.
- OCR is `Engine` in `src/ocr/mod.rs`. It is OpenDoc-0.1B: **PP-DocLayoutV2** (`src/ocr/layout.rs`) + **UniRec-0.1B** (`src/ocr/unirec.rs`), orchestrated by `src/ocr/pipeline.rs` which emits `doc::Block`s (formula labels → `BlockKind::Formula` with `$`/`$$` stripped so prefs/preview still wrap; table labels → `BlockKind::Table` HTML, converted to Markdown / LaTeX / TSV on copy). Ship contract only: layout freeze-fold (image-only, boxes in 800-space, output `[N,8]`) and **GQA** decoder (`cross_kt_0` + `seqlens_k`; past `[b,heads,len,dim]`). Reject older KV-v2 / `im_shape` weights at load. Stay on **V2**. Intra-op threads default to `clamp(cores/2, 2, 4)` with spinning **off**; override `LOCALTEX_INTRA_THREADS` / `LOCALTEX_SPINNING=1` (also accepts `OPENDOC_*`). Dark images invert if mean luma < 128 inside the pipeline — do **not** also invert in `imgutil`. Snips larger than **24 MP** are downscaled at ingest (`imgutil::cap_megapixels`) so OCR boxes match the stored PNG. Page-chrome labels are skipped before UniRec. Layout already has reading order. Missing ship files: app still starts, status shows missing, snip OCR returns `Err` (no dummy blocks). Do not add RapidOCR / FormulaNet / SLANet / PP-OCRv6 back, and do not reintroduce RecItem/markdown-file assembly.
- Keep `ort` `download-binaries` (static ONNX Runtime). Do not switch to a system `libonnxruntime.so` for “smaller binary.” Need **ort 2.0.0-rc.13** for MatMulNBits + GQA. Do not drop below rc.13.
- UniRec / DocLayoutV2 inference is **CPU only**.
- Do **not** eager-load ONNX at startup. Ship weights are ~244 MB on disk (`layout.onnx`, `encoder.onnx`, `decoder.onnx`, `unirec_tokenizer_mapping.json`) and become large anonymous RSS once `Session` is created. Install with `./scripts/download-models.sh` (GitHub release `v0.0.0`, asset `opendoc-0.1b-ship.tar.gz`) into `$LOCALTEX_MODELS` or `dirs::data_local_dir()/localtex/models`. `manifest.json` records INT8 + layout freeze-fold + GQA. Sessions are created on the first snip (`Engine` + `Mutex<Option<Pipeline>>`). Do not `commit_from_file` in `Engine::load`. Rebuild ship weights from the ocr-pipeline `models/ship` pipeline (`scripts/build_ship_pipeline.py`). Do not swap in PP-DocLayoutV3.
- Release profile already uses thin LTO + strip. Do not add UPX unless asked.

## Host-only files

- `.cargo/config.toml` is **machine-local** (gitignored). Copy `.cargo/config.toml.example` if `-lgbm` fails.
- `scripts/desktop-*.sh` are for driving a GNOME/X11 desktop from SSH. Do not bake `1920x1080` or `/home/yan/...` into app code.

## Hygiene

- User-facing replies: Simplified Chinese. Code, comments, and commit messages: English.
- Do not commit unless asked. Do not force-push. Do not skip hooks.
- Do not expand scope (new crates, themes, packaging) unless requested.
