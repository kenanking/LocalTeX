# Developing LocalTeX

[Back to the user guide](../README.md)

[Build and run](#build-and-run) · [Models](#models) · [Checks](#checks) · [Packaging and releases](#packaging-and-releases) · [Project layout](#project-layout)

LocalTeX is one Rust package and one application process. Supported desktop targets are Windows and Linux X11. Wayland is out of scope.

## Build and run

Use the latest stable Rust (edition 2024) and clone the repository:

```sh
git clone https://github.com/kenanking/LocalTeX.git
cd LocalTeX
```

### Windows

Install the MSVC Rust toolchain and Visual Studio Build Tools with the C++ desktop workload and Windows SDK. The GPUI build needs the SDK shader compiler (`fxc.exe`); if it is not discovered, set `GPUI_FXC_PATH` to its full path. The [Windows CI setup](../.github/workflows/check.yml) shows the lookup.

```powershell
.\scripts\download-models.ps1
cargo build --profile dev-opt
.\target\dev-opt\localtex.exe
```

### Linux X11

On Ubuntu 22.04, install the build dependencies used by the release workflow:

```sh
sudo apt-get update
sudo apt-get install clang libclang-dev cmake pkg-config \
  libfontconfig-dev libgbm-dev libegl1-mesa-dev libvulkan-dev \
  libpipewire-0.3-dev libx11-dev libx11-xcb-dev libxcb1-dev libxkbcommon-x11-dev

./scripts/download-models.sh
cargo build --profile dev-opt
./target/dev-opt/localtex
```

Run from an X11 graphical session with a working Vulkan driver. Linux release packages target glibc 2.35 or newer. If linking fails on `-lgbm`, install `libgbm-dev`; machine-local linker configuration is documented in [`.cargo/config.toml.example`](../.cargo/config.toml.example).

Use `cargo build --release` for a shipping binary. The `dev-opt` profile skips LTO for faster iteration; its output is separate from `target/release`.

### Build identity

The footer of Settings → System shows the version from `Cargo.toml` and the build-time Git commit. Builds without Git metadata still succeed and show `Git unavailable`. Uncommitted changes are not reflected.

## Models

The download commands above install the two model packs needed by LocalTeX:

| Folder | Used for |
|---|---|
| `opendoc/` | Text, formulas, and tables in images |
| `handwriting/` | Formulas written on the drawing board |

Weights are downloaded separately and stay outside the executable. They are not needed to compile the app, but recognition requires them. Existing installed models are discovered automatically.

By default, the download scripts use `%LOCALAPPDATA%\localtex\models` on Windows and `~/.local/share/localtex/models` on Linux. Set `LOCALTEX_MODELS` to use a different model directory. This override takes precedence, so it must point to a valid installation.

Check **Settings → System** for the selected directory and model status. If models are missing or outdated, rerun the downloader. Pack versions and required files are recorded in [`models/manifest.json`](../models/manifest.json); the download scripts handle cache validation.

## Checks

```sh
cargo fmt --all -- --check
cargo test
cargo clippy --all-targets -- -D warnings
cargo build --profile dev-opt
```

Normal unit tests skip model inference. With weights installed:

```sh
cargo test smoke_if_weights_exist -- --ignored --nocapture --test-threads=1
```

For an isolated Windows UI session, set an absolute, disposable `LOCALTEX_TEST_ROOT` and run the ignored `interactive_windows_ui` test. The normal application ignores that test-only override. See [AGENTS.md](../AGENTS.md) for UI verification rules.

## Packaging and releases

```sh
./scripts/bundle-linux.sh
```

```powershell
.\scripts\bundle-windows.ps1
```

Both scripts build the release binary, download the models, and write packages to `dist/`:

- **Windows:** installer and ZIP. Building the installer requires Inno Setup 7.
- **Linux:** DEB and tar.gz. DEB packaging also requires `dpkg-dev`.

Keep the bundled model folders when unpacking a portable package. Windows uses `models/` next to `localtex.exe`; Linux uses `share/localtex/models/`.

To publish an application release, update `version` in `Cargo.toml` and push a matching `v*` tag. The [release workflow](../.github/workflows/release.yml) builds and uploads the application packages. Check [Releases](https://github.com/kenanking/LocalTeX/releases) for available downloads.

## Project layout

| Path | Responsibility |
|---|---|
| `src/main.rs`, `src/identity.rs` | Startup, native menus, app identity, and data/model paths |
| `src/state/` | Document state, capture lifecycle, and OCR jobs |
| `src/capture.rs`, `src/desktop/` | Screen capture, native overlays, hotkeys, and tray integration |
| `src/ocr/` | Layout, image recognition, and handwriting inference |
| `src/doc.rs`, `src/export.rs` | Recognized blocks and export behavior |
| `src/store/` | Local SQLite history and image storage |
| `src/ui/`, `src/preview/` | GPUI interface, source editor, and formula rendering |
| `assets/`, `resources/` | Icons, Linux desktop entry, and Windows installer resources |
| `scripts/`, `.github/workflows/` | Model downloads, packaging, and CI |

Read [AGENTS.md](../AGENTS.md) before changing platform code or the OCR pipeline.
