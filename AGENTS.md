# LocalTeX agent guide

LocalTeX is a single-crate, single-process Rust desktop app built with git `gpui` and `gpui_platform` pinned to Zed `c8e44cfa7bda9b2e22c8d6934d78969352e7f61a` (v1.17.2), `gpui_platform` features `x11` only. Read the code for architecture; this file only records constraints that are easy to violate.

## Scope

- Keep one Cargo package and one application process. Do not introduce a workspace or extra crates unless explicitly requested.
- Make the smallest coherent change. Avoid unrelated dependencies, abstractions, features, and formatting.
- Preserve unrelated work in the tree. Read relevant callers, platform variants, and nearby tests before editing.
- Linux X11 is the locally verifiable path. Wayland is out of scope; do not claim Windows or macOS runtime verification from this host.

## Checks

```bash
cargo fmt --all -- --check
cargo test
cargo build --profile dev-opt
./scripts/run-on-desktop.sh
./scripts/desktop-ctl.sh shot /tmp/localtex-desktop.png
```

Run checks relevant to the change and report what actually ran. For visible UI changes, inspect the running app when possible. Agent shells may have no `DISPLAY`; build first and use the desktop scripts instead of assuming `cargo run` opened a window.

## Rust and GPUI

- Prefer existing files, `?`, and explicit error handling. Do not silently discard fallible results.
- Name contexts `cx`; order parameters as `window, cx`, with callbacks after `cx`.
- Inside `entity.update`, use the closure's inner `cx`. Never update an entity already being updated.
- Call `cx.notify()` after render-affecting state changes.
- `cx.spawn` runs on the UI thread. Put OCR, capture, image work, and disk access in `cx.background_spawn`.
- GPUI tasks are cancelled when dropped; await, store, or explicitly detach work that must continue.
- Render paths use prepared in-memory state: no I/O, model loading, subprocesses, blocking locks, or full-library scans.
- Use APIs available at the pinned Zed rev; do not chase later Zed or Waku APIs. Here, `svg().path(...)` also requires `text_color` to paint.
- Focusable mouse-down targets inside the history sidebar must stop propagation or the sidebar steals focus.

## Platform traps

- Keep xcap for one-shot capture. Do not replace it with streaming capture or an ffmpeg subprocess.
- Create `GlobalHotKeyManager` on the GPUI UI thread.
- Never open a second GPUI/Vulkan window for selection; use the native X11/Win32 overlays in `desktop/`.
- Freeze-frame, overlay, and crop coordinates are physical pixels at 1:1. Keep overlay placement tied to the same `capture::stitch` grab list.
- On Linux, hide and await the GPUI window before xcap. The overlay's X11 connection must not activate or configure GPUI's XID.
- On Windows, keep one popup and monitor-local DIB per display; preserve the post-minimize WGC wait.

## OCR

- Do not replace or expand the current OCR pipeline unless explicitly requested.
- Keep ONNX sessions lazy-loaded. Missing weights must fail clearly instead of producing placeholder output.
- Do not compile weights into the Rust binary (`include_bytes!`, `rust-embed`, linking `.onnx` as objects). Release packages may ship the on-disk `opendoc/` and `handwriting/` trees beside the executable.

## Packaging

- Linux artifacts come from `scripts/bundle-linux.sh` (tar.gz + deb). Windows artifacts come from `scripts/bundle-windows.ps1` (zip + Inno). Write them to `dist/` (gitignored). Do not add cargo-packager or a second crate.
- Keep the GitHub Release Linux job on `ubuntu-22.04`. That runner is the glibc floor (2.35).
- Do not change `AppId` in `resources/windows/localtex.iss`. Windows treats a new GUID as a second install.

## Hygiene

- User-facing replies are Simplified Chinese; code, comments, identifiers, and commit messages are English.
- Do not commit, push, or add machine-local files unless explicitly requested.
- Add rules only for repeated, non-obvious, actionable failures or deliberate constraints.
