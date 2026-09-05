# LocalTeX agent guide

LocalTeX is a Rust/GPUI desktop app for Windows and Linux X11; Wayland is out of scope. This file records project boundaries and non-obvious traps. Dependency versions and features live in `Cargo.toml`.

## Scope

- Keep one Cargo package and one application process. Do not introduce a workspace or extra crates unless explicitly requested.
- Make the smallest coherent change. Avoid unrelated dependencies, abstractions, features, and formatting.
- Preserve unrelated work in the tree.
- Complete the requested change through relevant verification and fixes for regressions it causes; local build/test iterations do not need separate approval.

## Checks

Choose checks for the affected behavior; documentation-only edits do not need Rust builds or tests. Available checks:

```text
cargo fmt --all -- --check
cargo test
cargo build --profile dev-opt
```

CI also runs `cargo clippy --all-targets -- -D warnings`. Report checks and platforms actually exercised. For visible UI changes, inspect the running app when possible.

- Linux UI: if the shell lacks `DISPLAY`, discover the active X11 session and `XAUTHORITY`, then launch the built app with `systemd-run --user` using those values.
- Windows UI: build `target/dev-opt/localtex.exe` and verify the running process path. For schema and destructive UI checks, use the ignored `interactive_windows_ui` test with an absolute, disposable `LOCALTEX_TEST_ROOT`; the normal executable ignores this override.
- OCR inference tests are ignored by default and require installed weights. For model-related changes, see `models/README.md` for installation and smoke-test commands.

## Rust and GPUI

- Name contexts `cx`; order parameters as `window, cx`, with callbacks after `cx`.
- Inside `entity.update`, use the closure's inner `cx`. Never update an entity already being updated.
- Call `cx.notify()` after render-affecting state changes.
- `cx.spawn` runs on the UI thread. Put OCR, capture, image work, and disk access in `cx.background_spawn`.
- GPUI tasks are cancelled when dropped; await, store, or explicitly detach work that must continue.
- Render paths use prepared in-memory state: no I/O, model loading, subprocesses, blocking locks, or full-library scans.
- Use APIs available at the Zed revision pinned in `Cargo.toml`. Here, `svg().path(...)` also requires `text_color` to paint.
- Focusable mouse-down targets inside the history sidebar must stop propagation or the sidebar steals focus.

## Platform traps

- Keep xcap for one-shot capture. Do not replace it with streaming capture or an ffmpeg subprocess.
- Create `GlobalHotKeyManager` on the GPUI UI thread on platforms that use it. Windows uses its dedicated low-level hook thread.
- Never open a second GPUI/Vulkan window for selection; use the native X11/Win32 overlays in `src/desktop/`.
- Freeze-frame, overlay, and crop coordinates are physical pixels at 1:1. Keep overlay placement tied to the same `capture::stitch` grab list.
- When hide-on-capture is enabled, await the GPUI window's hide/minimize before xcap. The Linux overlay's X11 connection must not activate or configure GPUI's XID.
- On Windows, keep one popup and monitor-local DIB per display; preserve the post-minimize WGC wait.

## OCR

- Do not replace or expand the current OCR pipeline unless explicitly requested.
- Keep ONNX sessions lazy-loaded. Missing weights must fail clearly instead of producing placeholder output.
- Keep weights on disk, outside git and the Rust binary. Bundles ship `models/opendoc/` and `models/handwriting/` beside the executable.

## Packaging

- Use `scripts/bundle-linux.sh` (tar.gz + deb) or `scripts/bundle-windows.ps1` (zip + Inno), with artifacts in `dist/` (gitignored). Do not add cargo-packager.
- For model installation, cache validation, and discovery, see `models/README.md` and the matching `scripts/download-models.*`. Bundle scripts already invoke the downloader. Do not call bash from Windows packaging; WindowsApps `bash.exe` is often a WSL stub.
- When publishing model packs: package ocr-pipeline's `current/opendoc` and `current/handwriting` outside this repo under the dated directory names in both download scripts. Keep those names, required files, and checksums aligned with `models/manifest.json` and `models/README.md`. The `v0.0.0` model release needs both tarballs, the manifest, and `SHA256SUMS` covering all three.
- Keep the GitHub Release Linux job on `ubuntu-22.04`. That runner is the glibc floor (2.35).
- Do not change `AppId` in `resources/windows/localtex.iss`. Windows treats a new GUID as a second install.
- Windows packaging uses Inno Setup 7. User-scope installs may omit `ISCC.exe` from PATH; compiler discovery is in the bundle script, and CI installation is in `.github/workflows/release.yml`.
- `Setup.exe` needs `SetupIconFile` separately from the app icon. `build.rs` generates `target/localtex.ico` for the bundle script; do not check the ICO into git.

## Hygiene

- User-facing replies are Simplified Chinese; code, comments, identifiers, and commit messages are English.
- Commit and push only when requested. Keep machine-local configuration and generated artifacts out of git.
- Add rules only for repeated, non-obvious, actionable failures or deliberate constraints. Keep task-specific procedures in their existing docs/scripts and link them where relevant; remove stale or redundant rules.
