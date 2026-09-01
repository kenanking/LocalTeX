# LocalTeX

Offline screenshot OCR for papers and notes: snip the screen, get Markdown or LaTeX. Built with [GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui) (the UI toolkit from [Zed](https://github.com/zed-industries/zed)).

- Global hotkey **Ctrl+Shift+S** (also **Snip** in the window, or the tray)
- Drag-select overlay, click or Esc to cancel
- Mixed **text + formula** document model; Markdown / LaTeX export and copy
- Local **PP-DocLayoutV2** layout + **UniRec-0.1B** recognition (text, formulas, tables; statically linked ONNX Runtime). Formula preview via RaTeX

Release packages include the ONNX packs on disk (~44 MB binary + ~267 MB weights). The packs are not compiled into the executable. Without them the app still starts; the status bar shows missing models and a snip fails instead of inventing text.

## Install

GitHub Releases attach four artifacts. Each one already contains `opendoc/` and `handwriting/`.

| File | Use |
|---|---|
| `localtex_<ver>_amd64.deb` | Debian / Ubuntu |
| `localtex-<ver>-x86_64-unknown-linux-gnu.tar.gz` | Unpack and run `bin/localtex` |
| `LocalTeX-<ver>-x86_64-Setup.exe` | Windows installer (per-user, no admin) |
| `localtex-<ver>-x86_64-pc-windows-msvc.zip` | Unpack and run `localtex.exe` |

The Linux tarball keeps weights at `share/localtex/models/`. The Windows zip keeps them in `models/` next to the exe. The app searches those locations before `~/.local/share/localtex/models` or `%LOCALAPPDATA%\localtex\models`.

To publish a build, bump `version` in `Cargo.toml`, commit, and push a matching `v*` tag. [`.github/workflows/release.yml`](.github/workflows/release.yml) builds Linux and Windows and attaches the files.

## Platforms

| Target | Capture | Snip UI | Hotkey | Tray | Notes |
|---|---|---|---|---|---|
| **Linux X11** | xcap | override-redirect freeze-frame | Ctrl+Shift+S | StatusNotifier (`ksni`) | Current product |
| **Windows** | xcap (WGC) | per-monitor Win32 freeze-frame | Ctrl+Shift+S (UI thread) | `tray-icon` | Dual-monitor / mixed DPI: overlay per display |
| **macOS** | xcap | not implemented yet | same as Windows | `tray-icon` | Compiles; no dedicated QA yet |
| **Linux Wayland** | — | — | — | — | Not supported |

The global hotkey uses `global-hotkey`, which on Linux is **X11 only**. Wayland has no standard global-hotkey API.

## Build

Rust stable. On Linux you need X11 and the usual GPUI/Vulkan stack.

```bash
git clone https://github.com/kenanking/LocalTeX.git
cd LocalTeX
./scripts/download-models.sh    # OpenDoc + handwriting packs from GitHub release v0.0.0
cargo build --profile dev-opt   # daily iteration (skips LTO)
cargo build --release           # smaller/slower link for a ship binary
```

Windows: the same `cargo build --release`, then run `target/release/localtex.exe` on that machine.

If linking fails on `-lgbm` (Linux), see `.cargo/config.toml.example` (unversioned `libgbm.so` often lives only in `-dev`; a local symlink is enough).

## Models

App packages already ship the packs. From a source checkout, install them from the [`v0.0.0` GitHub Release](https://github.com/kenanking/LocalTeX/releases/tag/v0.0.0):

```bash
./scripts/download-models.sh
```

That writes `opendoc/` and `handwriting/` under `$LOCALTEX_MODELS`, or `~/.local/share/localtex/models` on Linux, or `%LOCALAPPDATA%\localtex\models` on Windows. If the GitHub repo is private, log in with `gh` first. File names, graph contracts, and pack versions are in [`models/README.md`](models/README.md).

## Run

From a graphical session:

```bash
./target/release/localtex
```

From SSH / an agent with no `DISPLAY` (this machine: GNOME on X11):

```bash
./scripts/run-on-desktop.sh          # systemd --user, survives the SSH shell
./scripts/run-on-desktop.sh --stop
```

Hotkey **Ctrl+Shift+S** starts a capture. Overlay: drag to confirm; click or Esc cancels.

## Layout

Single crate (`localtex`). Product name / app id / data dir live in `src/identity.rs`. OS-owned services (tray, hotkey, Linux snip overlay) are `#[cfg]` backends; `AppState` is shared.

```
src/
  main.rs           entry, native menus, main window
  identity.rs       LocalTeX / localtex / com.localtex.app / models_dir
  actions.rs        GPUI actions (namespace `localtex`)
  state/            documents, capture lifecycle, OCR jobs
  doc.rs            Block / Document / export
  capture.rs        xcap grab + virtual-desktop stitch
  desktop.rs        DesktopCmd; hotkey on the UI thread
  desktop/linux.rs  ksni tray
  desktop/x11_snip.rs  override-redirect freeze-frame overlay
  desktop/other.rs  tray-icon (Windows / macOS)
  preview/          RaTeX → SVG
  ocr/              PP-DocLayoutV2 + UniRec-0.1B (OpenDoc)
  ui/               main window, theme
resources/          linux .desktop, Windows Inno script
scripts/            desktop/SSH helpers, download-models, bundle-linux / bundle-windows
.github/workflows/  tag-triggered Linux and Windows packages
```

Agent-oriented conventions: [AGENTS.md](./AGENTS.md).

## License

Apache-2.0. GPUI is part of the Zed project.
