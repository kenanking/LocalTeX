# LocalTeX

Offline screenshot OCR for papers and notes: snip the screen, get Markdown or LaTeX. Built with [GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui) (the UI toolkit from [Zed](https://github.com/zed-industries/zed)).

- Global hotkey **Ctrl+Alt+M** to snip (also **Snip** in the window, or the tray); **Ctrl+Alt+L** toggles the main window
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

To package from a checkout, run `./scripts/bundle-linux.sh` or `.\scripts\bundle-windows.ps1`. Artifacts land in `dist/` (gitignored). The release binary still comes from `target/release`. The Windows bundle uses `download-models.ps1` (no bash / WSL) and Inno Setup 7 (`ISCC.exe`).

## Platforms

| Target | Capture | Snip UI | Hotkey | Tray | Notes |
|---|---|---|---|---|---|
| **Linux X11** | xcap | override-redirect freeze-frame | Ctrl+Alt+M | StatusNotifier (`ksni`) | Current product |
| **Windows** | xcap (WGC) | per-monitor Win32 freeze-frame | Ctrl+Alt+M (low-level hook thread) | `tray-icon` | Dual-monitor / mixed DPI: overlay per display |
| **macOS** | xcap | not implemented yet | same as Windows | `tray-icon` | Compiles; no dedicated QA yet |
| **Linux Wayland** | — | — | — | — | Not supported |

Windows global shortcuts use a low-level keyboard hook. Other platforms use `global-hotkey`, which on Linux is **X11 only**. Wayland has no standard global-hotkey API.

## Build

The footer of Settings → System shows the package version from `Cargo.toml` and the build-time Git HEAD (12 characters, with the full commit in a tooltip). Uncommitted changes are not reflected. Shallow clones work; building without Git or project Git metadata still succeeds and shows `Git unavailable` for the commit. Running the app does not require Git.

Rust stable. On Linux you need X11 and the usual GPUI/Vulkan stack.

```bash
git clone https://github.com/kenanking/LocalTeX.git
cd LocalTeX
./scripts/download-models.sh    # OpenDoc + handwriting packs from GitHub release v0.0.0
cargo build --profile dev-opt   # daily iteration (skips LTO)
cargo build --release           # smaller/slower link for a ship binary
```

Windows: use `cargo build --profile dev-opt`, then run `target/dev-opt/localtex.exe` for development. Existing installed models are discovered automatically, including custom Inno installation locations. Set `LOCALTEX_MODELS` for an explicit override; an invalid override reports missing models. See [model discovery](models/README.md).

If linking fails on `-lgbm` (Linux), see `.cargo/config.toml.example` (unversioned `libgbm.so` often lives only in `-dev`; a local symlink is enough).

## Models

App packages already ship the packs. From a source checkout, install them from the [`v0.0.0` GitHub Release](https://github.com/kenanking/LocalTeX/releases/tag/v0.0.0):

```bash
./scripts/download-models.sh
```

Windows:

```powershell
.\scripts\download-models.ps1
```

That writes `opendoc/` and `handwriting/` under `$LOCALTEX_MODELS`, or `~/.local/share/localtex/models` on Linux, or `%LOCALAPPDATA%\localtex\models` on Windows. If the GitHub repo is private, log in with `gh` first. File names, graph contracts, and pack versions are in [`models/README.md`](models/README.md).

## Run

From a graphical session:

```bash
./target/release/localtex
```

Hotkey **Ctrl+Alt+M** starts a capture. Overlay: drag to confirm; click or Esc cancels. **Ctrl+Alt+L** toggles the main window.

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
  desktop.rs        DesktopCmd and platform hotkey service
  desktop/linux.rs  ksni tray
  desktop/x11_snip.rs  override-redirect freeze-frame overlay
  desktop/other.rs  tray-icon (Windows / macOS)
  preview/          RaTeX → SVG
  ocr/              PP-DocLayoutV2 + UniRec-0.1B (OpenDoc)
  ui/               main window, theme
resources/          linux .desktop, Windows Inno script
scripts/            download-models.sh / .ps1, bundle-linux / bundle-windows
.github/workflows/  tag-triggered Linux and Windows packages
```

Agent-oriented conventions: [AGENTS.md](./AGENTS.md).

## License

MIT. GPUI is part of the Zed project.
