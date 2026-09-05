# LocalTeX model card

Weights stay on disk, not in git and not inside the Rust binary. Release
packages ship `opendoc/` and `handwriting/` beside the executable. From a
source checkout, install the dated tarballs on the
[`v0.0.0` GitHub Release](https://github.com/kenanking/LocalTeX/releases/tag/v0.0.0).

```bash
./scripts/download-models.sh
```

```powershell
.\scripts\download-models.ps1
```

Destination is `$LOCALTEX_MODELS`, else `~/.local/share/localtex/models` (Windows: `%LOCALAPPDATA%\localtex\models`).
An existing destination or local cache is reused only when its manifest names
both current packs, all required files exist, and the dynamic layout model's
SHA-256 matches this release. Stale or partial caches are skipped.

## Directory layout

Runtime discovery is shared by development builds and test executables. A nonempty
`LOCALTEX_MODELS` is authoritative, including an invalid path. Otherwise discovery
checks executable-adjacent `models` (also above a test `deps` directory), the Linux
prefix, user data, the fixed Windows installer AppId's `InstallLocation`, and
`%LOCALAPPDATA%\Programs\LocalTeX\models`. It prefers a complete pair; if none exists,
it uses the first complete single pack without combining different roots.
Settings → System displays the selected directory and source. No automatic
download or permanent environment changes occur.

Run installed-model checks explicitly with
`cargo test smoke_if_weights_exist -- --ignored --nocapture --test-threads=1`.
These tests fail when required models are absent; normal unit tests skip inference.

```
models/
  opendoc/
  handwriting/
  manifest.json
```

ocr-pipeline keeps dated directories plus `current/` because it holds
several pack versions. LocalTeX only installs the pair in use.

`manifest.json` on the release is the machine-readable copy of this card.

## OpenDoc-0.1B (`opendoc_dynamic_int8_20260903_d8c4e76`)

Tarball: `opendoc_dynamic_int8_20260903_d8c4e76.tar.gz` (~244 MB unpacked). Installed as `opendoc/`.

| File | Role |
|---|---|
| `layout.onnx` | PP-DocLayoutV2, dynamic H/W, raw layout outputs |
| `encoder.onnx` | UniRec-0.1B encoder |
| `decoder.onnx` | UniRec-0.1B decoder (GQA, `cross_kt_0` + `seqlens_k`) |
| `unirec_tokenizer_mapping.json` | UniRec tokenizer |

Strategy: dynamic layout with FP32 position trigonometry, INT8 conv weights,
WOQ INT8 MatMul, and decoder GQA. LocalTeX selects `800×800` for ordinary
pages and `1280×320` for images whose width/height ratio is at least 4.

## Handwriting (`handwriting_e10_20260903_cf27b99`)

Tarball: `handwriting_e10_20260903_cf27b99.tar.gz` (~23 MB unpacked). Installed as `handwriting/`. Filenames match ocr-pipeline. The packs live in separate directories, so they do not collide with UniRec `encoder.onnx` / `decoder.onnx`.

Rust/WOQ scores: MW 63.47 / C23 50.87.

| File | Role |
|---|---|
| `encoder.onnx` | Stroke encoder (12-dim point features, KV memory) |
| `decoder_step.onnx` | Autoregressive decoder step (greedy, KV cache) |
| `vocab.json` | LaTeX vocab (258 tokens, JSON key order is the id) |

Strategy: weights-only quantization (MatMulConstBOnly). Input is online ink `(x, y, t)`, not a raster. Output is bare LaTeX.

## Runtime identity

Each ONNX file carries `localtex.*` metadata written after graph optimization
and quantization. At startup LocalTeX reads those string properties from disk
without opening an ORT session, then checks schema, component names, a common
pack ID, and agreement with the manifest. Settings shows **Checking…** until
that walk finishes, then **Verified**, **Unstamped**, or **Mismatch**. Sessions
still load on first use. The manifest and `SHA256SUMS` remain the source of
whole-pack integrity, including tokenizer and vocabulary files.
